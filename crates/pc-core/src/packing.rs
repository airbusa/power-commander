//! Pack (Alt+F5), Unpack Specific Files (Alt+F9) and Test Archives
//! (Alt+Shift+F9) through the bundled 7-Zip, in the queue with its
//! progress. Passwords go to 7-Zip's prompt, never on its command line.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use crate::archive::{self, seven_zip};
use crate::ops::{Report, Task};
use crate::space::{self, Space};
use crate::vfs::{self, Local};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarXz,
    TarBz2,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Zip,
        Format::SevenZip,
        Format::Tar,
        Format::TarGz,
        Format::TarXz,
        Format::TarBz2,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::SevenZip => "7-Zip",
            Format::Tar => "TAR",
            Format::TarGz => "TAR.GZ",
            Format::TarXz => "TAR.XZ",
            Format::TarBz2 => "TAR.BZ2",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZip => "7z",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarXz => "tar.xz",
            Format::TarBz2 => "tar.bz2",
        }
    }

    /// Only ZIP and 7-Zip keep a password.
    pub fn encrypts(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZip)
    }

    /// Compressed tars cannot be split into volumes nor added to.
    pub fn compressed_tar(self) -> bool {
        self.compressor().is_some()
    }

    /// The format a file name ends in.
    pub fn of(name: &str) -> Option<Format> {
        let name = name.to_lowercase();
        let ends = |ext: &str| name.ends_with(&format!(".{ext}"));
        Some(match () {
            _ if ends("tar.gz") || ends("tgz") => Format::TarGz,
            _ if ends("tar.xz") || ends("txz") => Format::TarXz,
            _ if ends("tar.bz2") || ends("tbz2") || ends("tbz") => Format::TarBz2,
            _ if ends("tar") => Format::Tar,
            _ if ends("7z") => Format::SevenZip,
            _ if ends("zip") || ends("jar") => Format::Zip,
            _ => return None,
        })
    }

    /// `name` with this format's extension in place of another archive's.
    pub fn rename(self, name: &str) -> String {
        format!("{}.{}", strip(name), self.extension())
    }

    fn compressor(self) -> Option<&'static str> {
        match self {
            Format::TarGz => Some("-tgzip"),
            Format::TarXz => Some("-txz"),
            Format::TarBz2 => Some("-tbzip2"),
            _ => None,
        }
    }

    fn type_switch(self) -> &'static str {
        match self {
            Format::Zip => "-tzip",
            Format::SevenZip => "-t7z",
            _ => "-ttar",
        }
    }
}

pub struct Pack {
    /// The archive; with `separate`, its folder and format are what count.
    pub archive: PathBuf,
    pub format: Format,
    /// 0 stores, 9 packs hardest.
    pub level: u8,
    /// Where the chosen entries are.
    pub folder: PathBuf,
    pub names: Vec<OsString>,
    pub password: Option<String>,
    pub volume_mb: Option<u64>,
    /// The packed entries are deleted once all is packed.
    pub move_files: bool,
    /// One archive for each entry, named after it.
    pub separate: bool,
}

pub fn pack(pack: Pack) -> Task {
    let what = match pack.names.as_slice() {
        [one] => format!("\"{}\"", one.display()),
        many => format!("{} items", many.len()),
    };
    let into = match pack.separate {
        true => format!("separate {} archives", pack.format.label()),
        false => pack
            .archive
            .file_name()
            .unwrap_or_default()
            .display()
            .to_string(),
    };
    Task {
        title: format!("Pack {what} into {into}"),
        run: Box::new(move |report| run_pack(&pack, report)),
    }
}

fn run_pack(pack: &Pack, report: &mut Report) -> io::Result<String> {
    let sizes: Vec<u64> = pack
        .names
        .iter()
        .map(|name| size_of(&pack.folder.join(name)))
        .collect();
    report.progress.total_bytes = sizes.iter().sum();
    report.progress.total_files = pack.names.len() as u64;
    let jobs: Vec<(PathBuf, Vec<&OsString>, u64)> = match pack.separate {
        true => {
            let dir = pack.archive.parent().unwrap_or(Path::new(""));
            let named = |name: &OsString| {
                let file = format!("{}.{}", name.display(), pack.format.extension());
                dir.join(file)
            };
            let each = pack.names.iter().zip(&sizes);
            each.map(|(name, size)| (named(name), vec![name], *size))
                .collect()
        }
        false => vec![(
            pack.archive.clone(),
            pack.names.iter().collect(),
            report.progress.total_bytes,
        )],
    };
    let mut base = 0;
    for (archive, names, size) in jobs {
        report.progress.current = archive.clone();
        let existed = archive.exists();
        if existed && pack.format.compressed_tar() {
            let name = archive.display();
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{name} exists already"),
            ));
        }
        let packed = pack_one(pack, &archive, &names, report, base, size);
        if packed.is_err() && !existed {
            remove_with_volumes(&archive);
        }
        packed?;
        base += size;
        report.progress.files += names.len() as u64;
    }
    if pack.move_files {
        for name in &pack.names {
            vfs::remove_all(&Local, &pack.folder.join(name))?;
        }
    }
    Ok(String::new())
}

fn pack_one(
    pack: &Pack,
    archive: &Path,
    names: &[&OsString],
    report: &mut Report,
    base: u64,
    size: u64,
) -> io::Result<()> {
    let level = format!("-mx={}", pack.level.min(9));
    if let Some(compressor) = pack.format.compressor() {
        // 7-Zip makes the tar on its stdout, and packs that.
        let mut tar = seven_zip(
            &["a", "-ttar", "-so", "-an", "-spd", "-bsp2", "-bso0"],
            None,
        )
        .args(names)
        .current_dir(&pack.folder)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
        let tarred = tar.stdout.take().expect("piped");
        let inner = archive.file_stem().unwrap_or_default().to_string_lossy();
        let mut packing = seven_zip(
            &[
                "a",
                compressor,
                &level,
                &format!("-si{inner}"),
                "-bsp0",
                "-bso0",
            ],
            Some(archive),
        )
        .stdin(tarred)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
        let progress = tar.stderr.take().expect("piped");
        let watched = watch(
            &mut tar,
            progress,
            None::<std::process::ChildStderr>,
            report,
            base,
            size,
        );
        let packed = finish(&mut packing);
        watched?;
        return packed;
    }
    let mut args = vec![
        "a",
        pack.format.type_switch(),
        &level,
        "-spd",
        "-bsp1",
        "-bso0",
    ];
    let volume = pack.volume_mb.map(|mb| format!("-v{mb}m"));
    args.extend(volume.as_deref());
    if pack.password.is_some() {
        args.push("-p");
        args.push(match pack.format {
            Format::SevenZip => "-mhe=on",
            _ => "-mem=AES256",
        });
    }
    let mut command = seven_zip(&args, Some(archive));
    command.args(names).current_dir(&pack.folder);
    // Asked twice: to enter and to verify.
    let answer = pack.password.as_ref().map(|pw| format!("{pw}\n{pw}\n"));
    run(command, answer, report, base, size)
}

pub struct Unpack {
    pub archives: Vec<PathBuf>,
    pub target: PathBuf,
    /// `*.txt *.md`; empty or `*` takes all.
    pub mask: String,
    pub with_paths: bool,
    pub overwrite: bool,
    /// Each archive into a folder named after it.
    pub separate: bool,
    pub password: Option<String>,
}

pub fn unpack(unpack: Unpack) -> Task {
    let what = match unpack.archives.as_slice() {
        [one] => format!("\"{}\"", one.file_name().unwrap_or_default().display()),
        many => format!("{} archives", many.len()),
    };
    Task {
        title: format!("Unpack {what} to {}", unpack.target.display()),
        run: Box::new(move |report| run_unpack(&unpack, report)),
    }
}

fn run_unpack(unpack: &Unpack, report: &mut Report) -> io::Result<String> {
    let sizes: Vec<u64> = unpack.archives.iter().map(|a| size_of(a)).collect();
    report.progress.total_bytes = sizes.iter().sum();
    report.progress.total_files = unpack.archives.len() as u64;
    let masks: Vec<&str> = unpack
        .mask
        .split([' ', ';'])
        .filter(|mask| !mask.is_empty() && !matches!(*mask, "*" | "*.*"))
        .collect();
    let mut base = 0;
    for (archive, size) in unpack.archives.iter().zip(sizes) {
        report.progress.current = archive.clone();
        let out = match unpack.separate {
            true => unpack.target.join(stem(archive)),
            false => unpack.target.clone(),
        };
        let out = format!("-o{}", out.display());
        let mut args = vec![
            if unpack.with_paths { "x" } else { "e" },
            if unpack.overwrite { "-aoa" } else { "-aos" },
            &out,
            "-bsp1",
            "-bso0",
        ];
        if !masks.is_empty() {
            args.push("-r");
        }
        let answer = unpack.password.as_ref().map(|pw| format!("{pw}\n"));
        if archive::is_compressed_tar(archive) {
            let mut unpacking = seven_zip(&["x", "-so", "-bsp2", "-bso0"], Some(archive))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(missing)?;
            let tarred = unpacking.stdout.take().expect("piped");
            args.extend(["-si", "-ttar"]);
            args.retain(|arg| *arg != "-bsp1");
            let mut taking = seven_zip(&args, None)
                .args(&masks)
                .stdin(tarred)
                .stderr(Stdio::piped())
                .spawn()
                .map_err(missing)?;
            let progress = unpacking.stderr.take().expect("piped");
            let watched = watch(
                &mut unpacking,
                progress,
                None::<std::process::ChildStderr>,
                report,
                base,
                size,
            );
            let taken = finish(&mut taking);
            watched?;
            taken?;
        } else {
            let mut command = seven_zip(&args, Some(archive));
            command.args(&masks);
            run(command, answer, report, base, size).map_err(|err| named(archive, err))?;
        }
        base += size;
        report.progress.files += 1;
    }
    Ok(String::new())
}

/// Tests each archive; a broken one fails the task, naming it.
pub fn test(archives: Vec<PathBuf>, password: Option<String>) -> Task {
    let what = match archives.as_slice() {
        [one] => format!("\"{}\"", one.file_name().unwrap_or_default().display()),
        many => format!("{} archives", many.len()),
    };
    Task {
        title: format!("Test {what}"),
        run: Box::new(move |report| {
            let sizes: Vec<u64> = archives.iter().map(|a| size_of(a)).collect();
            report.progress.total_bytes = sizes.iter().sum();
            report.progress.total_files = archives.len() as u64;
            let (mut base, mut broken) = (0, Vec::new());
            for (archive, size) in archives.iter().zip(sizes) {
                report.progress.current = archive.clone();
                let mut command = seven_zip(&["t", "-bsp1", "-bso0"], Some(archive));
                command.stdin(Stdio::piped());
                let answer = password.as_ref().map(|pw| format!("{pw}\n"));
                match run(command, answer, report, base, size) {
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => return Err(err),
                    Err(err) => broken.push(named(archive, err).to_string()),
                    Ok(()) => {}
                }
                base += size;
                report.progress.files += 1;
            }
            match broken.is_empty() {
                true => Ok(format!("{} archive(s) OK", archives.len())),
                false => Err(io::Error::other(format!(
                    "{} of {} broken\n{}",
                    broken.len(),
                    archives.len(),
                    broken.join("\n")
                ))),
            }
        }),
    }
}

fn named(archive: &Path, err: io::Error) -> io::Error {
    match err.kind() {
        io::ErrorKind::Interrupted => err,
        kind => {
            let name = archive.file_name().unwrap_or_default().display();
            io::Error::new(kind, format!("{name}: {err}"))
        }
    }
}

/// `name` without the archive extension it ends in: `a` of `a.tar.gz`.
fn strip(name: &str) -> &str {
    const KNOWN: [&str; 11] = [
        "tar.gz", "tar.xz", "tar.bz2", "tgz", "txz", "tbz2", "tbz", "tar", "7z", "zip", "jar",
    ];
    let lower = name.to_lowercase();
    let cut = KNOWN.iter().find(|ext| lower.ends_with(&format!(".{ext}")));
    cut.map_or(name, |ext| &name[..name.len() - ext.len() - 1])
}

/// The folder an archive unpacks into of its own: `a` of `a.tar.gz`, `b` of `b.rar`.
fn stem(archive: &Path) -> String {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    match strip(&name) {
        stripped if stripped.len() < name.len() => stripped.to_owned(),
        _ => Path::new(name.as_ref())
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

fn size_of(path: &Path) -> u64 {
    let mut space = Space::default();
    let _ = space::measure(
        &Local,
        path,
        &mut space,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    space.bytes
}

/// A new archive that failed goes, and its volumes with it.
fn remove_with_volumes(archive: &Path) {
    let _ = std::fs::remove_file(archive);
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let dir = archive.parent().unwrap_or(Path::new("."));
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        let volume = other
            .strip_prefix(&format!("{name}."))
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
        if volume {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Runs 7-Zip, its progress on stdout, answering its password prompt.
fn run(
    mut command: Command,
    answer: Option<String>,
    report: &mut Report,
    base: u64,
    size: u64,
) -> io::Result<()> {
    let mut child = command
        .stdin(if answer.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(missing)?;
    if let (Some(answer), Some(mut stdin)) = (answer, child.stdin.take()) {
        // A 7-Zip that does not ask closes its end; that is fine.
        let _ = stdin.write_all(answer.as_bytes());
    }
    let progress = child.stdout.take().expect("piped");
    let errors = child.stderr.take().expect("piped");
    watch(&mut child, progress, Some(errors), report, base, size)
}

/// Shows the percentages 7-Zip writes to `progress` as bytes of `size`
/// after `base`; cancelling kills it. Its error is its last stderr line.
fn watch(
    child: &mut Child,
    progress: impl Read + Send + 'static,
    errors: Option<impl Read + Send + 'static>,
    report: &mut Report,
    base: u64,
    size: u64,
) -> io::Result<()> {
    let (percents, seen) = mpsc::channel();
    let reading = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut progress = progress;
        let mut chunk = [0; 4096];
        while let Ok(read) = progress.read(&mut chunk) {
            if read == 0 {
                break;
            }
            all.extend_from_slice(&chunk[..read]);
            if let Some(percent) = last_percent(&chunk[..read]) {
                let _ = percents.send(percent);
            }
        }
        all
    });
    let errors = errors.map(|mut errors| {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = errors.read_to_end(&mut all);
            all
        })
    });
    loop {
        if report.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        match seen.recv_timeout(Duration::from_millis(100)) {
            Ok(percent) => {
                report.progress.bytes = base + size * percent / 100;
                report.show();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    let progress = reading.join().unwrap_or_default();
    let stderr = match errors {
        Some(errors) => errors.join().unwrap_or_default(),
        None => progress,
    };
    match status.success() {
        true => Ok(()),
        false => Err(io::Error::other(failure(status.code(), &stderr))),
    }
}

/// Waits for the second 7-Zip of a pipe.
fn finish(child: &mut Child) -> io::Result<()> {
    let mut stderr = Vec::new();
    if let Some(mut errors) = child.stderr.take() {
        let _ = errors.read_to_end(&mut stderr);
    }
    let status = child.wait()?;
    match status.success() {
        true => Ok(()),
        false => Err(io::Error::other(failure(status.code(), &stderr))),
    }
}

/// 7-Zip's last words, its progress left out; a password it waited for in vain named.
fn failure(code: Option<i32>, stderr: &[u8]) -> String {
    if code == Some(255) {
        return "it needs a password".into();
    }
    let text = String::from_utf8_lossy(stderr);
    let lines = text.split(['\n', '\r', '\u{8}']).map(str::trim);
    let words = lines.filter(|line| !line.is_empty() && last_percent(line.as_bytes()).is_none());
    let mut words: Vec<&str> = words.collect();
    // "ERROR: name" comes before what is wrong.
    words.retain(|line| !line.starts_with("ERRORS:") && *line != "ERROR:");
    match words.iter().rev().find(|line| !line.starts_with("ERROR: ")) {
        Some(line) if line.contains("Wrong password") => "wrong password".into(),
        Some(line) => (*line).to_owned(),
        None => format!("7-Zip failed ({})", code.unwrap_or(-1)),
    }
}

/// The last `NN%` 7-Zip wrote in `bytes`.
pub(crate) fn last_percent(bytes: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(bytes);
    text.split(['\u{8}', '\r', '\n'])
        .filter_map(|part| {
            let part = part.trim_start();
            let digits = part.find('%')?;
            let number = &part[..digits];
            (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
                .then(|| number.parse::<u64>().ok())?
        })
        .next_back()
        .map(|percent| percent.min(100))
}

fn missing(err: io::Error) -> io::Error {
    archive::missing(err)
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
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn names_formats() {
        assert_eq!(Format::of("a.TAR.gz"), Some(Format::TarGz));
        assert_eq!(Format::of("a.tgz"), Some(Format::TarGz));
        assert_eq!(Format::of("a.txt"), None);
        assert_eq!(Format::SevenZip.rename("photos.tar.gz"), "photos.7z");
        assert_eq!(Format::TarXz.rename("notes"), "notes.tar.xz");
        assert_eq!(stem(Path::new("/x/a.b.tar.gz")), "a.b");
        assert_eq!(
            last_percent(b"  5%\x08\x08\x08\x08 42% 3 + a.txt"),
            Some(42)
        );
        assert_eq!(last_percent(b"Everything is Ok"), None);
    }

    #[test]
    fn packs_tests_and_unpacks() {
        let root = std::env::temp_dir().join(format!("pc-packing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/sub")).unwrap();
        std::fs::write(root.join("src/a.txt"), "alpha").unwrap();
        std::fs::write(root.join("src/sub/b.md"), "beta").unwrap();
        let queue = Queue::default();
        for (format, password) in [
            (Format::Zip, None),
            (Format::SevenZip, Some("s3cret".to_string())),
            (Format::TarGz, None),
        ] {
            let archive = root.join(format!("out.{}", format.extension()));
            let id = queue.push_task(pack(Pack {
                archive: archive.clone(),
                format,
                level: 5,
                folder: root.join("src"),
                names: vec!["a.txt".into(), "sub".into()],
                password: password.clone(),
                volume_mb: None,
                move_files: false,
                separate: false,
            }));
            let job = wait(&queue, id);
            assert_eq!(job.status, Status::Done, "{format:?}: {:?}", job.note);
            let id = queue.push_task(test(vec![archive.clone()], password.clone()));
            let job = wait(&queue, id);
            assert_eq!(job.status, Status::Done, "{format:?}: {:?}", job.note);
            let target = root.join(format!("x-{}", format.extension()));
            let id = queue.push_task(unpack(Unpack {
                archives: vec![archive.clone()],
                target: target.clone(),
                mask: "*.md".into(),
                with_paths: true,
                overwrite: true,
                separate: false,
                password: password.clone(),
            }));
            let job = wait(&queue, id);
            assert_eq!(job.status, Status::Done, "{format:?}: {:?}", job.note);
            assert_eq!(
                std::fs::read_to_string(target.join("sub/b.md")).unwrap(),
                "beta"
            );
            assert!(
                !target.join("a.txt").exists(),
                "{format:?}: only the mask's"
            );
        }
        let id = queue.push_task(test(vec![root.join("out.7z")], Some("wrong".into())));
        let job = wait(&queue, id);
        assert_eq!(job.status, Status::Failed);
        assert!(job.note.unwrap().contains("wrong password"));
        let id = queue.push_task(test(vec![root.join("out.7z")], None));
        assert!(wait(&queue, id).note.unwrap().contains("needs a password"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
