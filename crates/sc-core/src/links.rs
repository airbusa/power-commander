//! Files > Create Link: symbolic and hard links; on Windows junctions and
//! shortcuts (.lnk) too.

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Symbolic,
    /// Files only.
    Hard,
    /// Windows', folders only.
    Junction,
    /// Windows' .lnk file.
    Shortcut,
}

impl Kind {
    /// This system's.
    pub const ALL: &[Kind] = match cfg!(windows) {
        true => &[Kind::Symbolic, Kind::Hard, Kind::Junction, Kind::Shortcut],
        false => &[Kind::Symbolic, Kind::Hard],
    };

    pub fn label(self) -> &'static str {
        match self {
            Kind::Symbolic => "Symbolic link",
            Kind::Hard => "Hard link",
            Kind::Junction => "Junction",
            Kind::Shortcut => "Shortcut (.lnk)",
        }
    }

    /// Whether it can point at what is chosen: some `folders`, some `files`.
    pub fn fits(self, folders: bool, files: bool) -> bool {
        match self {
            Kind::Hard => !folders,
            Kind::Junction => !files,
            _ => true,
        }
    }
}

/// Where a `kind` link asked for at `link` goes: a shortcut ends in .lnk.
pub fn named(kind: Kind, link: &Path) -> PathBuf {
    match kind == Kind::Shortcut && !link.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) {
        true => PathBuf::from(format!("{}.lnk", link.display())),
        false => link.to_owned(),
    }
}

/// Each `(target, link)` in turn, stopping at the first that fails: the
/// links made, and how it ended.
pub fn make_all(kind: Kind, items: &[(PathBuf, PathBuf)]) -> (Vec<PathBuf>, io::Result<()>) {
    let mut made = Vec::new();
    for (target, link) in items {
        let link = named(kind, link);
        if let Err(err) = platform::make(kind, target, &link) {
            return (made, Err(err));
        }
        made.push(link);
    }
    (made, Ok(()))
}

/// Makes them, noted for Undo and in the activity log; failed, why and
/// those not made.
pub fn create(kind: Kind, items: &[(PathBuf, PathBuf)]) -> Result<(), (io::Error, Vec<(PathBuf, PathBuf)>)> {
    let title = match items {
        [(target, link)] => format!(
            "{} to \"{}\" in {}",
            kind.label(),
            target.file_name().unwrap_or_default().to_string_lossy(),
            link.parent().unwrap_or(Path::new("")).display()
        ),
        _ => format!("{}s to {} items in {}", kind.label(), items.len(), items[0].1.parent().unwrap_or(Path::new("")).display()),
    };
    let (made, done) = make_all(kind, items);
    crate::activity::note(&title, &done);
    let rest = items[made.len()..].to_vec();
    if !made.is_empty() {
        crate::undo::remember(title, crate::undo::Step::Copied { fs: std::sync::Arc::new(crate::vfs::Local), made });
    }
    done.map_err(|err| (err, rest))
}

#[cfg(unix)]
mod platform {
    use super::*;

    pub fn make(kind: Kind, target: &Path, link: &Path) -> io::Result<()> {
        match kind {
            Kind::Hard => std::fs::hard_link(target, link),
            Kind::Symbolic => std::os::unix::fs::symlink(target, link),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "junctions and shortcuts are Windows'")),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};

    /// ERROR_PRIVILEGE_NOT_HELD.
    const NOT_HELD: i32 = 1314;

    pub fn make(kind: Kind, target: &Path, link: &Path) -> io::Result<()> {
        match kind {
            Kind::Hard => std::fs::hard_link(target, link),
            Kind::Symbolic => {
                let made = match target.is_dir() {
                    true => std::os::windows::fs::symlink_dir(target, link),
                    false => std::os::windows::fs::symlink_file(target, link),
                };
                made.map_err(|err| match err.raw_os_error() {
                    Some(NOT_HELD) => io::Error::new(io::ErrorKind::PermissionDenied, "symbolic links need an administrator, or Developer Mode"),
                    _ => err,
                })
            }
            Kind::Junction => junction(target, link),
            Kind::Shortcut => {
                let (target, link) = (target.to_owned(), link.to_owned());
                // Its own thread: COM as the shell wants it, whatever the caller's.
                std::thread::spawn(move || shortcut(&target, &link)).join().map_err(|_| io::Error::other("the shortcut's thread failed"))?
            }
        }
    }

    /// cmd's, as making one by hand is a reparse buffer.
    fn junction(target: &Path, link: &Path) -> io::Result<()> {
        const NO_WINDOW: u32 = 0x0800_0000;
        let line = format!("/c mklink /J \"{}\" \"{}\"", link.display(), target.display());
        let done = std::process::Command::new("cmd").raw_arg(line).creation_flags(NO_WINDOW).output()?;
        match done.status.success() {
            true => Ok(()),
            false => Err(io::Error::other(String::from_utf8_lossy(&done.stderr).trim().to_owned())),
        }
    }

    fn shortcut(target: &Path, link: &Path) -> io::Result<()> {
        let fail = |err: windows::core::Error| io::Error::other(err.message());
        // SAFETY: COM is started and ended on this thread; the strings outlive the calls.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(fail)?;
            let made = (|| {
                let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                shell.SetPath(&HSTRING::from(target.as_os_str()))?;
                if let Some(parent) = target.parent() {
                    shell.SetWorkingDirectory(&HSTRING::from(parent.as_os_str()))?;
                }
                shell.cast::<IPersistFile>()?.Save(&HSTRING::from(link.as_os_str()), true)
            })();
            CoUninitialize();
            made.map_err(fail)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn makes_links_and_stops_at_a_name_taken() {
        let dir = std::env::temp_dir().join(format!("pc-links-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("into")).unwrap();
        fs::write(dir.join("a.txt"), "a").unwrap();
        let (made, done) = make_all(Kind::Symbolic, &[(dir.join("a.txt"), dir.join("into/a.txt")), (dir.join("into"), dir.join("into/folder"))]);
        assert!(done.is_ok() && made.len() == 2);
        assert_eq!(fs::read_to_string(dir.join("into/a.txt")).unwrap(), "a");
        assert!(dir.join("into/folder").symlink_metadata().unwrap().is_symlink());
        let (made, done) = make_all(Kind::Hard, &[(dir.join("a.txt"), dir.join("b.txt")), (dir.join("a.txt"), dir.join("b.txt")), (dir.join("a.txt"), dir.join("c.txt"))]);
        assert_eq!(made, vec![dir.join("b.txt")]);
        assert_eq!(done.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(!dir.join("c.txt").exists(), "stopped at the first that failed");
        assert_eq!(named(Kind::Shortcut, &dir.join("a.txt")), dir.join("a.txt.lnk"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
