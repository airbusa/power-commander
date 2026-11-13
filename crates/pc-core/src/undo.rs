//! Files > Undo (Ctrl+Z): the last copies, moves, renames and moves to the
//! trash, undone newest first. What overwrote or joined something there
//! already is not kept: what was there is gone.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::ops::{Kind, Operation, Overwrite, Queue, Target};
use crate::vfs::{self, Fs};

pub enum Step {
    /// New copies: removed again, to the trash on this computer.
    Copied { fs: Arc<dyn Fs>, made: Vec<PathBuf> },
    /// Moved or renamed, `(was, now)`: back, from `to`'s file system to `from`'s.
    Moved { from: Arc<dyn Fs>, to: Arc<dyn Fs>, moves: Vec<(PathBuf, PathBuf)> },
    /// Moved to the trash at `at`: restored.
    Trashed { paths: Vec<PathBuf>, at: SystemTime },
}

pub struct Undo {
    /// What was done: "Copy 3 items to /home/me".
    pub title: String,
    pub step: Step,
}

/// How many are kept.
const KEPT: usize = 20;

static DONE: Mutex<Vec<Undo>> = Mutex::new(Vec::new());

pub fn remember(title: String, step: Step) {
    let mut done = DONE.lock().unwrap();
    done.push(Undo { title, step });
    if done.len() > KEPT {
        done.remove(0);
    }
}

/// The newest, to undo now.
pub fn take() -> Option<Undo> {
    DONE.lock().unwrap().pop()
}

/// Undoes `undo`: moves go back through `queue`, as a copy goes, unrecorded;
/// copies and the trash here, which blocks.
pub fn run(undo: Undo, queue: &Queue) -> io::Result<()> {
    match undo.step {
        Step::Copied { fs, made } => made.iter().try_for_each(|path| match fs.id() == "local" {
            true => trash::delete(path).map_err(io::Error::other),
            false => vfs::remove_all(&*fs, path),
        }),
        Step::Moved { from, to, moves } => {
            for (was, now) in moves {
                // The same name goes into its folder; a renamed one gets its name back.
                let target = match (was.file_name() == now.file_name(), was.parent()) {
                    (true, Some(parent)) => Target::Into(parent.to_owned()),
                    _ => Target::As(was),
                };
                let back = Operation { kind: Kind::Move, from: to.clone(), sources: vec![now], to: from.clone(), target, overwrite: Overwrite::Ask };
                queue.push_reversal(back);
            }
            Ok(())
        }
        Step::Trashed { paths, at } => restore(&paths, at),
    }
}

/// Each of `paths` back from the trash: the newest of its name, put there since `at`.
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn restore(paths: &[PathBuf], at: SystemTime) -> io::Result<()> {
    let since = at.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64) - 2;
    let mut items: Vec<trash::TrashItem> = trash::os_limited::list().map_err(io::Error::other)?;
    items.retain(|item| item.time_deleted >= since && paths.contains(&item.original_path()));
    items.sort_by_key(|item| std::cmp::Reverse(item.time_deleted));
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.original_path()));
    if items.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "they are no longer in the trash"));
    }
    trash::os_limited::restore_all(items).map_err(io::Error::other)
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn restore(_: &[PathBuf], _: SystemTime) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the trash here cannot be undone: Finder's Put Back can"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Local;
    use std::time::Duration;

    /// The undo list is the whole process's, other tests' too: this one's by its title.
    fn taken(title: &str) -> Option<Undo> {
        let mut done = DONE.lock().unwrap();
        let at = done.iter().position(|undo| undo.title == title)?;
        Some(done.remove(at))
    }

    fn finish(queue: &Queue) {
        while !queue.jobs().iter().all(|job| job.status.finished()) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn moves_back_what_was_moved_and_renamed() {
        let dir = std::env::temp_dir().join(format!("pc-undo-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("into")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let queue = Queue::default();
        let local = || -> Arc<dyn Fs> { Arc::new(Local) };
        let moved = Operation { kind: Kind::Move, from: local(), sources: vec![dir.join("a.txt")], to: local(), target: Target::Into(dir.join("into")), overwrite: Overwrite::Ask };
        let title = moved.title();
        queue.push(moved);
        finish(&queue);
        assert!(dir.join("into/a.txt").exists());
        let renamed = Operation { kind: Kind::Move, from: local(), sources: vec![dir.join("b.txt")], to: local(), target: Target::As(dir.join("c.txt")), overwrite: Overwrite::Ask };
        let renamed_title = renamed.title();
        queue.push(renamed);
        finish(&queue);
        for title in [title, renamed_title] {
            run(taken(&title).expect("remembered"), &queue).unwrap();
        }
        finish(&queue);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "a", "back in its folder");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "b", "its name back");
        assert!(!dir.join("into/a.txt").exists() && !dir.join("c.txt").exists());
        // The move back is not remembered in its turn.
        assert!(DONE.lock().unwrap().iter().all(|undo| !undo.title.contains(&*dir.to_string_lossy())));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
