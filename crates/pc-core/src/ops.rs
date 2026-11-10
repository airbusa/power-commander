//! Copy and move, one at a time, from a queue that can be reordered and cancelled.

use std::io::{self, Read, Write};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::command;
use crate::vfs::{Fs, Meta};

const BUFFER: usize = 1 << 20;
/// A file being written is named so until it is whole.
const PART: &str = ".pc-part";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
}

/// What happens to a file the target has already, as Total Commander offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overwrite {
    #[default]
    Ask,
    All,
    Skip,
    /// Only where the target is older.
    Older,
    /// The copy gets a free name: `a (2).txt`.
    Rename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Each source goes into the folder.
    Into(PathBuf),
    /// The one source gets this path.
    As(PathBuf),
}

impl Target {
    /// The copy dialog's text, relative to `base`: a folder for several
    /// sources, for one that exists, or for text ending in a slash; else
    /// the one source's new path.
    pub fn parse(text: &str, base: &Path, sources: usize, to: &dyn Fs) -> Self {
        let text = text.trim();
        let path = command::resolve(text, base);
        let slash = text.ends_with(['/', MAIN_SEPARATOR]);
        match sources > 1 || slash || to.metadata(&path).is_ok_and(|meta| meta.is_dir) {
            true => Target::Into(path),
            false => Target::As(path),
        }
    }
}

pub struct Operation {
    pub kind: Kind,
    pub from: Arc<dyn Fs>,
    pub sources: Vec<PathBuf>,
    pub to: Arc<dyn Fs>,
    pub target: Target,
    pub overwrite: Overwrite,
}

impl Operation {
    pub fn title(&self) -> String {
        let verb = match self.kind {
            Kind::Copy => "Copy",
            Kind::Move => "Move",
        };
        let what = match self.sources.as_slice() {
            [one] => format!("\"{}\"", name(one)),
            many => format!("{} items", many.len()),
        };
        let (Target::Into(target) | Target::As(target)) = &self.target;
        format!("{verb} {what} to {}", target.display())
    }

    fn same_fs(&self) -> bool {
        self.from.id() == self.to.id()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// Counting what to copy; the totals grow meanwhile.
    pub scanning: bool,
    pub files: u64,
    pub total_files: u64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub current: PathBuf,
    /// Files left out: by the overwrite choice or after an error.
    pub skipped: u64,
}

#[derive(Clone, Debug)]
pub enum Question {
    /// The target has `target` already.
    Exists {
        target: PathBuf,
        source: Meta,
        existing: Meta,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Overwrite,
    OverwriteAll,
    Skip,
    SkipAll,
    /// This one copy gets a free name.
    Rename,
    Retry,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Running,
    /// Finished; files skipped are counted in its progress.
    Done,
    Cancelled,
}

impl Status {
    pub fn finished(&self) -> bool {
        !matches!(self, Status::Waiting | Status::Running)
    }
}

/// A job as the queue manager shows it.
#[derive(Clone, Debug)]
pub struct JobView {
    pub id: u64,
    pub title: String,
    pub status: Status,
    pub progress: Progress,
    /// Waiting for the user's answer.
    pub question: Option<Question>,
}

struct Job {
    view: JobView,
    operation: Option<Operation>,
    answer: Option<Answer>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    jobs: Vec<Job>,
    next: u64,
    /// A worker thread runs.
    working: bool,
    /// Changes with anything the queue manager shows.
    version: u64,
}

/// One job runs at a time, in the queue's order.
#[derive(Clone, Default)]
pub struct Queue(Arc<(Mutex<State>, Condvar)>);

impl Queue {
    pub fn push(&self, operation: Operation) -> u64 {
        let mut state = self.lock();
        state.next += 1;
        let id = state.next;
        state.jobs.push(Job {
            view: JobView {
                id,
                title: operation.title(),
                status: Status::Waiting,
                progress: Progress::default(),
                question: None,
            },
            operation: Some(operation),
            answer: None,
            cancel: Arc::default(),
        });
        state.version += 1;
        if !state.working {
            state.working = true;
            let queue = self.clone();
            std::thread::spawn(move || queue.work());
        }
        id
    }

    pub fn version(&self) -> u64 {
        self.lock().version
    }

    pub fn jobs(&self) -> Vec<JobView> {
        self.lock()
            .jobs
            .iter()
            .map(|job| job.view.clone())
            .collect()
    }

    /// A waiting job is dropped; a running one stops at its next buffer,
    /// leaving no half-written file.
    pub fn cancel(&self, id: u64) {
        self.update(id, |job| match job.view.status {
            Status::Waiting => {
                job.view.status = Status::Cancelled;
                job.operation = None;
            }
            Status::Running => job.cancel.store(true, Ordering::Relaxed),
            _ => {}
        });
    }

    /// Moves a waiting job `by` places among the waiting ones.
    pub fn reorder(&self, id: u64, by: isize) {
        let mut state = self.lock();
        let waiting: Vec<usize> = (0..state.jobs.len())
            .filter(|&ix| state.jobs[ix].view.status == Status::Waiting)
            .collect();
        let Some(at) = waiting.iter().position(|&ix| state.jobs[ix].view.id == id) else {
            return;
        };
        let to = at.saturating_add_signed(by).min(waiting.len() - 1);
        if to != at {
            // After the removal, the slot of a later job is where it goes behind it.
            let job = state.jobs.remove(waiting[at]);
            state.jobs.insert(waiting[to], job);
            state.version += 1;
        }
    }

    pub fn answer(&self, id: u64, answer: Answer) {
        self.update(id, |job| {
            job.answer = Some(answer);
            job.view.question = None;
        });
    }

    /// At quit: cancels every job, and waits a little for the running one
    /// to remove its part file.
    pub fn stop_all(&self) {
        for job in self.jobs().iter().filter(|job| !job.status.finished()) {
            self.cancel(job.id);
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.lock().working && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn clear_finished(&self) {
        let mut state = self.lock();
        state.jobs.retain(|job| !job.view.status.finished());
        state.version += 1;
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.0.lock().unwrap()
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut Job)) {
        let mut state = self.lock();
        if let Some(job) = state.jobs.iter_mut().find(|job| job.view.id == id) {
            change(job);
        }
        state.version += 1;
        self.0.1.notify_all();
    }

    fn work(self) {
        loop {
            let (id, operation, cancel) = {
                let mut state = self.lock();
                let waiting = state
                    .jobs
                    .iter_mut()
                    .find(|job| job.view.status == Status::Waiting);
                let Some(job) = waiting else {
                    state.working = false;
                    return;
                };
                job.view.status = Status::Running;
                let operation = job
                    .operation
                    .take()
                    .expect("a waiting job has its operation");
                let found = (job.view.id, operation, job.cancel.clone());
                state.version += 1;
                found
            };
            let mut run = Run {
                queue: &self,
                id,
                cancel,
                overwrite: operation.overwrite,
                skip_errors: false,
                progress: Progress::default(),
                op: &operation,
            };
            let status = match run.all() {
                Ok(()) => Status::Done,
                Err(Cancelled) => Status::Cancelled,
            };
            self.update(id, |job| job.view.status = status);
        }
    }

    /// Waits for the user's answer; a cancel answers too.
    fn ask(&self, id: u64, question: Question) -> Answer {
        let mut state = self.lock();
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.view.id == id)
            .expect("running");
        job.view.question = Some(question);
        state.version += 1;
        loop {
            let job = state
                .jobs
                .iter_mut()
                .find(|job| job.view.id == id)
                .expect("running");
            if let Some(answer) = job.answer.take() {
                return answer;
            }
            if job.cancel.load(Ordering::Relaxed) {
                job.view.question = None;
                return Answer::Cancel;
            }
            state = self.0.1.wait(state).unwrap();
        }
    }
}

/// The user cancelled; errors are asked about file by file.
struct Cancelled;

/// One operation, running.
struct Run<'a> {
    queue: &'a Queue,
    id: u64,
    cancel: Arc<AtomicBool>,
    op: &'a Operation,
    /// Ask until the user picks one for all.
    overwrite: Overwrite,
    skip_errors: bool,
    progress: Progress,
}

impl Run<'_> {
    fn all(&mut self) -> Result<(), Cancelled> {
        let op = self.op;
        self.progress.scanning = true;
        let mut totals = Vec::new();
        for source in &op.sources {
            let before = (self.progress.total_files, self.progress.total_bytes);
            self.scan(source)?;
            let (files, bytes) = (self.progress.total_files, self.progress.total_bytes);
            totals.push((files - before.0, bytes - before.1));
        }
        self.progress.scanning = false;
        self.report();
        for (source, (files, bytes)) in op.sources.iter().zip(totals) {
            let target = match &op.target {
                Target::Into(dir) => dir.join(source.file_name().unwrap_or_default()),
                Target::As(path) => path.clone(),
            };
            // Within one file system a move is a rename, if nothing is in the way.
            let renamed = op.kind == Kind::Move
                && op.same_fs()
                && op.to.metadata(&target).is_err()
                && op.from.rename(source, &target).is_ok();
            if renamed {
                self.progress.files += files;
                self.progress.bytes += bytes;
                self.report();
            } else {
                self.item(source, &target)?;
            }
        }
        Ok(())
    }

    /// Counts the files and bytes under `path`; what cannot be read counts later, as it fails.
    fn scan(&mut self, path: &Path) -> Result<(), Cancelled> {
        self.check()?;
        let Ok(meta) = self.op.from.metadata(path) else {
            return Ok(());
        };
        if meta.is_dir && !meta.is_link {
            for name in self.op.from.list(path).unwrap_or_default() {
                self.scan(&path.join(name))?;
            }
        } else {
            self.progress.total_files += 1;
            self.progress.total_bytes += meta.size;
        }
        self.progress.current = path.to_owned();
        self.report();
        Ok(())
    }

    fn item(&mut self, source: &Path, target: &Path) -> Result<(), Cancelled> {
        self.check()?;
        let (from, to) = (self.op.from.clone(), self.op.to.clone());
        self.progress.current = source.to_owned();
        let Some(meta) = self.attempt(source, |_| from.metadata(source))? else {
            return Ok(());
        };
        if self.op.same_fs() && (target == source || meta.is_dir && target.starts_with(source)) {
            let into_itself =
                |_: &mut Self| Err::<(), _>(io::Error::other("cannot be copied into itself"));
            return self.attempt(source, into_itself).map(drop);
        }
        if meta.is_dir && !meta.is_link {
            return self.folder(source, target, &meta);
        }
        let Some(target) = self.resolve(target, &meta)? else {
            self.progress.skipped += 1;
            self.progress.files += 1;
            self.progress.bytes += meta.size;
            self.report();
            return Ok(());
        };
        // A rename replaces the target where the file system lets it.
        let renamed =
            self.op.kind == Kind::Move && self.op.same_fs() && from.rename(source, &target).is_ok();
        if renamed {
            self.progress.files += 1;
            self.progress.bytes += meta.size;
            self.report();
            return Ok(());
        }
        let copied = match meta.is_link && self.op.same_fs() {
            true => self.attempt(source, |_| {
                if to.metadata(&target).is_ok() {
                    to.remove(&target, false)?;
                }
                from.copy_link(source, &target)
            })?,
            false => self.attempt(source, |run| run.copy_file(source, &target, &meta))?,
        };
        self.progress.files += 1;
        if copied.is_none() {
            self.progress.skipped += 1;
        } else if self.op.kind == Kind::Move {
            self.attempt(source, |_| from.remove(source, false))?;
        }
        self.report();
        Ok(())
    }

    fn folder(&mut self, source: &Path, target: &Path, meta: &Meta) -> Result<(), Cancelled> {
        let (from, to) = (self.op.from.clone(), self.op.to.clone());
        let made = self.attempt(target, |_| match to.metadata(target) {
            Ok(existing) if existing.is_dir => Ok(()),
            Ok(_) => Err(io::Error::other("a file of this name is in the way")),
            Err(_) => to.mkdir(target),
        })?;
        if made.is_none() {
            return Ok(());
        }
        let names = self
            .attempt(source, |_| from.list(source))?
            .unwrap_or_default();
        for name in names {
            self.item(&source.join(&name), &target.join(&name))?;
        }
        let _ = to.set_meta(target, meta);
        if self.op.kind == Kind::Move {
            // Fails, and so stays, where something was skipped.
            let _ = from.remove(source, true);
        }
        Ok(())
    }

    /// Where the file goes, by the overwrite choice; none to skip it.
    fn resolve(&mut self, target: &Path, source: &Meta) -> Result<Option<PathBuf>, Cancelled> {
        let Ok(existing) = self.op.to.metadata(target) else {
            return Ok(Some(target.to_owned()));
        };
        let choice = match self.overwrite {
            Overwrite::Ask => {
                let question = Question::Exists {
                    target: target.to_owned(),
                    source: source.clone(),
                    existing: existing.clone(),
                };
                match self.queue.ask(self.id, question) {
                    Answer::Overwrite => Overwrite::All,
                    Answer::OverwriteAll => {
                        self.overwrite = Overwrite::All;
                        Overwrite::All
                    }
                    Answer::Skip => Overwrite::Skip,
                    Answer::SkipAll => {
                        self.overwrite = Overwrite::Skip;
                        Overwrite::Skip
                    }
                    Answer::Rename => Overwrite::Rename,
                    Answer::Retry | Answer::Cancel => return Err(Cancelled),
                }
            }
            choice => choice,
        };
        Ok(match choice {
            Overwrite::All => Some(target.to_owned()),
            Overwrite::Older => (existing.modified < source.modified).then(|| target.to_owned()),
            Overwrite::Rename => Some(self.free_name(target)),
            Overwrite::Skip | Overwrite::Ask => None,
        })
    }

    /// `a (2).txt`, `a (3).txt`, … the first the target has not.
    fn free_name(&self, target: &Path) -> PathBuf {
        let stem = target.file_stem().unwrap_or_default().to_string_lossy();
        let extension = target
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()));
        (2..)
            .map(|n| {
                target.with_file_name(format!(
                    "{stem} ({n}){}",
                    extension.as_deref().unwrap_or("")
                ))
            })
            .find(|path| self.op.to.metadata(path).is_err())
            .expect("some number is free")
    }

    /// Written under a part name and renamed when whole, so a failed or
    /// cancelled copy leaves the target as it was.
    fn copy_file(&mut self, source: &Path, target: &Path, meta: &Meta) -> io::Result<()> {
        let (from, to) = (self.op.from.clone(), self.op.to.clone());
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let part = target.with_file_name(format!("{name}{PART}"));
        let before = self.progress.bytes;
        let mut write = || -> io::Result<()> {
            let mut reader = from.open(source)?;
            let mut writer = to.create(&part)?;
            let mut buffer = vec![0; BUFFER];
            loop {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let read = match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err),
                };
                writer.write_all(&buffer[..read])?;
                self.progress.bytes += read as u64;
                self.report();
            }
            writer.flush()?;
            drop(writer);
            if to.metadata(target).is_ok() {
                to.remove(target, false)?;
            }
            to.rename(&part, target)?;
            let _ = to.set_meta(target, meta);
            Ok(())
        };
        let written = write();
        if written.is_err() {
            let _ = to.remove(&part, false);
            self.progress.bytes = before;
        }
        written
    }

    /// Runs `step`; on an error asks Retry, Skip, Skip all or Cancel.
    fn attempt<T>(
        &mut self,
        path: &Path,
        mut step: impl FnMut(&mut Self) -> io::Result<T>,
    ) -> Result<Option<T>, Cancelled> {
        loop {
            let error = match step(self) {
                Ok(value) => return Ok(Some(value)),
                Err(error) => error,
            };
            self.check()?;
            if self.skip_errors {
                return Ok(None);
            }
            let question = Question::Failed {
                path: path.to_owned(),
                error: error.to_string(),
            };
            match self.queue.ask(self.id, question) {
                Answer::Retry => continue,
                Answer::Skip => return Ok(None),
                Answer::SkipAll => {
                    self.skip_errors = true;
                    return Ok(None);
                }
                _ => return Err(Cancelled),
            }
        }
    }

    fn check(&self) -> Result<(), Cancelled> {
        match self.cancel.load(Ordering::Relaxed) {
            true => Err(Cancelled),
            false => Ok(()),
        }
    }

    fn report(&self) {
        let progress = self.progress.clone();
        self.queue
            .update(self.id, |job| job.view.progress = progress);
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::vfs::Local;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pc-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn operation(
        kind: Kind,
        sources: &[PathBuf],
        target: Target,
        overwrite: Overwrite,
    ) -> Operation {
        Operation {
            kind,
            from: Arc::new(Local),
            sources: sources.to_vec(),
            to: Arc::new(Local),
            target,
            overwrite,
        }
    }

    /// Runs `op`, answering its questions with `answer`, until it finishes.
    fn run(op: Operation, answer: impl Fn(&Question) -> Answer) -> JobView {
        let queue = Queue::default();
        let id = queue.push(op);
        loop {
            let job = queue.jobs().into_iter().find(|job| job.id == id).unwrap();
            if job.status.finished() {
                return job;
            }
            if let Some(question) = &job.question {
                queue.answer(id, answer(question));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn copies_and_asks_what_to_overwrite() {
        let dir = temp("copy");
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("sub/b.txt"), "bb").unwrap();
        fs::write(dst.join("a.txt"), "old").unwrap();
        let sources = [src.join("a.txt"), src.join("sub")];
        let into = Target::Into(dst.clone());

        let job = run(
            operation(Kind::Copy, &sources, into.clone(), Overwrite::Ask),
            |question| {
                assert!(matches!(question, Question::Exists { .. }));
                Answer::Skip
            },
        );
        assert_eq!(job.status, Status::Done);
        assert_eq!(
            (read(&dst.join("a.txt")), read(&dst.join("sub/b.txt"))),
            ("old".into(), "bb".into())
        );
        let p = &job.progress;
        assert_eq!(
            (p.files, p.total_files, p.skipped, p.total_bytes),
            (2, 2, 1, 3)
        );

        run(
            operation(Kind::Copy, &sources, into.clone(), Overwrite::Ask),
            |_| Answer::OverwriteAll,
        );
        assert_eq!(read(&dst.join("a.txt")), "a");
        run(
            operation(Kind::Copy, &sources[..1], into.clone(), Overwrite::Rename),
            |_| unreachable!(),
        );
        assert_eq!(read(&dst.join("a (2).txt")), "a");

        // Older: only a target older than the source is replaced.
        fs::write(dst.join("a.txt"), "newer").unwrap();
        let later = SystemTime::now() + Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(dst.join("a.txt"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        run(
            operation(Kind::Copy, &sources[..1], into, Overwrite::Older),
            |_| unreachable!(),
        );
        assert_eq!(read(&dst.join("a.txt")), "newer");

        let parts = fs::read_dir(&dst).unwrap().filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(PART)
        });
        assert_eq!(parts.count(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moves_by_renaming_or_copying() {
        let dir = temp("move");
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        fs::create_dir_all(src.join("tree/sub")).unwrap();
        fs::create_dir_all(dst.join("tree")).unwrap();
        fs::write(src.join("m.txt"), "m").unwrap();
        fs::write(src.join("tree/sub/c.txt"), "c").unwrap();
        fs::write(dst.join("tree/kept.txt"), "k").unwrap();

        let renamed = dir.join("renamed.txt");
        run(
            operation(
                Kind::Move,
                &[src.join("m.txt")],
                Target::As(renamed.clone()),
                Overwrite::Ask,
            ),
            |_| unreachable!(),
        );
        assert!(!src.join("m.txt").exists());
        assert_eq!(read(&renamed), "m");

        // The target has the folder: its contents merge, the source goes.
        let job = run(
            operation(
                Kind::Move,
                &[src.join("tree")],
                Target::Into(dst.clone()),
                Overwrite::Ask,
            ),
            |_| unreachable!(),
        );
        assert_eq!(job.progress.files, 1);
        assert_eq!(read(&dst.join("tree/sub/c.txt")), "c");
        assert!(dst.join("tree/kept.txt").exists());
        assert!(!src.join("tree").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_to_copy_a_folder_into_itself() {
        let dir = temp("itself");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        let op = operation(
            Kind::Copy,
            &[dir.join("a")],
            Target::Into(dir.join("a/b")),
            Overwrite::Ask,
        );
        let job = run(op, |question| match question {
            Question::Failed { error, .. } if error.contains("into itself") => Answer::Skip,
            other => panic!("{other:?}"),
        });
        assert_eq!(job.status, Status::Done);
        assert!(!dir.join("a/b/a").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn runs_jobs_one_at_a_time_in_the_users_order() {
        let dir = temp("queue");
        fs::write(dir.join("x"), "new").unwrap();
        fs::create_dir(dir.join("to")).unwrap();
        fs::write(dir.join("to/x"), "old").unwrap();
        let queue = Queue::default();
        let copy = |to: &str| {
            operation(
                Kind::Copy,
                &[dir.join("x")],
                Target::Into(dir.join(to)),
                Overwrite::Ask,
            )
        };
        let first = queue.push(copy("to"));
        // The first waits for an answer meanwhile.
        while queue.jobs()[0].question.is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        fs::create_dir(dir.join("second")).unwrap();
        fs::create_dir(dir.join("third")).unwrap();
        let second = queue.push(copy("second"));
        let third = queue.push(copy("third"));
        queue.reorder(third, -1);
        let order: Vec<u64> = queue.jobs().iter().map(|job| job.id).collect();
        assert_eq!(order, [first, third, second]);
        queue.cancel(second);
        queue.answer(first, Answer::Overwrite);
        while !queue.jobs().iter().all(|job| job.status.finished()) {
            std::thread::sleep(Duration::from_millis(2));
        }
        let status: Vec<Status> = queue.jobs().iter().map(|job| job.status.clone()).collect();
        assert_eq!(status, [Status::Done, Status::Done, Status::Cancelled]);
        assert_eq!(read(&dir.join("to/x")), "new");
        assert!(dir.join("third/x").exists() && !dir.join("second/x").exists());

        // A running job stops, even while it asks.
        let running = queue.push(copy("to"));
        while queue.jobs().last().unwrap().question.is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        queue.cancel(running);
        while !queue.jobs().last().unwrap().status.finished() {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(queue.jobs().last().unwrap().status, Status::Cancelled);
        queue.clear_finished();
        assert!(queue.jobs().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_target_as_total_commander() {
        let dir = temp("target");
        fs::create_dir(dir.join("folder")).unwrap();
        let parse = |text: &str, sources| Target::parse(text, &dir, sources, &Local);
        assert_eq!(parse("folder", 1), Target::Into(dir.join("folder")));
        assert_eq!(parse("new/", 1), Target::Into(dir.join("new")));
        assert_eq!(parse("new.txt", 1), Target::As(dir.join("new.txt")));
        assert_eq!(parse("new", 2), Target::Into(dir.join("new")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
