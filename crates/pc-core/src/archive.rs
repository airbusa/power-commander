//! Archives through the bundled 7-Zip.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::SystemTime;

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
    pub entry: Entry,
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
        let Some(path) = field("Path").and_then(clean) else {
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
        items.push(Item { path, entry });
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
            items.push(Item { path: dir, entry });
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
            panel.up().unwrap();
            assert_eq!(panel.cursor(), 1, "on the folder it left");
            panel.up().unwrap();
            assert_eq!(panel.path(), root, "out of the archive");
            assert_eq!(panel.entries()[panel.cursor()].display_name(), archive);
        }

        panel.set_cursor(at(&panel, "bad.zip"));
        let err = panel.open().unwrap_err().to_string();
        assert!(err.starts_with("bad.zip: "), "{err}");
        assert_eq!(panel.path(), root, "stays where it was");
        fs::remove_dir_all(&root).unwrap();
    }
}
