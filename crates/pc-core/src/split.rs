//! Split File and Combine Files: parts `name.001`, `name.002`… and a
//! `name.crc` with the whole file's name, size and CRC32, as Total
//! Commander writes them, so either program combines the other's.

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use crate::ops::Task;

/// `100 MB`, `1.5 GB`, `650m`, `4095MB`, or bytes.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim().to_lowercase().replace(' ', "");
    let digits = text.find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',');
    let (number, unit) = text.split_at(digits.unwrap_or(text.len()));
    let number: f64 = number.replace(',', ".").parse().ok()?;
    let scale = match unit.trim_end_matches('b').trim_end_matches('i') {
        "" => 1.,
        "k" => 1024.,
        "m" => 1024. * 1024.,
        "g" => 1024. * 1024. * 1024.,
        _ => return None,
    };
    let bytes = (number * scale) as u64;
    (bytes > 0).then_some(bytes)
}

/// `file` in parts of `size` bytes into `dir`.
pub fn split(file: PathBuf, dir: PathBuf, size: u64) -> Task {
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Task {
        title: format!("Split \"{name}\""),
        run: Box::new(move |report| {
            let total = std::fs::metadata(&file)?.len();
            report.progress.total_bytes = total;
            report.progress.total_files = total.div_ceil(size).max(1);
            let mut source = File::open(&file)?;
            let mut crc = crc32fast::Hasher::new();
            let mut buf = vec![0; (1 << 20).min(size as usize)];
            let mut written = Vec::new();
            let result = (|| {
                for part in 1.. {
                    let path = dir.join(format!("{name}.{part:03}"));
                    let mut left = size;
                    let mut out: Option<File> = None;
                    while left > 0 {
                        report.check()?;
                        let want = buf.len().min(left as usize);
                        let read = source.read(&mut buf[..want])?;
                        if read == 0 {
                            break;
                        }
                        if out.is_none() {
                            written.push(path.clone());
                            report.progress.current = path.clone();
                            out = Some(File::create(&path)?);
                        }
                        out.as_mut().expect("created").write_all(&buf[..read])?;
                        crc.update(&buf[..read]);
                        left -= read as u64;
                        report.progress.bytes += read as u64;
                        report.show();
                    }
                    if out.is_none() {
                        break;
                    }
                    report.progress.files += 1;
                }
                let crc_file = dir.join(format!("{}.crc", stem(&name)));
                let text = format!(
                    "filename={name}\nsize={total}\ncrc32={:08X}\n",
                    crc.clone().finalize()
                );
                written.push(crc_file.clone());
                std::fs::write(crc_file, text)
            })();
            if result.is_err() {
                for path in written {
                    let _ = std::fs::remove_file(path);
                }
            }
            result.map(|()| String::new())
        }),
    }
}

/// `a` of `a.zip`, as Total Commander names the CRC file.
fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// What a part tells of the whole: the first part's path, and the CRC file if any.
pub struct Parts {
    pub first: PathBuf,
    pub crc: Option<PathBuf>,
    /// As the CRC file names it, else the parts' name.
    pub name: String,
}

/// From any part (`a.zip.002`) or the CRC file (`a.crc`).
pub fn parts_of(path: &Path) -> Option<Parts> {
    let dir = path.parent().unwrap_or(Path::new(""));
    let file = path.file_name()?.to_string_lossy().into_owned();
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("crc"))
    {
        let named = read_crc(path).ok().map(|crc| crc.0)?;
        return Some(Parts {
            first: dir.join(format!("{named}.001")),
            crc: Some(path.to_owned()),
            name: named,
        });
    }
    let (whole, number) = file.rsplit_once('.')?;
    if number.len() < 3 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let crc = dir.join(format!("{}.crc", stem(whole)));
    Some(Parts {
        first: dir.join(format!("{whole}.001")),
        crc: crc.exists().then_some(crc),
        name: whole.to_owned(),
    })
}

/// The CRC file's name, size and CRC32.
fn read_crc(path: &Path) -> io::Result<(String, Option<u64>, Option<u32>)> {
    let (mut name, mut size, mut crc) = (None, None, None);
    for line in io::BufReader::new(File::open(path)?).lines() {
        let line = line?;
        match line.trim_end().split_once('=') {
            Some(("filename", value)) => name = Some(value.to_owned()),
            Some(("size", value)) => size = value.parse().ok(),
            Some(("crc32", value)) => crc = u32::from_str_radix(value, 16).ok(),
            _ => {}
        }
    }
    let name = name.ok_or_else(|| io::Error::other("the CRC file names no file"))?;
    Ok((name, size, crc))
}

/// The parts, `.001` on while they follow, into `target`; checked against
/// the CRC file where there is one.
pub fn combine(parts: Parts, target: PathBuf) -> Task {
    Task {
        title: format!("Combine \"{}\"", parts.name),
        run: Box::new(move |report| {
            let dir = parts.first.parent().unwrap_or(Path::new(""));
            let paths: Vec<PathBuf> = (1..)
                .map(|n| dir.join(format!("{}.{n:03}", parts.name)))
                .take_while(|path| path.exists())
                .collect();
            if paths.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is missing", parts.first.display()),
                ));
            }
            report.progress.total_files = paths.len() as u64;
            report.progress.total_bytes = paths
                .iter()
                .filter_map(|path| std::fs::metadata(path).ok())
                .map(|meta| meta.len())
                .sum();
            let combined = (|| {
                let mut out = File::create(&target)?;
                let mut crc = crc32fast::Hasher::new();
                let mut buf = vec![0; 1 << 20];
                for path in &paths {
                    report.progress.current = path.clone();
                    let mut part = File::open(path)?;
                    loop {
                        report.check()?;
                        let read = part.read(&mut buf)?;
                        if read == 0 {
                            break;
                        }
                        out.write_all(&buf[..read])?;
                        crc.update(&buf[..read]);
                        report.progress.bytes += read as u64;
                        report.show();
                    }
                    report.progress.files += 1;
                }
                Ok::<_, io::Error>(crc.finalize())
            })();
            let found = match combined {
                Ok(found) => found,
                Err(err) => {
                    let _ = std::fs::remove_file(&target);
                    return Err(err);
                }
            };
            let Some(crc_file) = &parts.crc else {
                return Ok(format!(
                    "{} parts combined, no CRC file to check",
                    paths.len()
                ));
            };
            let (_, size, crc) = read_crc(crc_file)?;
            let length = std::fs::metadata(&target)?.len();
            if size.is_some_and(|size| size != length) || crc.is_some_and(|crc| crc != found) {
                return Err(io::Error::other(format!(
                    "{} does not match {}: a part is missing or damaged",
                    target.display(),
                    crc_file.display()
                )));
            }
            Ok(String::new())
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
    fn splits_and_combines() {
        assert_eq!(parse_size("100 MB"), Some(100 << 20));
        assert_eq!(parse_size("1,5G"), Some(3 << 29));
        assert_eq!(parse_size("2500"), Some(2500));
        assert_eq!(parse_size("ten"), None);
        let root = std::env::temp_dir().join(format!("pc-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("parts")).unwrap();
        let data: Vec<u8> = (0..2500u32).map(|n| (n * 7) as u8).collect();
        std::fs::write(root.join("a.bin"), &data).unwrap();
        let queue = Queue::default();
        let id = queue.push_task(split(root.join("a.bin"), root.join("parts"), 1000));
        assert_eq!(wait(&queue, id).status, Status::Done);
        let parts = root.join("parts");
        assert_eq!(
            std::fs::metadata(parts.join("a.bin.003")).unwrap().len(),
            500
        );
        assert!(!parts.join("a.bin.004").exists());
        let crc = std::fs::read_to_string(parts.join("a.crc")).unwrap();
        assert!(
            crc.starts_with("filename=a.bin\nsize=2500\ncrc32="),
            "{crc}"
        );

        let found = parts_of(&parts.join("a.bin.002")).unwrap();
        assert_eq!(
            (found.first.clone(), found.name.as_str()),
            (parts.join("a.bin.001"), "a.bin")
        );
        let id = queue.push_task(combine(found, root.join("b.bin")));
        assert_eq!(wait(&queue, id).status, Status::Done);
        assert_eq!(std::fs::read(root.join("b.bin")).unwrap(), data);

        std::fs::write(parts.join("a.bin.002"), [0; 1000]).unwrap();
        let found = parts_of(&parts.join("a.crc")).unwrap();
        let id = queue.push_task(combine(found, root.join("c.bin")));
        let job = wait(&queue, id);
        assert_eq!(job.status, Status::Failed);
        assert!(job.note.unwrap().contains("missing or damaged"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
