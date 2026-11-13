//! What the user may not do, done as the system's administrator, as Total
//! Commander and Double Commander offer: one password prompt for the lot,
//! polkit's (pkexec) on Linux, AppleScript's on macOS, UAC on Windows.

use std::io;
use std::path::PathBuf;

pub enum Action {
    /// Each `(source, target)` whole, replacing what is there; with `remove`
    /// the sources go after (a move).
    Copy { items: Vec<(PathBuf, PathBuf)>, remove: bool },
    /// For good: the administrator has no trash of the user's.
    Delete(Vec<PathBuf>),
    MakeFolder(PathBuf),
    Rename(PathBuf, PathBuf),
}

/// Runs `action` as administrator, the system asking for the password;
/// declined, it fails as `Interrupted`. Blocks.
pub fn run(action: &Action) -> io::Result<()> {
    platform::run(action)
}

#[cfg(unix)]
fn quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// The shell's lines for `action`, stopping at the first that fails.
#[cfg(unix)]
pub(crate) fn script(action: &Action) -> String {
    let lines: Vec<String> = match action {
        Action::Copy { items, remove } => items
            .iter()
            .map(|(source, target)| {
                let (s, t) = (quote(source), quote(target));
                // A folder's content into the target, which may be there in part.
                let copy = match source.is_dir() {
                    true => format!("mkdir -p {t} && cp -a {s}/. {t}/"),
                    false => format!("cp -a {s} {t}"),
                };
                match remove {
                    true => format!("{copy} && rm -rf {s}"),
                    false => copy,
                }
            })
            .collect(),
        Action::Delete(paths) => vec![format!("rm -rf -- {}", paths.iter().map(|p| quote(p)).collect::<Vec<_>>().join(" "))],
        Action::MakeFolder(path) => vec![format!("mkdir -p -- {}", quote(path))],
        Action::Rename(from, to) => vec![format!("mv -- {} {}", quote(from), quote(to))],
    };
    format!("set -e\n{}\n", lines.join("\n"))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;
    use std::process::Command;

    pub fn run(action: &Action) -> io::Result<()> {
        let done = Command::new("pkexec").args(["/bin/sh", "-c", &script(action)]).output();
        let done = done.map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => io::Error::new(err.kind(), "pkexec (polkit) is not installed"),
            _ => err,
        })?;
        match done.status.code() {
            Some(0) => Ok(()),
            // pkexec's: the prompt dismissed, or not allowed.
            Some(126) => Err(io::Error::new(io::ErrorKind::Interrupted, "the password was not given")),
            Some(127) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "not authorised")),
            _ => Err(io::Error::other(String::from_utf8_lossy(&done.stderr).trim().to_owned())),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::process::Command;

    pub fn run(action: &Action) -> io::Result<()> {
        let shell = script(action).replace('\\', r"\\").replace('"', "\\\"");
        let apple = format!("do shell script \"{shell}\" with administrator privileges");
        let done = Command::new("osascript").args(["-e", &apple]).output()?;
        let why = String::from_utf8_lossy(&done.stderr).trim().to_owned();
        match done.status.success() {
            true => Ok(()),
            // AppleScript's -128: the prompt cancelled.
            false if why.contains("-128") => Err(io::Error::new(io::ErrorKind::Interrupted, "the password was not given")),
            false => Err(io::Error::other(why)),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{HSTRING, PCWSTR};

    fn quote(path: &std::path::Path) -> String {
        format!("\"{}\"", path.display())
    }

    /// cmd's commands for `action`, stopping at the first that fails.
    pub(crate) fn commands(action: &Action) -> String {
        let remove = |path: &std::path::Path| match path.is_dir() {
            true => format!("rmdir /S /Q {}", quote(path)),
            false => format!("del /F /Q {}", quote(path)),
        };
        let lines: Vec<String> = match action {
            Action::Copy { items, remove: moved } => items
                .iter()
                .map(|(source, target)| {
                    let copy = match source.is_dir() {
                        true => format!("xcopy {} {}\\ /E /I /H /Y /K /Q", quote(source), quote(target)),
                        false => format!("copy /Y {} {}", quote(source), quote(target)),
                    };
                    match moved {
                        true => format!("{copy} && {}", remove(source)),
                        false => copy,
                    }
                })
                .collect(),
            Action::Delete(paths) => paths.iter().map(|path| remove(path)).collect(),
            Action::MakeFolder(path) => vec![format!("mkdir {}", quote(path))],
            Action::Rename(from, to) => vec![format!("move /Y {} {}", quote(from), quote(to))],
        };
        lines.join(" && ")
    }

    pub fn run(action: &Action) -> io::Result<()> {
        let (verb, file, parameters) = (HSTRING::from("runas"), HSTRING::from("cmd.exe"), HSTRING::from(format!("/c {}", commands(action))));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: the strings outlive the call; the process handle is closed below.
        unsafe {
            if let Err(err) = ShellExecuteExW(&mut info) {
                return Err(match err.code() == ERROR_CANCELLED.to_hresult() {
                    true => io::Error::new(io::ErrorKind::Interrupted, "the administrator's consent was not given"),
                    false => io::Error::other(err.to_string()),
                });
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1;
            let got = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            got.map_err(|err| io::Error::other(err.to_string()))?;
            match code {
                0 => Ok(()),
                code => Err(io::Error::other(format!("as administrator it failed too ({code})"))),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    /// The scripts run here unprivileged: what they do, not who may.
    fn run_here(action: &Action) {
        let ran = Command::new("/bin/sh").args(["-c", &script(action)]).status().unwrap();
        assert!(ran.success());
    }

    #[test]
    fn copies_moves_deletes_and_renames() {
        let dir = std::env::temp_dir().join(format!("pc-elevate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src/sub")).unwrap();
        fs::create_dir_all(dir.join("dst/folder")).unwrap();
        fs::write(dir.join("src/sub/it's.txt"), "q").unwrap();
        fs::write(dir.join("src/one.txt"), "1").unwrap();
        // A folder copied in part already: its content goes into it, not inside.
        fs::write(dir.join("dst/folder/old.txt"), "old").unwrap();
        run_here(&Action::Copy { items: vec![(dir.join("src/sub"), dir.join("dst/folder"))], remove: false });
        assert_eq!(fs::read_to_string(dir.join("dst/folder/it's.txt")).unwrap(), "q");
        assert!(dir.join("dst/folder/old.txt").exists() && !dir.join("dst/folder/sub").exists());
        run_here(&Action::Copy { items: vec![(dir.join("src/one.txt"), dir.join("dst/one.txt"))], remove: true });
        assert!(dir.join("dst/one.txt").exists() && !dir.join("src/one.txt").exists(), "moved");
        run_here(&Action::MakeFolder(dir.join("dst/new folder/deep")));
        run_here(&Action::Rename(dir.join("dst/one.txt"), dir.join("dst/two.txt")));
        assert!(dir.join("dst/new folder/deep").is_dir() && dir.join("dst/two.txt").exists());
        run_here(&Action::Delete(vec![dir.join("dst/new folder"), dir.join("dst/two.txt")]));
        assert!(!dir.join("dst/new folder").exists() && !dir.join("dst/two.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
