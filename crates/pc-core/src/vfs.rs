//! What file operations read and write: the local disk and servers.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Entry;

/// A file being written is named so until it is whole.
pub const PART: &str = ".pc-part";
const BUFFER: usize = 1 << 20;

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub is_dir: bool,
    /// A link is never followed.
    pub is_link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, where the file system has them.
    pub mode: Option<u32>,
}

pub trait Fs: Send + Sync {
    /// Two values with the same id are one file system, where a move is a rename.
    fn id(&self) -> String;
    fn metadata(&self, path: &Path) -> io::Result<Meta>;
    fn list(&self, dir: &Path) -> io::Result<Vec<OsString>>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>>;
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>>;
    /// After writing: the source's time and permissions.
    fn set_meta(&self, path: &Path, meta: &Meta) -> io::Result<()>;
    fn mkdir(&self, path: &Path) -> io::Result<()>;
    /// A file, a link, or an empty folder.
    fn remove(&self, path: &Path, is_dir: bool) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Copies a link as a link, where the file system can.
    fn copy_link(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// A server's file system, as a panel lists it.
pub trait Remote: Fs {
    /// `sftp://user@host`: titles show it, and typing it again reconnects.
    fn url(&self) -> String;
    /// Where a fresh connection starts.
    fn home(&self) -> PathBuf;
    /// `..` first, except at the root.
    fn entries(&self, dir: &Path) -> io::Result<Vec<Entry>>;
}

/// Streams `source` to `target` under a part name, renamed when whole, so
/// a failed copy leaves the target as it was. `copied` hears of each
/// buffer; its error stops the copy.
pub fn copy_file(
    from: &dyn Fs,
    source: &Path,
    to: &dyn Fs,
    target: &Path,
    meta: &Meta,
    copied: &mut dyn FnMut(u64) -> io::Result<()>,
) -> io::Result<()> {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let part = target.with_file_name(format!("{name}{PART}"));
    let mut write = || -> io::Result<()> {
        let mut reader = from.open(source)?;
        let mut writer = to.create(&part)?;
        let mut buffer = vec![0; BUFFER];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            writer.write_all(&buffer[..read])?;
            copied(read as u64)?;
        }
        writer.flush()?;
        drop(writer);
        if to.metadata(target).is_ok() {
            to.remove(target, false)?;
        }
        to.rename(&part, target)?;
        let _ = to.set_meta(target, meta);
        Ok(())
    };
    let written = write();
    if written.is_err() {
        let _ = to.remove(&part, false);
    }
    written
}

/// Removes a folder and all in it, or a file.
pub fn remove_all(fs: &dyn Fs, path: &Path) -> io::Result<()> {
    let meta = fs.metadata(path)?;
    if meta.is_dir && !meta.is_link {
        for name in fs.list(path)? {
            remove_all(fs, &path.join(name))?;
        }
    }
    fs.remove(path, meta.is_dir && !meta.is_link)
}

/// Creates `path` and the folders above it that are missing.
pub fn create_all(fs: &dyn Fs, path: &Path) -> io::Result<()> {
    if fs.metadata(path).is_ok_and(|meta| meta.is_dir) {
        return Ok(());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_all(fs, parent)?;
    }
    fs.mkdir(path)
}

pub struct Local;

impl Fs for Local {
    fn id(&self) -> String {
        "local".into()
    }

    fn metadata(&self, path: &Path) -> io::Result<Meta> {
        let meta = fs::symlink_metadata(path)?;
        #[cfg(unix)]
        let mode = Some(std::os::unix::fs::PermissionsExt::mode(&meta.permissions()));
        #[cfg(not(unix))]
        let mode = None;
        Ok(Meta {
            is_dir: meta.is_dir(),
            is_link: meta.is_symlink(),
            size: meta.len(),
            modified: meta.modified().ok(),
            mode,
        })
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<OsString>> {
        fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect()
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(File::open(path)?))
    }

    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(File::create(path)?))
    }

    fn set_meta(&self, path: &Path, meta: &Meta) -> io::Result<()> {
        if let Some(modified) = meta.modified {
            File::options()
                .write(true)
                .open(path)?
                .set_modified(modified)?;
        }
        #[cfg(unix)]
        if let Some(mode) = meta.mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
        Ok(())
    }

    fn mkdir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn remove(&self, path: &Path, is_dir: bool) -> io::Result<()> {
        match is_dir {
            true => fs::remove_dir(path),
            false => fs::remove_file(path),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    #[cfg(unix)]
    fn copy_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    }
}
