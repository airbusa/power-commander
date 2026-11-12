//! Internal associations: by file mask, the programs Enter, F3 and F4 run
//! instead of the system's and the built-in ones. Without one, Enter runs
//! programs and opens other files as the system does.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::edit;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Association {
    /// `*.jpg *.png`, or `;` between them.
    pub mask: String,
    /// Command lines: `%1` is the file, else it goes last; empty keeps the usual.
    pub open: String,
    pub view: String,
    pub edit: String,
}

/// `*` any run of characters, `?` one; any of the masks, case aside.
pub fn matches(masks: &str, name: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    masks
        .split([' ', ';'])
        .filter(|mask| !mask.is_empty())
        .any(|mask| wildcard(&mask.to_lowercase().chars().collect::<Vec<_>>(), &name))
}

/// One glob, `*` and `?`; case aside unless `case_sensitive`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn wildcard_match(mask: &str, name: &str, case_sensitive: bool) -> bool {
    let chars = |text: &str| -> Vec<char> {
        match case_sensitive {
            true => text.chars().collect(),
            false => text.to_lowercase().chars().collect(),
        }
    };
    wildcard(&chars(mask), &chars(name))
}

fn wildcard(mask: &[char], name: &[char]) -> bool {
    // Where the last `*` was, and where in the name it began to match.
    let (mut m, mut n, mut star) = (0, 0, None);
    while n < name.len() {
        match mask.get(m) {
            Some('*') => {
                star = Some((m, n));
                m += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                m += 1;
                n += 1;
            }
            _ => match star {
                Some((at, from)) => {
                    m = at + 1;
                    n = from + 1;
                    star = Some((at, from + 1));
                }
                None => return false,
            },
        }
    }
    mask[m..].iter().all(|&c| c == '*')
}

/// The first association whose mask the name matches.
pub fn find<'a>(list: &'a [Association], name: &str) -> Option<&'a Association> {
    list.iter().find(|a| matches(&a.mask, name))
}

/// The mask "Associate With" starts from: `*.txt`, or the whole name.
pub fn mask_for(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("*{}", name[dot..].to_lowercase()),
        _ => name.to_owned(),
    }
}

/// Enter on a file: its association's program, else the file itself if it
/// is a program, else the system's choice.
pub fn open(list: &[Association], file: &Path, executable: bool) -> io::Result<()> {
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    if let Some(found) = find(list, &name).filter(|a| !a.open.trim().is_empty()) {
        return edit::edit_externally(&found.open, file);
    }
    if executable {
        let mut child = Command::new(file)
            .current_dir(file.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::spawn(move || child.wait());
        return Ok(());
    }
    edit::edit_externally(SYSTEM, file)
}

/// The program that opens a file as the desktop would.
const SYSTEM: &str = match () {
    _ if cfg!(windows) => "explorer.exe",
    _ if cfg!(target_os = "macos") => "open",
    _ => "xdg-open",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_masks() {
        assert!(matches("*.jpg *.png", "Holiday.JPG"));
        assert!(matches("*.tar.gz;*.tgz", "a.b.tar.gz"));
        assert!(matches("img_????.*", "IMG_1234.raw"));
        assert!(!matches("img_????.*", "IMG_12345.raw"));
        assert!(matches("*", "anything"));
        assert!(!matches("*.txt", "txt"));
        assert!(matches("*a*b*c", "xaxxbxxxc"));
        assert!(!matches("", "a"));
        let list = [
            Association {
                mask: "*.md".into(),
                view: "glow %1".into(),
                ..Association::default()
            },
            Association {
                mask: "*".into(),
                open: "other".into(),
                ..Association::default()
            },
        ];
        assert_eq!(find(&list, "README.MD").unwrap().view, "glow %1");
        assert_eq!(mask_for("Photo.JPG"), "*.jpg");
        assert_eq!(mask_for("Makefile"), "Makefile");
    }
}
