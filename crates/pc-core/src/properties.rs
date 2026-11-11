//! Properties (Alt+Enter): what the file system says of one entry.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use crate::vfs::Fs;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    File,
    Folder,
    /// Where it points, as written in the link.
    Link(String),
}

#[derive(Clone, Debug)]
pub struct Properties {
    pub kind: Kind,
    /// A link's target's, else its own.
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    /// Only for this computer's files.
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// A server's entry has what its listing tells; this computer's, more.
/// The system's own properties dialog for files of this computer: the
/// file manager's (Dolphin, Nautilus) by its D-Bus interface on Linux,
/// Finder's on macOS. Blocks until it is asked; elsewhere, unsupported.
pub fn show_in_system(paths: &[std::path::PathBuf]) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let uris: Vec<String> = paths.iter().map(|p| crate::clipboard::file_uri(p)).collect();
        let bus = zbus::blocking::Connection::session().map_err(io::Error::other)?;
        let name = "org.freedesktop.FileManager1";
        let path = "/org/freedesktop/FileManager1";
        let call = (uris, "");
        bus.call_method(Some(name), path, Some(name), "ShowItemProperties", &call)
            .map_err(io::Error::other)?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        // The paths come as arguments: no quoting inside the script.
        let script = [
            "on run argv",
            "tell application \"Finder\"",
            "activate",
            "repeat with p in argv",
            "open information window of (POSIX file (p as text) as alias)",
            "end repeat",
            "end tell",
            "end run",
        ];
        let mut osascript = std::process::Command::new("osascript");
        for line in script {
            osascript.arg("-e").arg(line);
        }
        let done = osascript.args(paths).output()?;
        if !done.status.success() {
            let why = String::from_utf8_lossy(&done.stderr).trim().to_owned();
            return Err(io::Error::other(why));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(io::Error::new(io::ErrorKind::Unsupported, "no system properties dialog here yet"))
}

pub fn of(fs: &dyn Fs, local: bool, path: &Path) -> io::Result<Properties> {
    if !local {
        let meta = fs.metadata(path)?;
        return Ok(Properties {
            kind: match (meta.is_link, meta.is_dir) {
                (true, _) => Kind::Link(String::new()),
                (false, true) => Kind::Folder,
                (false, false) => Kind::File,
            },
            size: meta.size,
            is_dir: meta.is_dir,
            modified: meta.modified,
            accessed: None,
            created: None,
            mode: meta.mode,
            owner: None,
            group: None,
        });
    }
    let own = std::fs::symlink_metadata(path)?;
    let kind = match own.is_symlink() {
        true => Kind::Link(std::fs::read_link(path)?.display().to_string()),
        false if own.is_dir() => Kind::Folder,
        false => Kind::File,
    };
    // A link shows what it points to, unless that is gone.
    let meta = std::fs::metadata(path).unwrap_or(own);
    #[cfg(unix)]
    let (mode, owner, group) = {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        (
            Some(meta.permissions().mode() & 0o7777),
            Some(name_of("/etc/passwd", meta.uid())),
            Some(name_of("/etc/group", meta.gid())),
        )
    };
    #[cfg(not(unix))]
    let (mode, owner, group) = (None, None, None);
    Ok(Properties {
        kind,
        size: meta.len(),
        is_dir: meta.is_dir(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        created: meta.created().ok(),
        mode,
        owner,
        group,
    })
}

/// The name of user or group `id` in `/etc/passwd` or `/etc/group`; its
/// number where it is not there (as from a directory service).
#[cfg(unix)]
fn name_of(file: &str, id: u32) -> String {
    let listed = std::fs::read_to_string(file).unwrap_or_default();
    listed
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let number = fields.nth(1)?.parse::<u32>().ok()?;
            (number == id).then(|| name.to_owned())
        })
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Local;

    #[test]
    fn describes_files_and_links() {
        let root = std::env::temp_dir().join(format!("pc-properties-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let file = of(&Local, true, &root.join("a.txt")).unwrap();
        assert_eq!((file.kind, file.size, file.is_dir), (Kind::File, 5, false));
        assert!(file.modified.is_some());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();
            let link = of(&Local, true, &root.join("link")).unwrap();
            assert_eq!((link.kind, link.size), (Kind::Link("a.txt".into()), 5));
            assert!(link.owner.is_some_and(|owner| !owner.is_empty()));
            assert_eq!(name_of("/etc/passwd", 0), "root");
        }
        let folder = of(&Local, false, &root).unwrap();
        assert_eq!(folder.kind, Kind::Folder);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
