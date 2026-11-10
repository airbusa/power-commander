//! The cursor's file for viewers and editors: a server's or an archive's
//! as a temporary local copy, and how an editor's saves go back.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::archive::Member;
use crate::vfs::{self, Fs, Local, Remote};

/// After each save: the copy goes back where it came from, and says so.
pub type Upload = Arc<dyn Fn(&Path) -> io::Result<String> + Send + Sync>;

pub enum Origin {
    Local(PathBuf),
    Server(Arc<dyn Fs>, PathBuf),
    /// With the server and path the archive was taken from.
    Archive(Member, Option<(Arc<dyn Remote>, PathBuf)>),
}

impl Origin {
    /// For messages.
    pub fn name(&self) -> String {
        match self {
            Origin::Local(path) | Origin::Server(_, path) => path.display().to_string(),
            Origin::Archive(member, _) => member.listed.clone(),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Origin::Local(_))
    }

    /// A file to view: a copy of a server's or an archive's. Blocks.
    pub fn view(&self) -> io::Result<PathBuf> {
        match self {
            Origin::Local(path) => Ok(path.clone()),
            Origin::Server(fs, path) => vfs::fetch(&**fs, path),
            Origin::Archive(member, _) => member.extract(),
        }
    }

    /// A file to edit, and for a copy how saves go back. Blocks.
    pub fn edit(self) -> io::Result<(PathBuf, Option<Upload>)> {
        match self {
            Origin::Local(path) => Ok((path, None)),
            Origin::Server(fs, path) => {
                let local = vfs::fetch(&*fs, &path)?;
                Ok((local, Some(upload_to(fs, path))))
            }
            Origin::Archive(member, origin) => {
                if !member.updatable() {
                    let why = "only files in 7z, zip, jar and tar archives can be edited";
                    return Err(io::Error::new(io::ErrorKind::Unsupported, why));
                }
                let (dir, file) = member.extract_with_folders()?;
                let upload: Upload = Arc::new(move |_| {
                    member.update(&dir)?;
                    let archive = member.archive.file_name().unwrap_or_default();
                    let archive = archive.display().to_string();
                    if let Some((remote, path)) = &origin {
                        vfs::upload(&member.archive, &**remote, path)?;
                        return Ok(format!("Updated {archive} on {}", remote.url()));
                    }
                    Ok(format!("Updated {archive}"))
                });
                Ok((file, Some(upload)))
            }
        }
    }
}

/// A server's file.
fn upload_to(fs: Arc<dyn Fs>, path: PathBuf) -> Upload {
    Arc::new(move |local| {
        let named =
            |err: io::Error| io::Error::new(err.kind(), format!("{}: {err}", path.display()));
        vfs::upload(local, &*fs, &path).map_err(named)?;
        Ok(format!("Uploaded {}", path.display()))
    })
}

/// When the file was last changed, to tell after an editor whether it saved.
pub fn modified(file: &Path) -> io::Result<Option<SystemTime>> {
    Ok(Local.metadata(file)?.modified)
}

/// Once an editor on `file` has ended: the copy goes back if it changed;
/// none when it did not.
pub fn send_back(
    file: &Path,
    before: Option<SystemTime>,
    upload: &Upload,
) -> Option<io::Result<String>> {
    let changed = modified(file).is_ok_and(|now| now != before);
    changed.then(|| upload(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_back_only_changes() {
        let dir = std::env::temp_dir().join(format!("pc-origin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "one").unwrap();
        let before = modified(&file).unwrap();
        let upload: Upload = Arc::new(|_| Ok("sent".into()));
        assert!(send_back(&file, before, &upload).is_none(), "unchanged");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(send_back(&file, before, &upload).unwrap().unwrap(), "sent");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
