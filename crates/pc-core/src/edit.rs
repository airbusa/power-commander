//! Text files for the editor: refused when editing could change them by accident.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Up to `limit_mb`, as Settings' Edit/View page says.
pub fn load_text(path: &Path, limit_mb: u64) -> io::Result<String> {
    let refuse = |why: &str| Err(io::Error::new(io::ErrorKind::InvalidData, why));
    if fs::metadata(path)?.len() > limit_mb << 20 {
        let why = format!(
            "over the built-in editor's {limit_mb} MB (Settings, Edit/View); Ctrl+Shift+F4 edits it in the external one"
        );
        return refuse(&why);
    }
    let bytes = fs::read(path)?;
    if bytes.contains(&0) {
        return refuse("a binary file; F3 views it");
    }
    String::from_utf8(bytes).or_else(|_| refuse("not UTF-8 text; saving would change it"))
}

/// Written beside the file and renamed over it, with its permissions; a
/// link's target is what gets saved.
pub fn save_text(path: &Path, text: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    let permissions = fs::metadata(&target)?.permissions();
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temp = target.with_file_name(format!(".{name}.pc-save"));
    let written = fs::write(&temp, text)
        .and_then(|()| fs::set_permissions(&temp, permissions))
        .and_then(|()| fs::rename(&temp, &target));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// Starts the external editor on `file`, as Ctrl+Shift+F4 does.
pub fn edit_externally(line: &str, file: &Path) -> io::Result<()> {
    let mut child = start_editor(line, file)?;
    // Reaps it once it ends.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The external editor on `file`; waiting for it tells when editing ends,
/// if the editor waits too (`code --wait`).
pub fn start_editor(line: &str, file: &Path) -> io::Result<Child> {
    let (program, args) = command_line(line, file)
        .ok_or_else(|| io::Error::other("no external editor is set (Settings, Edit/View)"))?;
    Command::new(&program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("{}: {err}", program.to_string_lossy())))
}

/// Words of `line`, `"quoted"` ones with their spaces; `%1` becomes `file`,
/// which goes last when no word has it.
fn command_line(line: &str, file: &Path) -> Option<(OsString, Vec<OsString>)> {
    let mut words = Vec::new();
    let (mut word, mut quoted, mut started) = (String::new(), false, false);
    for c in line.chars() {
        match c {
            '"' => (quoted, started) = (!quoted, true),
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            }
            c => (word, started) = (word + &c.to_string(), true),
        }
    }
    if started {
        words.push(word);
    }
    let mut placed = false;
    let mut args: Vec<OsString> = words
        .iter()
        .map(|word| {
            let mut arg = OsString::new();
            for (i, part) in word.split("%1").enumerate() {
                if i > 0 {
                    arg.push(file);
                    placed = true;
                }
                arg.push(part);
            }
            arg
        })
        .collect();
    if !placed {
        args.push(file.into());
    }
    let program = (!words.is_empty()).then(|| args.remove(0))?;
    Some((program, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_external_editors_command() {
        let file = Path::new("/tmp/a b.txt");
        let run = |line: &str| {
            let (program, args) = command_line(line, file).unwrap();
            let mut all = vec![program];
            all.extend(args);
            all.into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(run("xdg-open"), ["xdg-open", "/tmp/a b.txt"]);
        assert_eq!(run("code  --wait %1"), ["code", "--wait", "/tmp/a b.txt"]);
        assert_eq!(
            run(r#""C:\Program Files\Ed\ed.exe" --file="%1" -n"#),
            [r"C:\Program Files\Ed\ed.exe", "--file=/tmp/a b.txt", "-n"]
        );
        assert!(command_line("  ", file).is_none());
    }

    #[test]
    fn loads_and_saves_text() {
        let dir = std::env::temp_dir().join(format!("pc-edit-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "one\r\ntwo").unwrap();
        assert_eq!(
            load_text(&file, 32).unwrap(),
            "one\r\ntwo",
            "line ends kept"
        );
        save_text(&file, "three\r\n").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "three\r\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary left");

        fs::write(dir.join("bin"), b"a\0b").unwrap();
        assert!(load_text(&dir.join("bin"), 32).is_err());
        fs::write(dir.join("latin1"), b"caf\xe9").unwrap();
        assert!(load_text(&dir.join("latin1"), 32).is_err());
        assert!(load_text(&file, 0).is_err(), "over the limit");
        fs::remove_dir_all(&dir).unwrap();
    }
}
