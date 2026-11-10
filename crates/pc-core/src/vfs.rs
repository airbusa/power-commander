//! What file operations read and write: the local disk now, servers later.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::SystemTime;

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
