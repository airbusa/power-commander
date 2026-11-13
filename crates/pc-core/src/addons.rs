//! Add-ons: what the app works without, downloaded and updated apart from
//! it: the Craft tools (office documents), libmpv (video and sound) and
//! rclone (online storage). Our builds, a zip per platform on the project's
//! GitHub release `addons`, listed with their versions and SHA-256 in its
//! `addons.json`; installed in the user's data folder, where `bundled::path`
//! finds them.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The list of add-ons; `PC_ADDONS_MANIFEST` names another (a URL or a file), for testing.
const MANIFEST: &str = "https://github.com/airbusa/power-commander/releases/download/addons/addons.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Addon {
    Office,
    Media,
    Cloud,
}

impl Addon {
    pub const ALL: [Addon; 3] = [Addon::Office, Addon::Media, Addon::Cloud];

    /// Its folder, as in vendor/, and its name in the manifest.
    pub fn id(self) -> &'static str {
        match self {
            Addon::Office => "craft",
            Addon::Media => "mpv",
            Addon::Cloud => "rclone",
        }
    }

    /// The project's own name: other people's software, not ours.
    pub fn name(self) -> &'static str {
        match self {
            Addon::Office => "WordCraft, GridCraft, DeckCraft",
            Addon::Media => "mpv (libmpv)",
            Addon::Cloud => "rclone",
        }
    }

    /// What Power Commander uses it for.
    pub fn purpose(self) -> &'static str {
        match self {
            Addon::Office => "Office documents: the viewer shows Word, Excel and PowerPoint files as PDF.",
            Addon::Media => "Video and sound, and HEIC and AVIF photos: the viewer and Quick View show them.",
            Addon::Cloud => "Online storage: S3, Google Drive, OneDrive, Dropbox and other services.",
        }
    }

    /// The project's site: its name, and its address.
    pub fn website(self) -> (&'static str, &'static str) {
        match self {
            Addon::Office => ("getartcraft.com", "https://getartcraft.com/apps"),
            Addon::Media => ("mpv.io", "https://mpv.io"),
            Addon::Cloud => ("rclone.org", "https://rclone.org"),
        }
    }

    /// Its licence, as its own files say.
    pub fn licence(self) -> &'static str {
        match self {
            Addon::Office => "MIT or Apache 2.0",
            Addon::Media => "LGPL 2.1 or later, with FFmpeg's; libass ISC, dav1d BSD, HarfBuzz MIT",
            Addon::Cloud => "MIT",
        }
    }

    /// The files it is in; the first tells it is installed.
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Addon::Office if cfg!(windows) => &["wordcraft-cli.exe", "gridcraft-cli.exe", "deckcraft-cli.exe"],
            Addon::Office => &["wordcraft-cli", "gridcraft-cli", "deckcraft-cli"],
            Addon::Media if cfg!(windows) => &["libmpv-2.dll"],
            Addon::Media if cfg!(target_os = "macos") => &["libmpv.2.dylib"],
            Addon::Media => &["libmpv.so.2"],
            Addon::Cloud if cfg!(windows) => &["rclone.exe"],
            Addon::Cloud => &["rclone"],
        }
    }

    /// The add-on, else shipped or built from source, else none; the system's copy is not used.
    pub fn presence(self) -> Presence {
        if let Some(version) = installed_version(self) {
            return Presence::Ours(version);
        }
        let builtin = crate::bundled::builtin(self.id(), self.files()[0]);
        builtin.map_or(Presence::Missing, Presence::Elsewhere)
    }
}

/// The missing add-on that would show this file: libmpv a video, sound, or
/// HEIC or AVIF picture; the Craft tools an office document.
pub fn wanted_for(path: &Path) -> Option<Addon> {
    if crate::media::is_media(path) || crate::media::is_still(path) {
        return (!crate::media::available()).then_some(Addon::Media);
    }
    let office = crate::convert::is_office(path) && crate::pdf::available();
    (office && crate::convert::converter_for(path).is_none()).then_some(Addon::Office)
}

/// What comes with the app, shown beside the add-ons: the same facts, no download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    SevenZip,
    Pdfium,
}

impl Builtin {
    pub const ALL: [Builtin; 2] = [Builtin::SevenZip, Builtin::Pdfium];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::SevenZip => "7-Zip",
            Builtin::Pdfium => "PDFium",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Builtin::SevenZip => "Archives: packs and unpacks 7z, zip, rar, tar and others.",
            Builtin::Pdfium => "PDF: the viewer shows PDF files, and office documents once converted.",
        }
    }

    pub fn website(self) -> (&'static str, &'static str) {
        match self {
            Builtin::SevenZip => ("7-zip.org", "https://www.7-zip.org"),
            Builtin::Pdfium => ("pdfium.googlesource.com", "https://pdfium.googlesource.com/pdfium/"),
        }
    }

    pub fn licence(self) -> &'static str {
        match self {
            Builtin::SevenZip => "LGPL 2.1 or later, BSD, unRAR restriction",
            Builtin::Pdfium => "BSD 3-Clause, Apache 2.0",
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Builtin::SevenZip if cfg!(windows) => "7z.exe",
            Builtin::SevenZip => "7zz",
            Builtin::Pdfium if cfg!(windows) => "pdfium.dll",
            Builtin::Pdfium if cfg!(target_os = "macos") => "libpdfium.dylib",
            Builtin::Pdfium => "libpdfium.so",
        }
    }

    /// The version vendor/ fetches, which releases ship.
    pub fn version(self) -> &'static str {
        let script = match self {
            Builtin::SevenZip => include_str!("../../../vendor/7zip/fetch.sh"),
            Builtin::Pdfium => include_str!("../../../vendor/pdfium/fetch.sh"),
        };
        script.lines().find_map(|line| line.strip_prefix("VERSION=")).unwrap_or("?")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Presence {
    /// Installed as an add-on, this version.
    Ours(String),
    /// Shipped with the app or built from source.
    Elsewhere(PathBuf),
    Missing,
}

/// Where the add-ons are installed, each in a folder of its id.
pub fn folder() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("powercommander/addons"))
}

/// An installed add-on's file, for `bundled::path`.
pub(crate) fn file(dir: &str, name: &Path) -> Option<PathBuf> {
    let path = folder()?.join(dir).join(name);
    path.exists().then_some(path)
}

fn installed_version(addon: Addon) -> Option<String> {
    let dir = folder()?.join(addon.id());
    dir.join(addon.files()[0]).exists().then_some(())?;
    let version = std::fs::read_to_string(dir.join("version.txt")).ok()?;
    Some(version.trim().to_owned())
}

/// This build's platform, as the manifest and vendor/ name it: "macos-aarch64".
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Manifest {
    pub addons: Vec<Release>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub id: String,
    pub version: String,
    /// By platform.
    pub downloads: BTreeMap<String, Download>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Download {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl Manifest {
    /// The add-on's current release, when there is one for this platform.
    pub fn offer(&self, addon: Addon) -> Option<(&Release, &Download)> {
        let release = self.addons.iter().find(|release| release.id == addon.id())?;
        Some((release, release.downloads.get(&platform())?))
    }

    /// Installed add-ons whose release is another version.
    pub fn updates(&self) -> Vec<(Addon, String)> {
        Addon::ALL
            .into_iter()
            .filter_map(|addon| {
                let Presence::Ours(installed) = addon.presence() else {
                    return None;
                };
                let (release, _) = self.offer(addon)?;
                (release.version != installed).then(|| (addon, release.version.clone()))
            })
            .collect()
    }
}

/// The list of add-ons. Blocks.
pub fn fetch() -> io::Result<Manifest> {
    let source = std::env::var("PC_ADDONS_MANIFEST").unwrap_or_else(|_| MANIFEST.to_owned());
    let mut text = String::new();
    open(&source)?.0.read_to_string(&mut text)?;
    serde_json::from_str(&text).map_err(|err| io::Error::other(format!("the list of add-ons: {err}")))
}

/// A URL's body and its length if known; a `file://` URL or a path reads the file.
fn open(source: &str) -> io::Result<(Box<dyn Read + Send>, Option<u64>)> {
    if !source.starts_with("https://") {
        let path = Path::new(source.strip_prefix("file://").unwrap_or(source));
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        return Ok((Box::new(file), Some(size)));
    }
    let response = ureq::get(source).call().map_err(|err| io::Error::other(format!("{source}: {err}")))?;
    let size = response.body().content_length();
    Ok((Box::new(response.into_body().into_reader()), size))
}

/// Downloads, checks and installs `release`, over what is there; `progress`
/// gets the bytes so far. Blocks.
pub fn install(addon: Addon, release: &Release, download: &Download, progress: &dyn Fn(u64)) -> io::Result<()> {
    let root = folder().ok_or_else(|| io::Error::other("no data folder"))?;
    install_in(&root, addon, release, download, progress)
}

fn install_in(root: &Path, addon: Addon, release: &Release, download: &Download, progress: &dyn Fn(u64)) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let id = addon.id();
    let zip = root.join(format!(".{id}.zip"));
    let fresh = root.join(format!(".{id}.new"));
    let old = root.join(format!(".{id}.old"));
    let result = (|| {
        fetch_checked(download, &zip, progress)?;
        let _ = std::fs::remove_dir_all(&fresh);
        std::fs::create_dir_all(&fresh)?;
        unpack(&zip, &fresh)?;
        if !fresh.join(addon.files()[0]).exists() {
            return Err(io::Error::other(format!("the download has no {}", addon.files()[0])));
        }
        runnable(&fresh, addon)?;
        std::fs::write(fresh.join("version.txt"), &release.version)?;
        // The old one aside, the new one in its place: never half of each.
        let _ = std::fs::remove_dir_all(&old);
        let target = root.join(id);
        if target.exists() {
            std::fs::rename(&target, &old)?;
        }
        std::fs::rename(&fresh, &target)?;
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    })();
    let _ = std::fs::remove_file(&zip);
    let _ = std::fs::remove_dir_all(&fresh);
    result
}

/// Removes the installed add-on.
pub fn remove(addon: Addon) -> io::Result<()> {
    let root = folder().ok_or_else(|| io::Error::other("no data folder"))?;
    match std::fs::remove_dir_all(root.join(addon.id())) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// The download into `to`, its SHA-256 checked.
fn fetch_checked(download: &Download, to: &Path, progress: &dyn Fn(u64)) -> io::Result<()> {
    let (mut body, _) = open(&download.url)?;
    let mut file = std::fs::File::create(to)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; 256 * 1024];
    let mut done = 0;
    loop {
        let read = body.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
        done += read as u64;
        progress(done);
    }
    file.flush()?;
    let sum: String = digest.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    match sum.eq_ignore_ascii_case(&download.sha256) {
        true => Ok(()),
        false => Err(io::Error::other("the download is damaged: its checksum differs")),
    }
}

/// The zip's files into `dir`, by the bundled 7-Zip.
fn unpack(zip: &Path, dir: &Path) -> io::Result<()> {
    let out = format!("-o{}", dir.display());
    let done = crate::archive::seven_zip(&["x", "-y", &out], Some(zip)).output()?;
    match done.status.success() {
        true => Ok(()),
        false => Err(io::Error::other(format!("unpacking: {}", String::from_utf8_lossy(&done.stderr).trim()))),
    }
}

/// Its programs runnable, whatever the zip kept.
fn runnable(dir: &Path, addon: Addon) -> io::Result<()> {
    #[cfg(unix)]
    for name in addon.files() {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        if path.is_file() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, addon);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_manifest() {
        let text = r#"{"addons": [{"id": "rclone", "version": "1.75.2", "downloads": {"macos-aarch64": {"url": "https://example.com/rclone.zip", "sha256": "ab", "size": 9}}}]}"#;
        let manifest: Manifest = serde_json::from_str(text).unwrap();
        for builtin in Builtin::ALL {
            assert!(builtin.version().chars().all(|c| c.is_ascii_digit() || c == '.'), "{}: {}", builtin.name(), builtin.version());
        }
        let offer = manifest.offer(Addon::Cloud).map(|(release, download)| (release.version.as_str(), download.size));
        assert_eq!(offer, (platform() == "macos-aarch64").then_some(("1.75.2", 9)));
        assert!(manifest.offer(Addon::Media).is_none());
    }

    /// A zip of two programs, installed and then updated in place.
    #[cfg(unix)]
    #[test]
    fn installs_and_updates() {
        let dir = std::env::temp_dir().join(format!("pc-addon-install-{}", std::process::id()));
        let files = dir.join("files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("rclone"), "#!/bin/sh\n").unwrap();
        std::fs::write(files.join("COPYING"), "MIT").unwrap();
        let zip = dir.join("rclone.zip");
        let made = crate::archive::seven_zip(&["a", "-tzip"], Some(&zip)).arg(files.join("rclone")).arg(files.join("COPYING")).output();
        if !made.is_ok_and(|made| made.status.success()) {
            return eprintln!("no 7-Zip here: vendor/7zip/fetch.sh");
        }
        let body = std::fs::read(&zip).unwrap();
        let sha256: String = ring::digest::digest(&ring::digest::SHA256, &body).as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let download = Download { url: zip.display().to_string(), sha256, size: body.len() as u64 };
        let release = |version: &str| Release { id: "rclone".into(), version: version.into(), downloads: BTreeMap::new() };
        let root = dir.join("addons");
        let seen = std::cell::Cell::new(0);
        install_in(&root, Addon::Cloud, &release("1.0"), &download, &|bytes| seen.set(bytes)).unwrap();
        assert_eq!(seen.get(), body.len() as u64);
        install_in(&root, Addon::Cloud, &release("1.1"), &download, &|_| {}).unwrap();
        let installed = root.join("rclone");
        assert_eq!(std::fs::read_to_string(installed.join("version.txt")).unwrap(), "1.1");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(installed.join("rclone")).unwrap().permissions().mode() & 0o111, 0o111);
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, ["rclone"], "no download or old copy left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The published list, over the network; run by hand with `--ignored`.
    #[test]
    #[ignore]
    fn fetches_the_published_list() {
        let manifest = fetch().unwrap();
        for release in &manifest.addons {
            println!("{} {}: {:?}", release.id, release.version, release.downloads.keys().collect::<Vec<_>>());
        }
        assert!(!manifest.addons.is_empty());
    }

    #[test]
    fn checks_the_download() {
        let dir = std::env::temp_dir().join(format!("pc-addon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        std::fs::write(&source, b"add-on").unwrap();
        let wrong = "0".repeat(64);
        let sum: String = ring::digest::digest(&ring::digest::SHA256, b"add-on").as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let download = |sha256: &str| Download { url: source.display().to_string(), sha256: sha256.to_owned(), size: 6 };
        fetch_checked(&download(&sum), &dir.join("ok"), &|_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("ok")).unwrap(), b"add-on");
        assert!(fetch_checked(&download(&wrong), &dir.join("bad"), &|_| {}).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
