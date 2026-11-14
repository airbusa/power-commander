//! Create Checksum Files and Verify Checksums: SFV (CRC32), MD5, SHA1,
//! SHA256 and SHA512 files, as `md5sum` and Total Commander write them.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use md5::Digest;
use ring::digest;

use crate::ops::{Report, Task};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Crc32,
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Method {
    pub const ALL: [Method; 5] = [
        Method::Crc32,
        Method::Md5,
        Method::Sha1,
        Method::Sha256,
        Method::Sha512,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Method::Crc32 => "CRC32 (SFV)",
            Method::Md5 => "MD5",
            Method::Sha1 => "SHA1",
            Method::Sha256 => "SHA256",
            Method::Sha512 => "SHA512",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Method::Crc32 => "sfv",
            Method::Md5 => "md5",
            Method::Sha1 => "sha1",
            Method::Sha256 => "sha256",
            Method::Sha512 => "sha512",
        }
    }

    /// By a checksum file's extension.
    pub fn of(path: &Path) -> Option<Method> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Method::ALL
            .into_iter()
            .find(|method| method.extension() == ext)
    }

    /// By a checksum's length, for files of other names.
    fn of_length(hex: usize) -> Option<Method> {
        Some(match hex {
            8 => Method::Crc32,
            32 => Method::Md5,
            40 => Method::Sha1,
            64 => Method::Sha256,
            128 => Method::Sha512,
            _ => return None,
        })
    }

    fn hasher(self) -> Hasher {
        match self {
            Method::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
            Method::Md5 => Hasher::Md5(md5::Md5::new()),
            Method::Sha1 => Hasher::Ring(digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY)),
            Method::Sha256 => Hasher::Ring(digest::Context::new(&digest::SHA256)),
            Method::Sha512 => Hasher::Ring(digest::Context::new(&digest::SHA512)),
        }
    }

    /// A line of its file: SFV puts the name first.
    fn line(self, name: &str, hex: &str) -> String {
        match self {
            Method::Crc32 => format!("{name} {hex}\n"),
            _ => format!("{hex} *{name}\n"),
        }
    }
}

enum Hasher {
    Crc32(crc32fast::Hasher),
    Md5(md5::Md5),
    Ring(digest::Context),
}

impl Hasher {
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Crc32(hasher) => hasher.update(bytes),
            Hasher::Md5(hasher) => hasher.update(bytes),
            Hasher::Ring(context) => context.update(bytes),
        }
    }

    fn hex(self) -> String {
        match self {
            Hasher::Crc32(hasher) => format!("{:08X}", hasher.finalize()),
            Hasher::Md5(hasher) => hex(&hasher.finalize()),
            Hasher::Ring(context) => hex(context.finish().as_ref()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The checksum of a file, its bytes counted in `report`.
pub(crate) fn hash(path: &Path, method: Method, report: &mut Report) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = method.hasher();
    let mut buf = vec![0; 1 << 20];
    loop {
        report.check()?;
        let read = file.read(&mut buf)?;
        if read == 0 {
            return Ok(hasher.hex());
        }
        hasher.update(&buf[..read]);
        report.progress.bytes += read as u64;
        report.show();
    }
}

pub struct Create {
    pub folder: PathBuf,
    pub names: Vec<OsString>,
    pub method: Method,
    /// Where the one checksum file goes.
    pub file: PathBuf,
    /// A checksum file beside each file instead.
    pub separate: bool,
}

pub fn create(create: Create) -> Task {
    let title = match create.separate {
        true => format!("Create {} checksum files", create.method.label()),
        false => format!(
            "Create {}",
            create.file.file_name().unwrap_or_default().display()
        ),
    };
    Task {
        title,
        run: Box::new(move |report| {
            // Folders' files, by their path from `folder`.
            let mut files = Vec::new();
            for name in &create.names {
                gather(&create.folder, Path::new(name), &mut files)?;
            }
            report.progress.total_files = files.len() as u64;
            report.progress.total_bytes = files.iter().map(|(_, size)| size).sum();
            let mut lines = String::new();
            for (relative, _) in &files {
                let path = create.folder.join(relative);
                report.progress.current = path.clone();
                let hex = hash(&path, create.method, report)?;
                let name = relative.to_string_lossy().replace('\\', "/");
                match create.separate {
                    true => {
                        let own = path.file_name().unwrap_or_default().to_string_lossy();
                        let mut file = path.as_os_str().to_owned();
                        file.push(format!(".{}", create.method.extension()));
                        std::fs::write(file, create.method.line(&own, &hex))?;
                    }
                    false => lines += &create.method.line(&name, &hex),
                }
                report.progress.files += 1;
            }
            if !create.separate {
                File::create(&create.file)?.write_all(lines.as_bytes())?;
            }
            Ok(String::new())
        }),
    }
}

/// The files at `relative` under `folder`, with their sizes; links are not followed.
fn gather(folder: &Path, relative: &Path, files: &mut Vec<(PathBuf, u64)>) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(folder.join(relative))?;
    if meta.is_dir() {
        let mut names: Vec<OsString> = std::fs::read_dir(folder.join(relative))?
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        names.sort();
        for name in names {
            gather(folder, &relative.join(name), files)?;
        }
    } else if meta.is_file() {
        files.push((relative.to_owned(), meta.len()));
    }
    Ok(())
}

/// The checksums and names a checksum file lists; comments and blank lines left out.
fn parse(text: &str, method: Option<Method>) -> Vec<(String, String)> {
    let is_hex = |word: &str| !word.is_empty() && word.bytes().all(|b| b.is_ascii_hexdigit());
    text.lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty() && !line.starts_with(';') && !line.starts_with('#'))
        .filter_map(|line| {
            // SFV: `name CRC`; the others: `hash *name` or `hash  name`.
            if method == Some(Method::Crc32) {
                let (name, crc) = line.rsplit_once(' ')?;
                return is_hex(crc).then(|| (crc.to_owned(), name.trim().to_owned()));
            }
            let (hash, name) = line.split_once(' ')?;
            let name = name.strip_prefix(['*', ' ']).unwrap_or(name);
            is_hex(hash).then(|| (hash.to_owned(), name.to_owned()))
        })
        .collect()
}

/// Checks the files each checksum file lists, beside it; a wrong or
/// missing one fails the task, naming it.
pub fn verify(files: Vec<PathBuf>) -> Task {
    let title = match files.as_slice() {
        [one] => format!("Verify {}", one.file_name().unwrap_or_default().display()),
        many => format!("Verify {} checksum files", many.len()),
    };
    Task {
        title,
        run: Box::new(move |report| {
            let mut listed = Vec::new();
            for file in &files {
                let text = std::fs::read_to_string(file)?;
                let method = Method::of(file);
                let dir = file.parent().unwrap_or(Path::new(""));
                for (hash, name) in parse(&text, method) {
                    let found = method.or(Method::of_length(hash.len()));
                    listed.push((dir.join(&name), name, hash, found));
                }
            }
            report.progress.total_files = listed.len() as u64;
            report.progress.total_bytes = listed
                .iter()
                .filter_map(|(path, ..)| std::fs::metadata(path).ok())
                .map(|meta| meta.len())
                .sum();
            let (mut ok, mut wrong, mut missing) = (0, Vec::new(), Vec::new());
            for (path, name, expected, method) in listed {
                report.progress.current = path.clone();
                let Some(method) = method else {
                    wrong.push(name);
                    continue;
                };
                match hash(&path, method, report) {
                    Ok(found) if found.eq_ignore_ascii_case(&expected) => ok += 1,
                    Ok(_) => wrong.push(name),
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => return Err(err),
                    Err(_) => missing.push(name),
                }
                report.progress.files += 1;
            }
            if wrong.is_empty() && missing.is_empty() {
                return Ok(format!("all {ok} file(s) OK"));
            }
            let mut text = format!("{ok} OK, {} wrong, {} missing", wrong.len(), missing.len());
            for name in wrong.iter().take(10) {
                text += &format!("\nwrong: {name}");
            }
            for name in missing.iter().take(10) {
                text += &format!("\nmissing: {name}");
            }
            Err(io::Error::other(text))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Queue, Status};

    fn wait(queue: &Queue, id: u64) -> crate::ops::JobView {
        loop {
            let job = queue.jobs().into_iter().find(|job| job.id == id).unwrap();
            if job.status.finished() {
                return job;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn creates_and_verifies() {
        let root = std::env::temp_dir().join(format!("pc-checksum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "abc").unwrap();
        std::fs::write(root.join("sub/b.txt"), "").unwrap();
        let queue = Queue::default();
        let expected = [
            (Method::Crc32, "a.txt 352441C2\nsub/b.txt 00000000\n"),
            (
                Method::Md5,
                "900150983cd24fb0d6963f7d28e17f72 *a.txt\nd41d8cd98f00b204e9800998ecf8427e *sub/b.txt\n",
            ),
        ];
        for (method, text) in expected {
            let file = root.join(format!("all.{}", method.extension()));
            let id = queue.push_task(create(Create {
                folder: root.clone(),
                names: vec!["a.txt".into(), "sub".into()],
                method,
                file: file.clone(),
                separate: false,
            }));
            assert_eq!(wait(&queue, id).status, Status::Done);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
            let id = queue.push_task(verify(vec![file]));
            let job = wait(&queue, id);
            assert_eq!(job.note.as_deref(), Some("all 2 file(s) OK"), "{method:?}");
        }
        let id = queue.push_task(create(Create {
            folder: root.clone(),
            names: vec!["a.txt".into()],
            method: Method::Sha256,
            file: PathBuf::new(),
            separate: true,
        }));
        wait(&queue, id);
        let own = std::fs::read_to_string(root.join("a.txt.sha256")).unwrap();
        assert!(own.starts_with(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *a.txt"
        ));
        std::fs::write(root.join("a.txt"), "abd").unwrap();
        std::fs::remove_file(root.join("sub/b.txt")).unwrap();
        let id = queue.push_task(verify(vec![root.join("all.md5")]));
        let job = wait(&queue, id);
        assert_eq!(job.status, Status::Failed);
        assert_eq!(
            job.note.as_deref(),
            Some("0 OK, 1 wrong, 1 missing\nwrong: a.txt\nmissing: sub/b.txt")
        );
        assert_eq!(
            parse("hash  two spaces.txt\n", Some(Method::Md5)),
            [],
            "not hex"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
