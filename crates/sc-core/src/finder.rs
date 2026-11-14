//! macOS: what Finder's context menu does beyond Spark Commander's own
//! commands, by the system's tools.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Copies each beside itself, named as Finder's Duplicate names them:
/// `Name copy.ext`, then `Name copy 2.ext`; the copies.
pub fn duplicate(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    paths
        .iter()
        .map(|path| {
            let copy = (1..).map(|n| copy_name(path, n)).find(|copy| !copy.exists()).expect("a free name");
            run(Command::new("/bin/cp").arg("-Rp").arg(path).arg(&copy))?;
            Ok(copy)
        })
        .collect()
}

/// `Name copy.ext`, `Name copy 2.ext`; a folder's name stays whole, an application's keeps `.app`.
fn copy_name(path: &Path, n: u32) -> PathBuf {
    let suffix = match n {
        1 => " copy".to_owned(),
        _ => format!(" copy {n}"),
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let folder = path.is_dir() && path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("app"));
    let named = match (folder, name.rfind('.')) {
        (false, Some(dot)) if dot > 0 => format!("{}{suffix}{}", &name[..dot], &name[dot..]),
        _ => format!("{name}{suffix}"),
    };
    path.with_file_name(named)
}

/// Finder's aliases of each, beside them (`Name alias`).
pub fn make_alias(paths: &[PathBuf]) -> io::Result<()> {
    const SCRIPT: &str = r#"on run argv
    repeat with p in argv
        set target to (POSIX file (p as text)) as alias
        tell application "Finder" to make alias file to target at (container of target)
    end repeat
end run"#;
    run(Command::new("osascript").arg("-e").arg(SCRIPT).args(paths))
}

/// Finder's window, each selected in its folder.
pub fn reveal(paths: &[PathBuf]) -> io::Result<()> {
    run(Command::new("open").arg("-R").args(paths))
}

/// Quick Look's preview; it stays until closed.
pub fn quick_look(paths: &[PathBuf]) -> io::Result<()> {
    let mut preview = Command::new("qlmanage").arg("-p").args(paths).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    // Waited for, so it leaves no zombie behind.
    std::thread::spawn(move || preview.wait());
    Ok(())
}

/// Each opened by the application `app`.
pub fn open_with(app: &Path, paths: &[PathBuf]) -> io::Result<()> {
    run(Command::new("open").arg("-a").arg(app).args(paths))
}

fn run(command: &mut Command) -> io::Result<()> {
    let output = command.stdin(Stdio::null()).output()?;
    match output.status.success() {
        true => Ok(()),
        false => Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_copies_as_finder() {
        let dir = std::env::temp_dir().join(format!("pc-finder-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Folder.d")).unwrap();
        std::fs::create_dir_all(dir.join("Tool.app")).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        let name = |name: &str, n| copy_name(&dir.join(name), n).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name("notes.txt", 1), "notes copy.txt");
        assert_eq!(name("notes.txt", 2), "notes copy 2.txt");
        assert_eq!(name("Folder.d", 1), "Folder.d copy");
        assert_eq!(name("Tool.app", 1), "Tool copy.app");
        let copies = duplicate(&[dir.join("notes.txt"), dir.join("notes.txt")]).unwrap();
        assert_eq!(copies, [dir.join("notes copy.txt"), dir.join("notes copy 2.txt")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
