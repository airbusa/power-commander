//! Text files for the editor: refused when editing could change them by accident.

use std::fs;
use std::io;
use std::path::Path;

/// The largest file F4 edits; F3 views any size.
pub const EDIT_LIMIT: u64 = 8 << 20;

pub fn load_text(path: &Path) -> io::Result<String> {
    let refuse = |why: &str| Err(io::Error::new(io::ErrorKind::InvalidData, why));
    if fs::metadata(path)?.len() > EDIT_LIMIT {
        return refuse("too large to edit (over 8 MB); F3 views it");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_saves_text() {
        let dir = std::env::temp_dir().join(format!("pc-edit-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "one\r\ntwo").unwrap();
        assert_eq!(load_text(&file).unwrap(), "one\r\ntwo", "line ends kept");
        save_text(&file, "three\r\n").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "three\r\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary left");

        fs::write(dir.join("bin"), b"a\0b").unwrap();
        assert!(load_text(&dir.join("bin")).is_err());
        fs::write(dir.join("latin1"), b"caf\xe9").unwrap();
        assert!(load_text(&dir.join("latin1")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
