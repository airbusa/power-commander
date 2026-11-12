//! Where a long video, a sound mix or a book was left, to go on from there
//! next time, as Plex does. Files are known by a fingerprint of their
//! contents, not by their path: renamed, moved or copied to another disk,
//! they keep their place.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings;

/// Videos and sound shorter than this, in seconds, start at the beginning, as in Plex.
pub const MEDIA_MIN: f64 = 600.;
/// Past this share of it, it was finished: next time from the start.
const FINISHED: f64 = 0.95;
/// Before this share, it was hardly begun.
const BEGUN: f64 = 0.01;
/// Places kept; past it the longest left go.
const KEPT: usize = 1000;
const FILE: &str = "positions.json";
/// Each end's bytes the fingerprint reads; a smaller file it reads whole.
const CHUNK: usize = 64 * 1024;

/// A file by its contents: its size and a hash of its first and last 64 KiB
/// (all of it when smaller), as OpenSubtitles' and Kodi's hash, so a 10 GB
/// movie takes no longer than a book.
pub fn fingerprint(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let mut chunk = vec![0; CHUNK];
    let mut add = |file: &mut File, hash: &mut ring::digest::Context| -> io::Result<()> {
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                return Ok(());
            }
            hash.update(&chunk[..read]);
            if read == CHUNK {
                return Ok(());
            }
        }
    };
    if size <= 2 * CHUNK as u64 {
        let mut whole = Vec::new();
        file.read_to_end(&mut whole)?;
        hash.update(&whole);
    } else {
        add(&mut file, &mut hash)?;
        file.seek(SeekFrom::Start(size - CHUNK as u64))?;
        add(&mut file, &mut hash)?;
    }
    let digest = hash.finish();
    let hex: String = digest.as_ref()[..16].iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{size:x}-{hex}"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Seconds into a video or sound; a share of a book, 0 to 1.
    pub at: f64,
    /// Seconds long; 1 for a book.
    pub length: f64,
    /// When it was left, seconds since 1970.
    pub left: u64,
    /// The file's name then, for whoever reads the file.
    pub name: String,
}

/// Every file's place, by fingerprint.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Positions(HashMap<String, Position>);

impl Positions {
    pub fn load() -> io::Result<Self> {
        settings::read_json(FILE)
    }

    pub fn save(&self) -> io::Result<()> {
        settings::write_json(FILE, self)
    }

    /// Where the file was left, if it was begun and not finished.
    pub fn at(&self, fingerprint: &str) -> Option<f64> {
        self.0.get(fingerprint).map(|position| position.at)
    }

    /// Left at `at` of `length`: kept, or forgotten when hardly begun or finished.
    pub fn leave(&mut self, fingerprint: &str, name: &str, at: f64, length: f64) {
        if length <= 0. || at < length * BEGUN || at > length * FINISHED {
            self.0.remove(fingerprint);
            return;
        }
        let left = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let position = Position { at, length, left, name: name.to_owned() };
        self.0.insert(fingerprint.to_owned(), position);
        if self.0.len() > KEPT {
            let mut by_age: Vec<(u64, String)> = self.0.iter().map(|(key, p)| (p.left, key.clone())).collect();
            by_age.sort();
            for (_, key) in by_age.into_iter().take(self.0.len() - KEPT) {
                self.0.remove(&key);
            }
        }
    }
}

/// Where `fingerprint` was left, from the file; none when unreadable too.
pub fn left_at(fingerprint: &str) -> Option<f64> {
    Positions::load().ok()?.at(fingerprint)
}

/// Records where `fingerprint` was left (see `Positions::leave`).
pub fn leave(fingerprint: &str, name: &str, at: f64, length: f64) -> io::Result<()> {
    let mut positions = Positions::load()?;
    positions.leave(fingerprint, name, at, length);
    positions.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_by_contents() {
        let dir = std::env::temp_dir().join(format!("pc-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("a.mkv"), &big).unwrap();
        std::fs::write(dir.join("renamed.mkv"), &big).unwrap();
        let mut other = big.clone();
        other[299_999] ^= 1;
        std::fs::write(dir.join("b.mkv"), &other).unwrap();
        std::fs::write(dir.join("small.epub"), b"tiny").unwrap();
        let print = |name: &str| fingerprint(&dir.join(name)).unwrap();
        assert_eq!(print("a.mkv"), print("renamed.mkv"));
        assert_ne!(print("a.mkv"), print("b.mkv"), "the last bytes count");
        assert!(print("a.mkv").starts_with("493e0-"), "the size first: {}", print("a.mkv"));
        assert!(print("small.epub").starts_with("4-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_what_was_begun_and_not_finished() {
        let mut positions = Positions::default();
        positions.leave("film", "Film.mkv", 1200., 6000.);
        assert_eq!(positions.at("film"), Some(1200.));
        positions.leave("film", "Film.mkv", 5900., 6000.);
        assert_eq!(positions.at("film"), None, "finished: from the start next time");
        positions.leave("book", "Book.epub", 0.005, 1.);
        assert_eq!(positions.at("book"), None, "hardly begun");
        positions.leave("book", "Book.epub", 0.4, 1.);
        assert_eq!(positions.at("book"), Some(0.4));
    }
}
