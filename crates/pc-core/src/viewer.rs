//! Text of any size: rows indexed in the background, only the shown ones read.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Longer lines go on in the next row.
pub const ROW_BYTES: usize = 1024;
/// The index keeps the start of every `STEP`th row.
const STEP: usize = 64;
const CHUNK: usize = 1 << 20;

pub struct TextFile {
    file: Mutex<File>,
    len: u64,
    index: Arc<Index>,
}

#[derive(Default)]
struct Index {
    progress: Mutex<Progress>,
    stop: AtomicBool,
}

#[derive(Default)]
struct Progress {
    /// Starts of rows 0, STEP, 2·STEP, …
    starts: Vec<u64>,
    rows: usize,
    scanned: u64,
    done: bool,
}

impl TextFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut scan = File::open(path)?;
        let index = Arc::new(Index::default());
        // Row 0 is there at once: the first screen does not wait for the index.
        index.progress.lock().unwrap().starts.push(0);
        let building = index.clone();
        std::thread::spawn(move || building.build(&mut scan, len));
        Ok(Self {
            file: Mutex::new(file),
            len,
            index,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Rows indexed so far, bytes scanned, and whether the index is complete.
    pub fn indexed(&self) -> (usize, u64, bool) {
        let progress = self.index.progress.lock().unwrap();
        (progress.rows, progress.scanned, progress.done)
    }

    /// The indexed rows of `range` as shown: tabs expanded, control characters as `·`.
    pub fn rows(&self, range: Range<usize>) -> Vec<String> {
        let (start, first) = {
            let progress = self.index.progress.lock().unwrap();
            let block = range.start / STEP;
            match progress.starts.get(block) {
                Some(&start) => (start, block * STEP),
                None => return Vec::new(),
            }
        };
        let wanted = range.end - first;
        let size = (wanted * (ROW_BYTES + 1)).min((self.len - start) as usize);
        let mut buf = vec![0; size];
        let read = read_at(&mut self.file.lock().unwrap(), start, &mut buf).unwrap_or(0);
        let eof = start + read as u64 >= self.len;
        let mut rows = Vec::new();
        let (mut at, mut row) = (0, first);
        while row < range.end && at < read {
            let Some(len) = row_len(&buf[at..read], eof) else {
                break;
            };
            if row >= range.start {
                rows.push(display(&buf[at..at + len]));
            }
            at += len;
            row += 1;
        }
        rows
    }
}

impl Drop for TextFile {
    fn drop(&mut self) {
        self.index.stop.store(true, Ordering::Relaxed);
    }
}

impl Index {
    fn build(&self, file: &mut File, len: u64) {
        let mut buf = vec![0; CHUNK];
        let (mut pos, mut rows) = (0u64, 0usize);
        while pos < len && !self.stop.load(Ordering::Relaxed) {
            let read = match read_at(file, pos, &mut buf) {
                Ok(read) if read > 0 => read,
                _ => break,
            };
            let eof = pos + read as u64 >= len;
            let mut starts = Vec::new();
            let mut at = 0;
            while at < read {
                let Some(len) = row_len(&buf[at..read], eof) else {
                    break;
                };
                at += len;
                rows += 1;
                if rows.is_multiple_of(STEP) {
                    starts.push(pos + at as u64);
                }
            }
            pos += at as u64;
            let mut progress = self.progress.lock().unwrap();
            progress.starts.extend(starts);
            progress.rows = rows;
            progress.scanned = pos;
        }
        self.progress.lock().unwrap().done = true;
    }
}

/// The row at the start of `bytes`: through its newline, or `ROW_BYTES` cut
/// at a character's start. None while more of it may follow.
fn row_len(bytes: &[u8], eof: bool) -> Option<usize> {
    let window = &bytes[..bytes.len().min(ROW_BYTES + 1)];
    if let Some(newline) = memchr::memchr(b'\n', window) {
        return Some(newline + 1);
    }
    if bytes.len() > ROW_BYTES {
        let mut end = ROW_BYTES;
        // UTF-8 continuation bytes: a character goes whole to the next row.
        while end > ROW_BYTES - 3 && bytes[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        return Some(end);
    }
    (eof && !bytes.is_empty()).then_some(bytes.len())
}

fn display(row: &[u8]) -> String {
    let text = String::from_utf8_lossy(row);
    let mut shown = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.trim_end_matches(['\n', '\r']).chars() {
        match c {
            '\t' => {
                let spaces = 8 - column % 8;
                shown.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            c if c.is_control() => {
                shown.push('·');
                column += 1;
            }
            c => {
                shown.push(c);
                column += 1;
            }
        }
    }
    shown
}

fn read_at(file: &mut File, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
    file.seek(SeekFrom::Start(pos))?;
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_rows() {
        assert_eq!(row_len(b"ab\r\ncd", false), Some(4));
        assert_eq!(row_len(b"cd", false), None, "more may follow");
        assert_eq!(row_len(b"cd", true), Some(2));
        let long = "é".repeat(ROW_BYTES);
        let cut = row_len(long.as_bytes(), false).unwrap();
        assert!(cut <= ROW_BYTES && long.is_char_boundary(cut));
        assert_eq!(display(b"a\tb\x01\r\n"), "a       b\u{b7}");
    }

    #[test]
    fn reads_rows_of_a_file_bigger_than_a_chunk() {
        let path = std::env::temp_dir().join(format!("pc-viewer-test-{}", std::process::id()));
        let mut text = String::new();
        for i in 0..100_000 {
            text += &format!("line {i}\n");
        }
        text += &"x".repeat(ROW_BYTES + 10);
        std::fs::write(&path, &text).unwrap();
        assert!(text.len() > CHUNK);

        let file = TextFile::open(&path).unwrap();
        while !file.indexed().2 {
            std::thread::yield_now();
        }
        assert_eq!(file.indexed().0, 100_002, "the long last line is two rows");
        assert_eq!(file.rows(0..2), ["line 0", "line 1"]);
        assert_eq!(
            file.rows(99_998..100_001)[..2],
            ["line 99998", "line 99999"]
        );
        assert_eq!(file.rows(100_001..100_005), ["x".repeat(10)]);
        std::fs::remove_file(&path).unwrap();
    }
}
