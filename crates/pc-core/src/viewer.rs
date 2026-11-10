//! Files of any size as text or a hex dump: rows indexed in the background, only the shown ones read.

use std::borrow::Cow;
use std::fmt::Write;
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
/// Bytes in a row of the hex dump.
pub const HEX_ROW: usize = 16;

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
        let index = Arc::new(Index::default());
        // Row 0 is there at once: the first screen does not wait for the index.
        index.progress.lock().unwrap().starts.push(0);
        let mut scan = File::open(path)?;
        let building = index.clone();
        std::thread::spawn(move || building.build(&mut scan, len));
        Ok(Self {
            file: Mutex::new(file),
            len,
            index,
        })
    }

    pub fn hex_row_count(&self) -> usize {
        self.len.div_ceil(HEX_ROW as u64) as usize
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

impl TextFile {
    pub fn hex_rows(&self, range: Range<usize>) -> Vec<String> {
        let start = (range.start * HEX_ROW) as u64;
        if start >= self.len {
            return Vec::new();
        }
        let size = (range.len() * HEX_ROW).min((self.len - start) as usize);
        let mut buf = vec![0; size];
        let read = read_at(&mut self.file.lock().unwrap(), start, &mut buf).unwrap_or(0);
        buf[..read]
            .chunks(HEX_ROW)
            .enumerate()
            .map(|(i, bytes)| hex_row(start + (i * HEX_ROW) as u64, bytes))
            .collect()
    }
}

/// `00000010  0D 0A 1A 0A 00 00 00 0D  49 48 44 52 00 00 04 B0  ........IHDR....`
fn hex_row(offset: u64, bytes: &[u8]) -> String {
    let mut row = format!("{offset:08X} ");
    for i in 0..HEX_ROW {
        if i == HEX_ROW / 2 {
            row.push(' ');
        }
        match bytes.get(i) {
            Some(byte) => write!(row, " {byte:02X}").unwrap(),
            None => row.push_str("   "),
        }
    }
    row.push_str("  ");
    row.extend(bytes.iter().map(|&byte| match byte {
        0x20..=0x7E => byte as char,
        _ => '.',
    }));
    row
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

/// UTF-8 if valid, else Windows-1252 like TC's ANSI: binary as UTF-8 draws slowly.
fn display(row: &[u8]) -> String {
    let text = match std::str::from_utf8(row) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(row.iter().map(|&byte| ansi(byte)).collect()),
    };
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

/// Windows-1252's 0x80–0x9F; `·` where it has none.
const CP1252: [char; 32] = [
    '€', '·', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '·', 'Ž', '·', '·', '‘', '’',
    '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '·', 'ž', 'Ÿ',
];

fn ansi(byte: u8) -> char {
    match byte {
        0x80..=0x9F => CP1252[byte as usize - 0x80],
        _ => byte as char,
    }
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
        assert_eq!(display("é€".as_bytes()), "é€", "UTF-8");
        assert_eq!(
            display(b"\x89PNG\x80\xe9\xff"),
            "‰PNG€éÿ",
            "not UTF-8: ANSI"
        );
    }

    #[test]
    fn shows_binary_as_hex() {
        let path = std::env::temp_dir().join(format!("pc-hex-test-{}", std::process::id()));
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend([0xFF; 3]);
        std::fs::write(&path, &bytes).unwrap();
        let file = TextFile::open(&path).unwrap();
        assert_eq!(file.hex_row_count(), 2);
        assert_eq!(
            file.hex_rows(0..5),
            [
                "00000000  89 50 4E 47 0D 0A 1A 0A  00 00 00 0D 49 48 44 52  .PNG........IHDR",
                "00000010  FF FF FF                                          ...",
            ]
        );
        std::fs::remove_file(&path).unwrap();
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
