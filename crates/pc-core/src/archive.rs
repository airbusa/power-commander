//! Archives through the bundled 7-Zip.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime};

use chrono::{Local, NaiveDateTime, TimeZone};

use crate::Entry;

const ARCHIVES: &[&str] = &[
    "7z", "zip", "rar", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst",
    "lzma", "tlz", "z", "taz", "iso", "cab", "wim", "arj", "lzh", "cpio", "rpm", "deb", "jar",
    "war", "apk", "xpi", "msi", "chm", "vhd", "vhdx", "dmg",
];
const TARS: &[&str] = &["tgz", "tbz", "tbz2", "txz", "tzst", "tlz", "taz"];

#[derive(Clone)]
pub(crate) struct Item {
    pub path: PathBuf,
    /// As 7-Zip lists it, for taking it out: `./a.txt` where `path` is `a.txt`.
    pub listed: String,
    pub entry: Entry,
}

/// A file inside an archive, to take out for F3 and F4.
#[derive(Clone, Debug)]
pub struct Member {
    pub archive: PathBuf,
    pub listed: String,
    pub name: OsString,
    /// Its path inside the archive.
    pub inside: PathBuf,
}

impl Member {
    /// Extracts the file into a folder of its own under `scratch()`.
    pub fn extract(&self) -> io::Result<PathBuf> {
        let dir = fresh_scratch()?;
        extract(&self.archive, &[&self.listed], &dir, false)?;
        self.found(dir.join(&self.name))
    }

    /// Extracts it with its folders, as `update` takes it back: the
    /// folder it went to, and the file.
    pub fn extract_with_folders(&self) -> io::Result<(PathBuf, PathBuf)> {
        let dir = fresh_scratch()?;
        extract(&self.archive, &[&self.listed], &dir, true)?;
        let file = self.found(dir.join(&self.inside))?;
        Ok((dir, file))
    }

    fn found(&self, path: PathBuf) -> io::Result<PathBuf> {
        match path.exists() {
            true => Ok(path),
            false => Err(io::Error::other(format!(
                "{} is not in the archive",
                self.listed
            ))),
        }
    }

    /// Whether 7-Zip can write the archive back.
    pub fn updatable(&self) -> bool {
        updatable(&self.archive)
    }

    /// Puts the file back from `dir`, where `extract_with_folders` took it.
    // shortcut: a tar listing `./a.txt` gets `a.txt` beside it; strip such prefixes if it matters.
    pub fn update(&self, dir: &Path) -> io::Result<()> {
        let name = self
            .archive
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        // Files named are always taken; all else stays as it is.
        let output = seven_zip(
            &["u", "-spd", "-bso0", "-bsp0", "-up1q1r2x2y2z2w2"],
            Some(&self.archive),
        )
        .arg(&self.inside)
        .current_dir(dir)
        .output()
        .map_err(missing)?;
        match output.status.success() {
            true => Ok(()),
            false => Err(io::Error::other(format!(
                "{name}: {}",
                failure(output.status, &output.stderr)
            ))),
        }
    }
}

/// What 7-Zip can update in place; compressed tars it cannot.
const UPDATABLE: &[&str] = &["7z", "zip", "tar", "jar"];

/// Whether 7-Zip can change the archive in place.
pub(crate) fn updatable(archive: &Path) -> bool {
    extension(archive.file_name().unwrap_or_default()).is_some_and(|ext| UPDATABLE.contains(&ext.as_str()))
}

/// Runs 7-Zip's `args` on `archive` in `dir`, then `names`, which are no wildcards.
fn change(args: &[&str], archive: &Path, dir: Option<&Path>, names: &[&OsStr]) -> io::Result<()> {
    let mut command = seven_zip(&[args, &["-y", "-spd", "-bso0", "-bsp0"]].concat(), Some(archive));
    command.args(names);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let done = command.output().map_err(missing)?;
    match done.status.success() {
        true => Ok(()),
        false => {
            let name = archive.file_name().unwrap_or_default().to_string_lossy();
            Err(io::Error::other(format!("{name}: {}", failure(done.status, &done.stderr))))
        }
    }
}

/// `sources`, this computer's, into `archive`'s folder `dir` (empty: its top),
/// replacing what has their names there. 7-Zip names them by where they
/// are, so they are linked (Windows: copied) into that folder's path first.
pub fn add(archive: &Path, sources: &[PathBuf], dir: &Path) -> io::Result<()> {
    if dir.as_os_str().is_empty() {
        let names: Vec<&OsStr> = sources.iter().map(|source| source.as_os_str()).collect();
        return change(&["a"], archive, None, &names);
    }
    let stage = fresh_scratch()?;
    let inside = stage.join(dir);
    fs::create_dir_all(&inside)?;
    let staged = sources.iter().try_for_each(|source| stage_one(source, &inside.join(source.file_name().unwrap_or_default())));
    let top = dir.components().next().map(|top| top.as_os_str().to_owned()).unwrap_or_default();
    let added = staged.and_then(|()| change(&["a"], &archive.canonicalize()?, Some(&stage), &[&top]));
    // Links go, never what they point to.
    let _ = fs::remove_dir_all(&stage);
    added
}

#[cfg(unix)]
fn stage_one(source: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source.canonicalize()?, at)
}

/// Windows lets only some make links: copied.
#[cfg(not(unix))]
fn stage_one(source: &Path, at: &Path) -> io::Result<()> {
    if !fs::metadata(source)?.is_dir() {
        return fs::copy(source, at).map(drop);
    }
    fs::create_dir_all(at)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        stage_one(&entry.path(), &at.join(entry.file_name()))?;
    }
    Ok(())
}

/// The items at `paths` inside `archive` out of it, folders with all in them.
pub fn delete(archive: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let names: Vec<&OsStr> = paths.iter().map(|path| path.as_os_str()).collect();
    change(&["d"], archive, None, &names)
}

/// The item at `from` inside `archive` (a folder with all in it) at `to`.
pub(crate) fn rename(archive: &Path, from: &Path, to: &Path) -> io::Result<()> {
    change(&["rn"], archive, None, &[from.as_os_str(), to.as_os_str()])
}

/// An empty folder `dir`/`name` inside `archive`.
pub(crate) fn new_folder(archive: &Path, dir: &Path, name: &str) -> io::Result<()> {
    let stage = fresh_scratch()?;
    fs::create_dir_all(stage.join(dir).join(name))?;
    let top = dir.join(name).components().next().map(|top| top.as_os_str().to_owned()).unwrap_or_default();
    let made = change(&["a"], &archive.canonicalize()?, Some(&stage), &[&top]);
    let _ = fs::remove_dir_all(&stage);
    made
}

/// Where files taken out of archives go while the app runs.
pub fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("{SCRATCH}{}", std::process::id()))
}

const SCRATCH: &str = "powercommander-";
/// Locked while its run lasts; the system unlocks it when the run ends.
const LOCK: &str = ".lock";

/// This run's locks, once `scratch()` and `tars()` exist.
static HELD: Mutex<Option<File>> = Mutex::new(None);
static HELD_TARS: Mutex<Option<File>> = Mutex::new(None);

/// Compressed tars' tars, unpacked once a run: on disk, as a big one would
/// fill a /tmp that is memory (Fedora's).
fn tars() -> PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    cache.join(format!("{SCRATCH}{}", std::process::id()))
}

/// A new folder under `scratch()`, so files of one name never meet.
pub fn fresh_scratch() -> io::Result<PathBuf> {
    static TAKEN: AtomicUsize = AtomicUsize::new(0);
    let dir = claim()?.join(TAKEN.fetch_add(1, Ordering::Relaxed).to_string());
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Creates `scratch()` and locks it, at the first use.
fn claim() -> io::Result<PathBuf> {
    claim_in(scratch(), &HELD)
}

fn claim_in(dir: PathBuf, held: &Mutex<Option<File>>) -> io::Result<PathBuf> {
    let mut held = held.lock().unwrap();
    if held.is_none() {
        fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.lock()?;
        *held = Some(lock);
    }
    Ok(dir)
}

/// Removes `scratch()` and `tars()` as the app quits; Windows deletes no open file, so unlocked first.
pub fn remove_scratch() {
    HELD.lock().unwrap().take();
    let _ = fs::remove_dir_all(scratch());
    HELD_TARS.lock().unwrap().take();
    let _ = fs::remove_dir_all(tars());
}

/// Removes the folders runs left behind when killed or crashed.
pub fn remove_stale_scratch() {
    remove_stale(&std::env::temp_dir(), &scratch());
    if let Some(cache) = tars().parent() {
        remove_stale(cache, &tars());
    }
}

fn remove_stale(temp: &Path, ours: &Path) {
    let Ok(dirs) = fs::read_dir(temp) else {
        return;
    };
    for dir in dirs.flatten() {
        let path = dir.path();
        let name = dir.file_name();
        if path == ours || !name.to_string_lossy().starts_with(SCRATCH) {
            continue;
        }
        let stale = match File::open(path.join(LOCK)) {
            Ok(lock) => lock.try_lock().is_ok(),
            // Unlocked yet: a run between creating it and locking it, unless old.
            Err(_) => dir
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|time| time.elapsed().unwrap_or_default() > Duration::from_secs(3600)),
        };
        if stale {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// `listed` out of `archive` into `dir`, with their folders or without.
/// A folder named comes with all in it.
pub(crate) fn extract(
    archive: &Path,
    listed: &[&str],
    dir: &Path,
    folders: bool,
) -> io::Result<()> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let fail = |err: String| io::Error::other(format!("{name}: {err}"));
    let out = dir
        .to_str()
        .ok_or_else(|| fail("the temporary folder's name is not UTF-8".into()))?;
    let out = format!("-o{out}");
    // -spd: a name with `*` or `[` is no wildcard.
    let command = if folders { "x" } else { "e" };
    let take = [command, "-y", "-spd", "-bso0", "-bsp0", out.as_str()];
    let unpacked = unpacked_tar(archive).filter(|tar| tar.is_file());
    let output = if let Some(tar) = unpacked {
        seven_zip(&take, Some(&tar)).args(listed).output().map_err(missing)?
    } else if is_compressed_tar(archive) {
        let mut unpack = seven_zip(&["x", "-so"], Some(archive))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(missing)?;
        let stdout = unpack.stdout.take().expect("piped");
        let extracted = seven_zip(&[&take[..], &["-si", "-ttar"]].concat(), None)
            .args(listed)
            .stdin(stdout)
            .output()
            .map_err(missing)?;
        let unpacked = unpack.wait()?;
        if extracted.status.success() && !unpacked.success() {
            return Err(fail(failure(unpacked, b"")));
        }
        extracted
    } else {
        seven_zip(&take, Some(archive))
            .args(listed)
            .output()
            .map_err(missing)?
    };
    match output.status.success() {
        true => Ok(()),
        false => Err(fail(failure(output.status, &output.stderr))),
    }
}

pub fn is_archive(name: &OsStr) -> bool {
    extension(name).is_some_and(|ext| ARCHIVES.contains(&ext.as_str()))
}

fn extension(name: &OsStr) -> Option<String> {
    Some(Path::new(name).extension()?.to_str()?.to_ascii_lowercase())
}

// 7-Zip sees only the tar inside a compressed tar.
pub(crate) fn is_compressed_tar(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default();
    let stem = Path::new(Path::new(name).file_stem().unwrap_or_default());
    extension(name).is_some_and(|ext| TARS.contains(&ext.as_str()))
        || extension(stem.as_os_str()).as_deref() == Some("tar")
}

pub(crate) fn list(archive: &Path) -> io::Result<Vec<Item>> {
    list_watched(archive, &Watch::default())
}

/// A listing that can take a while (a compressed tar is unpacked whole):
/// how far it is, and a request to stop it, shared with the front end.
#[derive(Clone, Default)]
pub struct Watch {
    /// 0 to 100; 0 while 7-Zip tells none.
    pub percent: Arc<AtomicU64>,
    pub stop: Arc<AtomicBool>,
}

/// Stopped, it fails as `Interrupted`.
pub(crate) fn list_watched(archive: &Path, watch: &Watch) -> io::Result<Vec<Item>> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let fail = |err: String| io::Error::other(format!("{name}: {err}"));
    // A compressed tar is unpacked whole, once: its tar is listed, and files are taken from it at once.
    let listed = match is_compressed_tar(archive) {
        true => unpack_tar(archive, watch)?,
        false => archive.to_owned(),
    };
    let mut listing = seven_zip(&["l", "-slt", "-ba"], Some(&listed))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
    let (mut stdout, mut stderr) = (listing.stdout.take().expect("piped"), listing.stderr.take().expect("piped"));
    let (done, read) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let _ = stdout.read_to_end(&mut out);
        let _ = stderr.read_to_end(&mut err);
        let _ = done.send((out, err));
    });
    let (stdout, stderr) = loop {
        match read.recv_timeout(Duration::from_millis(50)) {
            Ok(output) => break output,
            Err(mpsc::RecvTimeoutError::Timeout) if !watch.stop.load(Ordering::Relaxed) => {}
            Err(_) => {
                let _ = listing.kill();
                let _ = listing.wait();
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
        }
    };
    let status = listing.wait()?;
    if !status.success() {
        return Err(fail(failure(status, &stderr)));
    }
    Ok(parse(&String::from_utf8_lossy(&stdout)))
}

/// Where a compressed tar's tar is unpacked this run: by its path, size and time.
fn unpacked_tar(archive: &Path) -> Option<PathBuf> {
    let key = crate::convert::key(archive).ok()?;
    is_compressed_tar(archive).then(|| tars().join(format!("{key}.tar")))
}

/// The tar inside `archive`, unpacked into `tars()` unless it is already,
/// 7-Zip's percentage in `watch`; stopped, or failing, it leaves none.
fn unpack_tar(archive: &Path, watch: &Watch) -> io::Result<PathBuf> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let tar = unpacked_tar(archive).ok_or_else(|| io::Error::other(format!("{name}: cannot be read")))?;
    if tar.is_file() {
        return Ok(tar);
    }
    claim_in(tars(), &HELD_TARS)?;
    let part = tar.with_extension("part");
    let mut file = File::create(&part)?;
    let mut unpack = seven_zip(&["x", "-so", "-bsp2", "-bso0"], Some(archive))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
    // Its percentages, as they come.
    let (mut progress, percent) = (unpack.stderr.take().expect("piped"), watch.percent.clone());
    let reading = std::thread::spawn(move || {
        let mut chunk = [0; 4096];
        while let Ok(read @ 1..) = progress.read(&mut chunk) {
            if let Some(at) = crate::packing::last_percent(&chunk[..read]) {
                percent.store(at, Ordering::Relaxed);
            }
        }
    });
    let mut stdout = unpack.stdout.take().expect("piped");
    let (done, copied) = mpsc::channel();
    std::thread::spawn(move || drop(done.send(io::copy(&mut stdout, &mut file))));
    let copied = loop {
        match copied.recv_timeout(Duration::from_millis(50)) {
            Ok(copied) => break copied,
            Err(mpsc::RecvTimeoutError::Timeout) if !watch.stop.load(Ordering::Relaxed) => {}
            Err(_) => {
                let _ = unpack.kill();
                let _ = unpack.wait();
                let _ = fs::remove_file(&part);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
        }
    };
    let unpacked = unpack.wait()?;
    let _ = reading.join();
    let failed = match (copied, unpacked.success()) {
        (Ok(_), true) => return fs::rename(&part, &tar).map(|()| tar),
        // As a full disk.
        (Err(err), _) => io::Error::new(err.kind(), format!("{name}: {err}")),
        (Ok(_), false) => io::Error::other(format!("{name}: {}", failure(unpacked, b""))),
    };
    let _ = fs::remove_file(&part);
    Err(failed)
}

/// The entries of `dir` inside the archive, `..` first.
pub(crate) fn entries(items: &[Item], dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = items
        .iter()
        .filter(|item| item.path.parent() == Some(dir))
        .map(|item| item.entry.clone())
        .collect();
    entries.insert(0, crate::parent_entry(None));
    entries
}

pub(crate) fn seven_zip(args: &[&str], archive: Option<&Path>) -> Command {
    let mut command = Command::new(executable());
    // `--`: an archive named `-x.zip` is no switch.
    command
        .args(args)
        .args(["-sccUTF-8", "--"])
        .args(archive)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

// Releases ship it; source builds use `vendor/7zip`.
fn executable() -> PathBuf {
    let name = if cfg!(windows) { "7z.exe" } else { "7zz" };
    crate::bundled::path("7zip", name)
}

pub(crate) fn missing(err: io::Error) -> io::Error {
    match err.kind() {
        io::ErrorKind::NotFound => io::Error::new(
            err.kind(),
            format!(
                "7-Zip is missing: {} (run vendor/7zip/fetch.sh or fetch.bat)",
                executable().display()
            ),
        ),
        _ => err,
    }
}

fn failure(status: ExitStatus, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    match stderr.lines().map(str::trim).rfind(|line| !line.is_empty()) {
        Some(line) => line.to_owned(),
        None => format!("7-Zip failed ({status})"),
    }
}

/// `l -slt` output; folders only implied by their contents are added.
fn parse(listing: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut dirs = HashSet::new();
    for block in listing.replace("\r\n", "\n").split("\n\n") {
        let field = |key: &str| {
            block
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(" = "))
        };
        let Some(listed) = field("Path") else {
            continue;
        };
        let Some(path) = clean(listed) else {
            continue;
        };
        let is_dir = field("Folder") == Some("+");
        if is_dir && !dirs.insert(path.clone()) {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_owned();
        let entry = Entry {
            hidden: crate::dot_file(&name),
            system: false,
            attributes: attributes(field("Mode").or(field("Attributes")).unwrap_or("")),
            tags: Vec::new(),
            online: false,
            name,
            is_dir,
            size: field("Size").and_then(|s| s.parse().ok()).unwrap_or(0),
            modified: field("Modified").and_then(parse_time),
        };
        let listed = listed.to_owned();
        items.push(Item {
            path,
            listed,
            entry,
        });
    }
    let implied: Vec<PathBuf> = items
        .iter()
        .flat_map(|item| item.path.ancestors().skip(1))
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect();
    for dir in implied {
        if dirs.insert(dir.clone()) {
            let name = dir.file_name().unwrap_or_default().to_owned();
            let entry = Entry {
                hidden: crate::dot_file(&name),
                system: false,
                attributes: String::new(),
                tags: Vec::new(),
                online: false,
                name,
                is_dir: true,
                size: 0,
                modified: None,
            };
            let listed = dir.to_string_lossy().into_owned();
            items.push(Item {
                path: dir,
                listed,
                entry,
            });
        }
    }
    items
}

/// 7-Zip's `-rw-r--r--` (tar, zip from Unix) or `RAHS` letters (zip from Windows).
fn attributes(text: &str) -> String {
    let last = text.split_whitespace().last().unwrap_or_default();
    if last.len() >= 9 && last.is_ascii() {
        return last[last.len() - 9..].to_owned();
    }
    if last.is_empty() {
        return String::new();
    }
    let flags = [('R', 'r'), ('A', 'a'), ('H', 'h'), ('S', 's')];
    flags
        .iter()
        .map(|&(letter, shown)| if last.contains(letter) { shown } else { '-' })
        .collect()
}

/// Relative and without `.`; none for paths that climb out with `..`.
fn clean(path: &str) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(part) => clean.push(part),
            Component::ParentDir => return None,
            _ => {}
        }
    }
    (!clean.as_os_str().is_empty()).then_some(clean)
}

fn parse_time(text: &str) -> Option<SystemTime> {
    let time = NaiveDateTime::parse_from_str(text.get(..19)?, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(Local.from_local_datetime(&time).earliest()?.into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A listing waiting for its archive (a pipe nobody writes) stops when asked.
    #[cfg(unix)]
    /// The run's folders (`scratch()`, `tars()`) are the whole test process's,
    /// and one test removes them: those using them take turns.
    pub(crate) fn shared() -> std::sync::MutexGuard<'static, ()> {
        static TURNS: Mutex<()> = Mutex::new(());
        TURNS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn stops_a_listing() {
        let _alone = shared();
        if !executable().exists() {
            return eprintln!("no 7-Zip here: vendor/7zip/fetch.sh");
        }
        let dir = std::env::temp_dir().join(format!("pc-archive-stop-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let never = dir.join("never.tar.gz");
        assert!(Command::new("mkfifo").arg(&never).status().unwrap().success());
        let watch = Watch::default();
        let stop = watch.stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            stop.store(true, Ordering::Relaxed);
        });
        let err = list_watched(&never, &watch).err().expect("stopped");
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        fs::remove_dir_all(&dir).unwrap();
    }

    pub(crate) fn attributes_of(text: &str) -> String {
        attributes(text)
    }
    use crate::Panel;
    use std::fs;

    fn names(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(Entry::display_name).collect()
    }

    #[test]
    fn parses_listings() {
        let listing = "Path = ./docs/guide/a.txt\r\nFolder = -\r\nSize = 12\r\nModified = 2026-11-10 09:50:50.8421726\r\n\r\n\
                       Path = docs\nFolder = +\n\nPath = ../evil\nFolder = -\n\nPath = b.txt\nSize = 3\n\n\
                       Path = docs\nFolder = +\n";
        let items = parse(listing);
        assert_eq!(
            items[0].listed, "./docs/guide/a.txt",
            "as 7-Zip takes it out"
        );
        assert_eq!(
            names(&entries(&items, Path::new(""))),
            ["..", "docs", "b.txt"]
        );
        assert_eq!(names(&entries(&items, Path::new("docs"))), ["..", "guide"]);
        let guide = entries(&items, Path::new("docs/guide"));
        assert_eq!(names(&guide), ["..", "a.txt"]);
        assert_eq!(guide[1].size, 12);
        assert!(guide[1].modified.is_some());
    }

    /// Into a folder inside a zip: files and folders added, a folder made,
    /// one renamed, two deleted; a compressed tar says it cannot be changed.
    #[test]
    fn edits_archives() {
        let _alone = shared();
        if !executable().exists() {
            return eprintln!("no 7-Zip here: vendor/7zip/fetch.sh");
        }
        let root = std::env::temp_dir().join(format!("pc-archive-edit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/sub")).unwrap();
        fs::create_dir_all(root.join("newdir")).unwrap();
        for (file, text) in [("src/a.txt", "a"), ("src/sub/b.txt", "b"), ("new.txt", "new"), ("newdir/inner.txt", "inner")] {
            fs::write(root.join(file), text).unwrap();
        }
        let packed = seven_zip(&["a", "-bso0", "-tzip"], Some(&root.join("t.zip"))).arg("src").current_dir(&root).status().unwrap();
        assert!(packed.success());
        for (kind, archive, what) in [("-ttar", "t.tar", "new.txt"), ("-tgzip", "t.tar.gz", "t.tar")] {
            let packed = seven_zip(&["a", "-bso0", kind], Some(&root.join(archive))).arg(what).current_dir(&root).status().unwrap();
            assert!(packed.success());
        }
        let mut panel = Panel::new(&root, crate::Filter::default()).unwrap();
        let open = |panel: &mut Panel, name: &str| {
            let at = panel.entries().iter().position(|e| e.display_name() == name).unwrap();
            panel.set_cursor(at);
            panel.open().unwrap();
        };
        open(&mut panel, "t.zip");
        open(&mut panel, "src");
        let (archive, dir) = panel.archive_place().unwrap();
        assert_eq!(dir, Path::new("src"));
        add(&archive, &[root.join("new.txt"), root.join("newdir")], &dir).unwrap();
        panel.reload().unwrap();
        assert_eq!(names(panel.entries()), ["..", "newdir", "sub", "a.txt", "new.txt"]);
        assert!(root.join("newdir/inner.txt").exists(), "the sources stay");
        panel.new_folder("made").unwrap();
        panel.rename_entry(OsStr::new("a.txt"), "renamed.txt").unwrap();
        assert_eq!(names(panel.entries()), ["..", "made", "newdir", "sub", "new.txt", "renamed.txt"]);
        for name in ["newdir", "new.txt"] {
            let at = panel.entries().iter().position(|e| e.display_name() == name).unwrap();
            panel.set_cursor(at);
            panel.toggle_mark();
        }
        panel.delete(false).unwrap();
        assert_eq!(names(panel.entries()), ["..", "made", "sub", "renamed.txt"]);
        open(&mut panel, "sub");
        assert_eq!(names(panel.entries()), ["..", "b.txt"], "a folder's own items stay");
        let mut panel = Panel::new(&root, crate::Filter::default()).unwrap();
        open(&mut panel, "t.tar.gz");
        assert_eq!(panel.archive_place().unwrap_err().kind(), io::ErrorKind::Unsupported);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn browses_archives_with_the_bundled_7zip() {
        let _alone = shared();
        assert!(
            executable().exists(),
            "run vendor/7zip/fetch.sh or fetch.bat first"
        );
        let root = std::env::temp_dir().join(format!("pc-archive-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/sub")).unwrap();
        fs::write(root.join("src/a.txt"), "a").unwrap();
        fs::write(root.join("src/sub/b.txt"), "b").unwrap();
        fs::write(root.join("src/sub/b[1].txt"), "b[1]").unwrap();
        fs::write(root.join("src/sub/b1.txt"), "b1").unwrap();
        let pack = |kind: &str, archive: &str, what: &str| {
            let status = seven_zip(&["a", "-bso0", kind], Some(&root.join(archive)))
                .arg(what)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(status.success());
        };
        pack("-tzip", "-t.zip", "src");
        pack("-ttar", "t.tar", "src");
        pack("-tgzip", "t.tar.gz", "t.tar");
        fs::write(root.join("bad.zip"), "not a zip").unwrap();

        let mut panel = Panel::new(&root, crate::Filter::default()).unwrap();
        let at = |panel: &Panel, name: &str| {
            panel
                .entries()
                .iter()
                .position(|e| e.display_name() == name)
                .unwrap()
        };
        for archive in ["-t.zip", "t.tar.gz"] {
            panel.set_cursor(at(&panel, archive));
            assert!(panel.open().unwrap());
            assert_eq!(panel.path(), root.join(archive));
            assert_eq!(names(panel.entries()), ["..", "src"]);
            panel.set_cursor(1);
            panel.open().unwrap();
            assert_eq!(names(panel.entries()), ["..", "sub", "a.txt"]);
            assert_eq!(panel.path(), root.join(archive).join("src"));
            panel.set_cursor(2);
            let a = panel.archive_member().unwrap().extract().unwrap();
            assert_eq!(fs::read_to_string(&a).unwrap(), "a");
            panel.set_cursor(1);
            assert!(panel.archive_member().is_none(), "a folder");
            panel.open().unwrap();
            panel.set_cursor(at(&panel, "b[1].txt"));
            let b = panel.archive_member().unwrap().extract().unwrap();
            assert_eq!(fs::read_to_string(&b).unwrap(), "b[1]", "no wildcard");
            assert_eq!(fs::read_dir(b.parent().unwrap()).unwrap().count(), 1);
            panel.up().unwrap();
            panel.up().unwrap();
            assert_eq!(panel.cursor(), 1, "on the folder it left");
            panel.up().unwrap();
            assert_eq!(panel.path(), root, "out of the archive");
            assert_eq!(panel.entries()[panel.cursor()].display_name(), archive);
        }

        // F5 out of it: a folder with all in it; F4 in it: back into the archive.
        for archive in ["-t.zip", "t.tar.gz"] {
            panel.set_cursor(at(&panel, archive));
            panel.open().unwrap();
            panel.set_cursor(1);
            panel.open().unwrap();
            panel.set_cursor(at(&panel, "sub"));
            let taken = panel.extract_chosen().unwrap();
            assert!(taken[0].ends_with("src/sub"), "{taken:?}");
            assert_eq!(fs::read_dir(&taken[0]).unwrap().count(), 3);
            panel.set_cursor(at(&panel, "a.txt"));
            let member = panel.archive_member().unwrap();
            assert_eq!(member.updatable(), archive == "-t.zip");
            if member.updatable() {
                let (dir, file) = member.extract_with_folders().unwrap();
                assert_eq!(file, dir.join("src/a.txt"));
                fs::write(&file, "edited").unwrap();
                member.update(&dir).unwrap();
                panel.reload().unwrap();
                let again = panel.archive_member().unwrap().extract().unwrap();
                assert_eq!(fs::read_to_string(again).unwrap(), "edited");
                assert_eq!(
                    names(panel.entries()),
                    ["..", "sub", "a.txt"],
                    "nothing added"
                );
            }
            panel.up().unwrap();
            panel.up().unwrap();
        }

        panel.set_cursor(at(&panel, "bad.zip"));
        let err = panel.open().unwrap_err().to_string();
        assert!(err.starts_with("bad.zip: "), "{err}");
        assert_eq!(panel.path(), root, "stays where it was");
        fs::remove_dir_all(&root).unwrap();
        remove_scratch();
        assert!(!scratch().exists());
    }

    #[test]
    fn removes_what_ended_runs_left() {
        let temp = std::env::temp_dir().join(format!("pc-stale-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        let run = |name: &str| {
            let dir = temp.join(format!("{SCRATCH}{name}"));
            fs::create_dir_all(dir.join("0")).unwrap();
            File::create(dir.join(LOCK)).unwrap()
        };
        drop(run("ended"));
        let running = run("running");
        running.lock().unwrap();
        let _ours = run("ours");
        fs::create_dir(temp.join("other")).unwrap();
        remove_stale(&temp, &temp.join(format!("{SCRATCH}ours")));
        let mut left: Vec<_> = fs::read_dir(&temp)
            .unwrap()
            .map(|dir| dir.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["other", "powercommander-ours", "powercommander-running"]
        );
        fs::remove_dir_all(&temp).unwrap();
    }
}
