use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub enum Outcome {
    /// `cd`: the panel goes there.
    ChangeDir(PathBuf),
    /// Anything else runs in the shell, on its own.
    Started,
}

/// Runs a command line typed in `dir`, on its own; `failed` gets why when
/// the shell found no such program, or could not run it.
pub fn run(line: &str, dir: &Path, failed: impl FnOnce(String) + Send + 'static) -> io::Result<Outcome> {
    if let Some(target) = change_dir(line, dir) {
        return Ok(Outcome::ChangeDir(target));
    }
    let mut child = shell(line)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let program = line.split_whitespace().next().unwrap_or_default().to_owned();
    // Reaps it once it ends.
    std::thread::spawn(move || {
        if let Ok(status) = child.wait()
            && let Some(why) = not_run(&program, status.code())
        {
            failed(why);
        }
    });
    Ok(Outcome::Started)
}

/// The shell's exit code for a program it could not find or run.
fn not_run(program: &str, code: Option<i32>) -> Option<String> {
    match code? {
        127 if cfg!(unix) => Some(format!("{program}: command not found")),
        126 if cfg!(unix) => Some(format!("{program}: cannot be run")),
        9009 if cfg!(windows) => Some(format!("{program}: command not found")),
        _ => None,
    }
}

/// Where a `cd` line goes; none for other lines.
pub fn change_dir(line: &str, dir: &Path) -> Option<PathBuf> {
    let target = line
        .trim()
        .strip_prefix("cd")
        .filter(|rest| rest.is_empty() || rest.starts_with(' '))?;
    Some(resolve(target.trim().trim_matches('"'), dir))
}

/// The system's shell, to run `line`.
pub fn shell(line: &str) -> Command {
    let line = line.trim();
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", line]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", line]);
        command
    }
}

/// A typed path: `~`, relative to `dir`, `..` resolved.
pub fn resolve(target: &str, dir: &Path) -> PathBuf {
    let home = std::env::home_dir().unwrap_or_default();
    let path = match target {
        "" | "~" => home,
        _ => match target.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => dir.join(target),
        },
    };
    // `..` and `.` resolved without following links, as the panel shows paths.
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            part => clean.push(part),
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd(line: &str) -> PathBuf {
        match run(line, Path::new("/a/b"), |_| {}).unwrap() {
            Outcome::ChangeDir(path) => path,
            Outcome::Started => panic!("{line} ran"),
        }
    }

    #[test]
    fn changes_dir() {
        assert_eq!(cd("cd .."), Path::new("/a"));
        assert_eq!(cd("cd ../c/./d"), Path::new("/a/c/d"));
        assert_eq!(cd("cd /x"), Path::new("/x"));
        assert_eq!(cd("cd \"my dir\""), Path::new("/a/b/my dir"));
        assert_eq!(cd("cd"), std::env::home_dir().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn runs_in_the_shell() {
        let dir = std::env::temp_dir().join(format!("pc-command-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(run("touch made", &dir, |_| {}).unwrap(), Outcome::Started));
        for _ in 0..50 {
            if dir.join("made").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(dir.join("made").exists(), "ran in the panel's folder");
        let (sent, failed) = std::sync::mpsc::channel();
        assert!(
            matches!(run("cdrom-tool-pc-test", &dir, move |why| sent.send(why).unwrap()), Ok(Outcome::Started)),
            "not cd"
        );
        let why = failed.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(why.as_deref(), Ok("cdrom-tool-pc-test: command not found"));
        let (sent, failed) = std::sync::mpsc::channel();
        run("exit 3", &dir, move |why| sent.send(why).unwrap()).unwrap();
        assert!(failed.recv_timeout(std::time::Duration::from_secs(5)).is_err(), "a program's own failure is its own");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
