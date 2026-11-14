//! Calculate Occupied Space (Ctrl+L): files, folders and bytes under the
//! chosen entries, links not followed.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::Fs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Space {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Folders that could not be listed.
    pub unreadable: u64,
}

/// Adds what is at `path` to `space`; `seen` gets the totals after each
/// folder. Setting `stop` gives up.
pub fn measure(
    fs: &dyn Fs,
    path: &Path,
    space: &mut Space,
    stop: &AtomicBool,
    seen: &mut dyn FnMut(&Space),
) -> io::Result<()> {
    if stop.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
    }
    let meta = fs.metadata(path)?;
    if !meta.is_dir || meta.is_link {
        space.files += 1;
        space.bytes += meta.size;
        return Ok(());
    }
    space.dirs += 1;
    let Ok(names) = fs.list(path) else {
        space.unreadable += 1;
        return Ok(());
    };
    for name in names {
        match measure(fs, &path.join(name), space, stop, seen) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => return Err(err),
            // Gone since listed, or unreadable: the rest still counts.
            Err(_) => space.unreadable += 1,
            Ok(()) => {}
        }
    }
    seen(space);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Local;

    #[test]
    fn adds_up_folders() {
        let root = std::env::temp_dir().join(format!("pc-space-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("one"), [0; 10]).unwrap();
        std::fs::write(root.join("a/two"), [0; 20]).unwrap();
        std::fs::write(root.join("a/b/three"), [0; 30]).unwrap();
        let mut space = Space::default();
        let mut seen = 0;
        let stop = AtomicBool::new(false);
        measure(&Local, &root, &mut space, &stop, &mut |_| seen += 1).unwrap();
        let expected = Space {
            files: 3,
            dirs: 3,
            bytes: 60,
            unreadable: 0,
        };
        assert_eq!(space, expected);
        assert_eq!(seen, 3, "once a folder");
        stop.store(true, Ordering::Relaxed);
        let stopped = measure(&Local, &root, &mut Space::default(), &stop, &mut |_| {});
        assert_eq!(stopped.unwrap_err().kind(), io::ErrorKind::Interrupted);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
