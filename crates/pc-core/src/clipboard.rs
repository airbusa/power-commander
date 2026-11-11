//! Files on the clipboard as text: the paths this program puts there, and
//! the `file://` addresses or paths other programs do.

use std::ffi::OsString;
use std::path::PathBuf;

/// The files' text: their paths, a line each.
pub fn text_of(paths: &[PathBuf]) -> String {
    let lines: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
    lines.join("\n")
}

/// A path as a `file://` address, other bytes than letters, digits and
/// `-._~/` written `%XX`.
pub fn file_uri(path: &std::path::Path) -> String {
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();
    let mut uri = String::from("file://");
    for byte in bytes {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The files a text names, a line each, as `file://` addresses or
/// absolute paths, all of them there; none for any other text.
pub fn paths_in(text: &str) -> Option<Vec<PathBuf>> {
    let lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let paths = lines.map(|line| {
        let path = match line.strip_prefix("file://") {
            Some(rest) => PathBuf::from(decode(rest.strip_prefix("localhost").unwrap_or(rest))?),
            None => PathBuf::from(line),
        };
        (path.is_absolute() && path.exists()).then_some(path)
    });
    paths.collect::<Option<Vec<_>>>().filter(|paths| !paths.is_empty())
}

/// `%20` and the like, as addresses have them.
fn decode(text: &str) -> Option<OsString> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                at += 3;
            }
            byte => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    #[cfg(unix)]
    return Some(std::os::unix::ffi::OsStringExt::from_vec(decoded));
    #[cfg(not(unix))]
    return String::from_utf8(decoded).ok().map(OsString::from);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_files_from_text() {
        let dir = std::env::temp_dir().join(format!("pc-clip-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a b")).unwrap();
        std::fs::write(dir.join("é.txt"), "x").unwrap();
        let (folder, file) = (dir.join("a b"), dir.join("é.txt"));
        let text = text_of(&[folder.clone(), file.clone()]);
        assert_eq!(paths_in(&text), Some(vec![folder.clone(), file.clone()]));
        let uri = format!("file://{}\r\n", dir.display()).replace(' ', "%20") + "a%20b";
        assert_eq!(paths_in(&uri.replace("\r\na%20b", "/a%20b")), Some(vec![folder]));
        let encoded = format!("file://{}/%C3%A9.txt", dir.display());
        assert_eq!(paths_in(&encoded), Some(vec![file.clone()]));
        let uri = file_uri(&dir.join("a b"));
        assert!(uri.ends_with("/a%20b"), "{uri}");
        assert_eq!(paths_in(&file_uri(&file)), Some(vec![file]));
        assert_eq!(paths_in("just some words"), None);
        assert_eq!(paths_in("/no/such/file"), None);
        assert_eq!(paths_in(""), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
