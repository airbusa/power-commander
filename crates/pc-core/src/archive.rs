//! Archives through the bundled 7-Zip.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
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
        let name = self.archive.file_name().unwrap_or_default();
        extension(name).is_some_and(|ext| UPDATABLE.contains(&ext.as_str()))
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

/// Where files taken out of archives go while the app runs.
pub fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("{SCRATCH}{}", std::process::id()))
}

const SCRATCH: &str = "powercommander-";
/// Locked while its run lasts; the system unlocks it when the run ends.
const LOCK: &str = ".lock";

/// This run's lock, once `scratch()` exists.
static HELD: Mutex<Option<File>> = Mutex::new(None);

/// A new folder under `scratch()`, so files of one name never meet.
pub fn fresh_scratch() -> io::Result<PathBuf> {
    static TAKEN: AtomicUsize = AtomicUsize::new(0);
    let dir = claim()?.join(TAKEN.fetch_add(1, Ordering::Relaxed).to_string());
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Creates `scratch()` and locks it, at the first use.
fn claim() -> io::Result<PathBuf> {
    let dir = scratch();
    let mut held = HELD.lock().unwrap();
    if held.is_none() {
        fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.lock()?;
        *held = Some(lock);
    }
    Ok(dir)
}

/// Removes `scratch()` as the app quits; Windows deletes no open file, so unlocked first.
pub fn remove_scratch() {
    HELD.lock().unwrap().take();
    let _ = fs::remove_dir_all(scratch());
}

/// Removes the folders runs left behind when killed or crashed.
pub fn remove_stale_scratch() {
    remove_stale(&std::env::temp_dir(), &scratch());
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
    let output = if is_compressed_tar(archive) {
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
fn is_compressed_tar(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default();
    let stem = Path::new(Path::new(name).file_stem().unwrap_or_default());
    extension(name).is_some_and(|ext| TARS.contains(&ext.as_str()))
        || extension(stem.as_os_str()).as_deref() == Some("tar")
}

pub(crate) fn list(archive: &Path) -> io::Result<Vec<Item>> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let fail = |err: String| io::Error::other(format!("{name}: {err}"));
    let output = if is_compressed_tar(archive) {
        let mut unpack = seven_zip(&["x", "-so"], Some(archive))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(missing)?;
        let stdout = unpack.stdout.take().expect("piped");
        let listing = seven_zip(&["l", "-slt", "-ba", "-si", "-ttar"], None)
            .stdin(stdout)
            .output()
            .map_err(missing)?;
        let unpacked = unpack.wait()?;
        if listing.status.success() && !unpacked.success() {
            return Err(fail(failure(unpacked, b"")));
        }
        listing
    } else {
        seven_zip(&["l", "-slt", "-ba"], Some(archive))
            .output()
            .map_err(missing)?
    };
    if !output.status.success() {
        return Err(fail(failure(output.status, &output.stderr)));
    }
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
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

// Releases ship it in `7zip/` beside the executable; source builds use `vendor/7zip`.
fn executable() -> PathBuf {
    let name = if cfg!(windows) { "7z.exe" } else { "7zz" };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("7zip").join(name)));
    match beside {
        Some(path) if path.exists() => path,
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/7zip")
            .join(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
            .join(name),
    }
}

fn missing(err: io::Error) -> io::Error {
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

    #[test]
    fn browses_archives_with_the_bundled_7zip() {
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
