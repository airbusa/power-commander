//! Change Attributes: permission bits set or cleared, and the modification
//! time, optionally on everything inside the chosen folders.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::TimeZone;

use crate::ops::{Report, Task};
use crate::vfs::{Fs, Meta};

/// The permission bits the dialog shows: `rwxrwxrwx`.
pub const BITS: [(u32, char); 9] = [
    (0o400, 'r'),
    (0o200, 'w'),
    (0o100, 'x'),
    (0o040, 'r'),
    (0o020, 'w'),
    (0o010, 'x'),
    (0o004, 'r'),
    (0o002, 'w'),
    (0o001, 'x'),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    /// Bits turned on, and bits turned off; the rest stay as they are.
    pub set: u32,
    pub clear: u32,
    pub modified: Option<SystemTime>,
    pub recurse: bool,
}

impl Change {
    pub fn mode(&self, old: u32) -> u32 {
        (old & !self.clear) | self.set
    }

    fn changes_mode(&self) -> bool {
        self.set != 0 || self.clear != 0
    }
}

/// Each bit of `BITS` as the chosen entries have it: all on, all off, or
/// mixed (none).
pub fn common_bits(modes: &[u32]) -> [Option<bool>; 9] {
    BITS.map(|(bit, _)| {
        let on = modes.iter().filter(|mode| *mode & bit != 0).count();
        match on {
            0 => Some(false),
            on if on == modes.len() => Some(true),
            _ => None,
        }
    })
}

/// The bits of an Attr column text: `rwxr-x---`.
pub fn mode_of(attributes: &str) -> Option<u32> {
    let chars: Vec<char> = attributes.chars().collect();
    if chars.len() != 9 {
        return None;
    }
    let mut mode = 0;
    for (c, (bit, letter)) in chars.iter().zip(BITS) {
        match *c {
            '-' => {}
            c if c == letter => mode |= bit,
            _ => return None,
        }
    }
    Some(mode)
}

/// `2026-11-10 14:04:33` in local time; seconds may be left out.
pub fn parse_time(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(text, format).ok())?;
    let local = chrono::Local.from_local_datetime(&naive).earliest()?;
    Some(local.into())
}

/// The change on `paths`, in the queue: errors are counted and named, not fatal.
pub fn task(fs: Arc<dyn Fs>, paths: Vec<PathBuf>, change: Change) -> Task {
    let title = match paths.as_slice() {
        [one] => format!(
            "Change attributes of \"{}\"",
            one.file_name().unwrap_or_default().display()
        ),
        many => format!("Change attributes of {} items", many.len()),
    };
    Task {
        title,
        run: Box::new(move |report| {
            let mut failed = Vec::new();
            for path in &paths {
                apply(&*fs, path, &change, report, &mut failed)?;
            }
            // Only failures are worth a word.
            Ok(match failed.is_empty() {
                true => String::new(),
                false => summary(report.progress.files, &failed),
            })
        }),
    }
}

fn apply(
    fs: &dyn Fs,
    path: &Path,
    change: &Change,
    report: &mut Report,
    failed: &mut Vec<String>,
) -> io::Result<()> {
    report.check()?;
    let meta = match fs.metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            failed.push(format!("{}: {err}", path.display()));
            return Ok(());
        }
    };
    // Links get nothing: their targets are elsewhere.
    if !meta.is_link {
        let wanted = Meta {
            modified: change.modified,
            mode: meta
                .mode
                .filter(|_| change.changes_mode())
                .map(|mode| change.mode(mode)),
            ..meta.clone()
        };
        match fs.set_meta(path, &wanted) {
            Ok(()) => report.progress.files += 1,
            Err(err) => failed.push(format!("{}: {err}", path.display())),
        }
        report.progress.current = path.to_owned();
        report.show();
    }
    if change.recurse && meta.is_dir && !meta.is_link {
        let names = fs.list(path).unwrap_or_else(|err| {
            failed.push(format!("{}: {err}", path.display()));
            Vec::new()
        });
        for name in names {
            apply(fs, &path.join(name), change, report, failed)?;
        }
    }
    Ok(())
}

/// What a task did, naming the first few failures.
pub(crate) fn summary(done: u64, failed: &[String]) -> String {
    let mut text = format!("{done} changed");
    if !failed.is_empty() {
        text += &format!(", {} failed", failed.len());
        for line in failed.iter().take(5) {
            text += &format!("\n{line}");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Queue, Status};
    use crate::vfs::Local;

    #[test]
    fn finds_common_bits_and_reads_times() {
        let bits = common_bits(&[0o644, 0o600]);
        assert_eq!(bits[0..3], [Some(true), Some(true), Some(false)]);
        assert_eq!(bits[3], None, "group read: mixed");
        let change = Change {
            set: 0o100,
            clear: 0o004,
            ..Change::default()
        };
        assert_eq!(change.mode(0o644), 0o740);
        let time = parse_time("2026-11-10 14:04").unwrap();
        assert_eq!(
            crate::local_time(time, "%Y-%m-%d %H:%M:%S"),
            "2026-11-10 14:04:00"
        );
        assert!(parse_time("yesterday").is_none());
        assert_eq!(mode_of("rwxr-x---"), Some(0o750));
        assert_eq!(mode_of("rahs"), None, "Windows' letters");
    }

    #[cfg(unix)]
    #[test]
    fn changes_inside_folders() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pc-attributes-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a"), "a").unwrap();
        let time = parse_time("2020-01-02 03:04:05").unwrap();
        let change = Change {
            set: 0o001,
            clear: 0o004,
            modified: Some(time),
            recurse: true,
        };
        let queue = Queue::default();
        let id = queue.push_task(task(Arc::new(Local), vec![root.join("sub")], change));
        let job = loop {
            let job = queue.jobs().into_iter().find(|job| job.id == id).unwrap();
            if job.status.finished() {
                break job;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(job.status, Status::Done);
        assert_eq!((job.note, job.progress.files), (None, 2));
        let meta = std::fs::metadata(root.join("sub/a")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o007, 0o001);
        assert_eq!(meta.modified().unwrap(), time);
        let dir = std::fs::metadata(root.join("sub")).unwrap();
        assert_eq!(dir.modified().unwrap(), time, "folders get the time too");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
