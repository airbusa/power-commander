//! Files of any size as Lister shows them: rows indexed in the background, only the shown ones read.

use std::fmt::Write;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::codepage;

/// Unwrapped lines are cut into rows of this many characters.
pub const LONG_ROW: usize = 1024;
/// The index keeps the start of every `STEP`th row.
const STEP: usize = 64;
const CHUNK: usize = 1 << 20;
/// Bytes in a row of the hex dump.
pub const HEX_ROW: usize = 16;

/// How bytes become characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoding {
    /// UTF-8 where a row is valid UTF-8, else the code page.
    Auto(u16),
    Utf8,
    Utf16 {
        big_endian: bool,
    },
    Page(u16),
}

/// How a file is cut into rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Lines, longer ones cut every `width` characters.
    Lines {
        width: usize,
    },
    /// `width` bytes a row, control bytes as `.`: Lister's Binary.
    Fixed {
        width: usize,
    },
    Hex,
}

/// What Lister's Autodetect finds at a file's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detected {
    /// No byte order mark nor UTF-8 beyond ASCII: a code page's text.
    Plain,
    Utf8,
    Utf16 {
        big_endian: bool,
    },
}

pub fn detect(path: &Path) -> io::Result<Detected> {
    let mut head = vec![0; 64 << 10];
    let read = read_at(&mut Data::File(File::open(path)?), 0, &mut head)?;
    let head = &head[..read];
    Ok(match head {
        [0xFF, 0xFE, ..] => Detected::Utf16 { big_endian: false },
        [0xFE, 0xFF, ..] => Detected::Utf16 { big_endian: true },
        [0xEF, 0xBB, 0xBF, ..] => Detected::Utf8,
        _ if !head.is_ascii() && is_utf8(head, read == 64 << 10) => Detected::Utf8,
        _ => Detected::Plain,
    })
}

/// Valid UTF-8, but for a character cut off where a sample ends.
fn is_utf8(bytes: &[u8], sample: bool) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(err) => sample && err.error_len().is_none(),
    }
}

pub struct TextFile {
    data: Mutex<Data>,
    len: u64,
    layout: Layout,
    decoding: Decoding,
    index: Arc<Index>,
}

enum Data {
    File(File),
    Text(Arc<[u8]>),
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
    pub fn open(path: &Path, layout: Layout, decoding: Decoding) -> io::Result<Self> {
        let file = File::open(path)?;
        let scan = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self::new(
            Data::File(file),
            Data::File(scan),
            len,
            layout,
            decoding,
        ))
    }

    /// Text made from a file, as HTML's.
    pub fn text(text: String, layout: Layout) -> Self {
        let bytes: Arc<[u8]> = text.into_bytes().into();
        let len = bytes.len() as u64;
        let scan = Data::Text(bytes.clone());
        Self::new(Data::Text(bytes), scan, len, layout, Decoding::Utf8)
    }

    fn new(data: Data, mut scan: Data, len: u64, layout: Layout, decoding: Decoding) -> Self {
        let index = Arc::new(Index::default());
        {
            let mut progress = index.progress.lock().unwrap();
            // Row 0 is there at once: the first screen does not wait for the index.
            progress.starts.push(0);
            let fixed = match layout {
                Layout::Lines { .. } => None,
                Layout::Fixed { width } => Some(width),
                Layout::Hex => Some(HEX_ROW),
            };
            if let Some(width) = fixed {
                progress.rows = len.div_ceil(width as u64) as usize;
                progress.scanned = len;
                progress.done = true;
            }
        }
        if let Layout::Lines { width } = layout {
            let building = index.clone();
            std::thread::spawn(move || building.build(&mut scan, len, width, decoding));
        }
        Self {
            data: Mutex::new(data),
            len,
            layout,
            decoding,
            index,
        }
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

    /// The rows of `range` as shown: tabs expanded, control characters as `·`.
    pub fn rows(&self, range: Range<usize>) -> Vec<String> {
        match self.layout {
            Layout::Lines { width } => self.lines(range, width),
            Layout::Fixed { width } => self.fixed(range, width, |_, bytes| {
                let page = match self.decoding {
                    Decoding::Auto(page) | Decoding::Page(page) => page,
                    _ => 1252,
                };
                codepage::decode(page, bytes)
                    .chars()
                    .map(|c| if c.is_control() { '.' } else { c })
                    .collect()
            }),
            Layout::Hex => self.fixed(range, HEX_ROW, hex_row),
        }
    }

    fn lines(&self, range: Range<usize>, width: usize) -> Vec<String> {
        let (start, first) = {
            let progress = self.index.progress.lock().unwrap();
            let block = range.start / STEP;
            match progress.starts.get(block) {
                Some(&start) => (start, block * STEP),
                None => return Vec::new(),
            }
        };
        // A character takes 4 bytes at most, and a row ends in a line end.
        let wanted = range.end - first;
        let size = (wanted * (width * 4 + 2)).min((self.len - start) as usize);
        let mut buf = vec![0; size];
        let read = read_at(&mut self.data.lock().unwrap(), start, &mut buf).unwrap_or(0);
        let eof = start + read as u64 >= self.len;
        let mut rows = Vec::new();
        let (mut at, mut row) = (0, first);
        while row < range.end && at < read {
            let Some(len) = row_len(&buf[at..read], eof, width, self.decoding) else {
                break;
            };
            if row >= range.start {
                rows.push(display(
                    &buf[at..at + len],
                    self.decoding,
                    start + at as u64 == 0,
                ));
            }
            at += len;
            row += 1;
        }
        rows
    }

    fn fixed(
        &self,
        range: Range<usize>,
        width: usize,
        show: impl Fn(u64, &[u8]) -> String,
    ) -> Vec<String> {
        let start = (range.start * width) as u64;
        if start >= self.len {
            return Vec::new();
        }
        let size = (range.len() * width).min((self.len - start) as usize);
        let mut buf = vec![0; size];
        let read = read_at(&mut self.data.lock().unwrap(), start, &mut buf).unwrap_or(0);
        buf[..read]
            .chunks(width)
            .enumerate()
            .map(|(i, bytes)| show(start + (i * width) as u64, bytes))
            .collect()
    }
}

impl Drop for TextFile {
    fn drop(&mut self) {
        self.index.stop.store(true, Ordering::Relaxed);
    }
}

impl Index {
    fn build(&self, data: &mut Data, len: u64, width: usize, decoding: Decoding) {
        let mut buf = vec![0; CHUNK];
        let (mut pos, mut rows) = (0u64, 0usize);
        while pos < len && !self.stop.load(Ordering::Relaxed) {
            let read = match read_at(data, pos, &mut buf) {
                Ok(read) if read > 0 => read,
                _ => break,
            };
            let eof = pos + read as u64 >= len;
            let mut starts = Vec::new();
            let mut at = 0;
            while at < read {
                let Some(len) = row_len(&buf[at..read], eof, width, decoding) else {
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

/// The row at the start of `bytes`: through its line end, or `width`
/// characters. None while more of it may follow.
fn row_len(bytes: &[u8], eof: bool, width: usize, decoding: Decoding) -> Option<usize> {
    if let Decoding::Utf16 { big_endian } = decoding {
        let unit = |i: usize| match big_endian {
            true => u16::from_be_bytes([bytes[2 * i], bytes[2 * i + 1]]),
            false => u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]),
        };
        for i in 0..(bytes.len() / 2).min(width + 2) {
            let c = unit(i);
            if c == u16::from(b'\n') {
                return Some(2 * i + 2);
            }
            // A full row's "\r\n" still ends it.
            if i > width || i == width && c != u16::from(b'\r') {
                let space = (0..width).rev().find(|&j| unit(j) == u16::from(b' '));
                return Some(2 * space.map_or(width, |j| j + 1));
            }
        }
        return (eof && !bytes.is_empty()).then_some(bytes.len());
    }
    let utf8 = matches!(decoding, Decoding::Auto(_) | Decoding::Utf8);
    let reach = if utf8 { width * 4 } else { width } + 2;
    let window = &bytes[..bytes.len().min(reach)];
    let line_end = memchr::memchr(b'\n', window);
    let line = &window[..line_end.unwrap_or(window.len())];
    let line = match line_end {
        Some(_) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    };
    // Where the character after the row's last starts, in a line too long.
    let cut = match line.len() > width {
        false => None,
        true if utf8 => line
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte & 0xC0 != 0x80)
            .nth(width)
            .map(|(at, _)| at),
        true => Some(width),
    };
    // Lister wraps after the row's last space, as word processors do.
    let cut = cut.map(|cut| memchr::memrchr(b' ', &line[..cut]).map_or(cut, |space| space + 1));
    match (cut, line_end) {
        (Some(cut), _) => Some(cut),
        (None, Some(end)) => Some(end + 1),
        (None, None) => (eof && !bytes.is_empty()).then_some(window.len()),
    }
}

fn display(row: &[u8], decoding: Decoding, first: bool) -> String {
    let text = match decoding {
        Decoding::Auto(page) => match std::str::from_utf8(row) {
            Ok(text) => text.to_owned(),
            // Binary as UTF-8 would be rare characters, each slow to draw.
            Err(_) => codepage::decode(page, row),
        },
        Decoding::Utf8 => String::from_utf8_lossy(row).into_owned(),
        Decoding::Utf16 { big_endian } => {
            let units = row.as_chunks::<2>().0.iter().map(|&pair| match big_endian {
                true => u16::from_be_bytes(pair),
                false => u16::from_le_bytes(pair),
            });
            char::decode_utf16(units)
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        Decoding::Page(page) => codepage::decode(page, row),
    };
    let text = match first {
        true => text.trim_start_matches('\u{feff}'),
        false => &text,
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

fn read_at(data: &mut Data, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
    let file = match data {
        Data::File(file) => file,
        Data::Text(text) => {
            let start = (pos as usize).min(text.len());
            let read = buf.len().min(text.len() - start);
            buf[..read].copy_from_slice(&text[start..start + read]);
            return Ok(read);
        }
    };
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

    const AUTO: Decoding = Decoding::Auto(1252);

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pc-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn all_rows(file: &TextFile) -> Vec<String> {
        while !file.indexed().2 {
            std::thread::yield_now();
        }
        file.rows(0..file.indexed().0)
    }

    #[test]
    fn splits_rows() {
        assert_eq!(row_len(b"ab\r\ncd", false, LONG_ROW, AUTO), Some(4));
        assert_eq!(
            row_len(b"cd", false, LONG_ROW, AUTO),
            None,
            "more may follow"
        );
        assert_eq!(row_len(b"cd", true, LONG_ROW, AUTO), Some(2));
        assert_eq!(
            row_len("éééé\n".as_bytes(), false, 3, AUTO),
            Some(6),
            "3 characters"
        );
        assert_eq!(
            row_len(b"\xe9\xe9\xe9\xe9\n", false, 3, Decoding::Page(1252)),
            Some(3)
        );
        assert_eq!(
            row_len(
                b"a\0b\0\n\0c\0",
                false,
                9,
                Decoding::Utf16 { big_endian: false }
            ),
            Some(6)
        );
        assert_eq!(display(b"a\tb\x01\r\n", AUTO, false), "a       b\u{b7}");
        assert_eq!(display("é€".as_bytes(), AUTO, false), "é€", "UTF-8");
        assert_eq!(
            display(b"\x89PNG\x80\xe9\xff", AUTO, false),
            "\u{2030}PNG€éÿ",
            "else ANSI"
        );
    }

    #[test]
    fn shows_files_as_lister_does() {
        // Lister's test files in the Windows VM.
        let long = format!(
            "short line\r\n\tTab indented\r\n{}\r\nafter long\r\n",
            "0123456789".repeat(300)
        );
        let path = temp("long", long.as_bytes());
        let wrapped = all_rows(&TextFile::open(&path, Layout::Lines { width: 81 }, AUTO).unwrap());
        assert_eq!(wrapped[1], "        Tab indented");
        assert_eq!(wrapped[2].len(), 81);
        assert_eq!(wrapped.len(), 2 + 3000usize.div_ceil(81) + 1);
        let binary = TextFile::open(&path, Layout::Fixed { width: 75 }, AUTO).unwrap();
        let first = &binary.rows(0..1)[0];
        assert!(
            first.starts_with("short line...Tab indented..0123"),
            "{first}"
        );
        assert_eq!(first.len(), 75);
        let hex = TextFile::open(&path, Layout::Hex, AUTO).unwrap();
        assert!(hex.rows(0..1)[0].starts_with("00000000  73 68 6F 72"));
        std::fs::remove_file(&path).unwrap();

        let words: Vec<_> = (0..200).map(|i| format!("word{i}")).collect();
        let full = "x".repeat(81);
        let path = temp(
            "words",
            format!("{}\r\n{full}\r\nshort", words.join(" ")).as_bytes(),
        );
        for decoding in [AUTO, Decoding::Page(1252)] {
            let wrapped =
                all_rows(&TextFile::open(&path, Layout::Lines { width: 81 }, decoding).unwrap());
            assert_eq!(wrapped[0], format!("{} ", words[..13].join(" ")));
            assert_eq!(wrapped[1], format!("{} ", words[13..24].join(" ")));
            let after = wrapped.iter().position(|row| *row == full).unwrap();
            assert_eq!(wrapped[after + 1], "short");
        }
        let utf16: Vec<u8> = format!("{} \r\n", words[..20].join(" "))
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let utf16_path = temp("words16", &utf16);
        let wrapped = all_rows(
            &TextFile::open(
                &utf16_path,
                Layout::Lines { width: 81 },
                Decoding::Utf16 { big_endian: false },
            )
            .unwrap(),
        );
        assert_eq!(wrapped[0], format!("{} ", words[..13].join(" ")));
        std::fs::remove_file(&utf16_path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let path = temp("utf16", b"\xff\xfe\xc1\x00r\x00\r\x00\n\x00l\x00");
        assert_eq!(
            detect(&path).unwrap(),
            Detected::Utf16 { big_endian: false }
        );
        let utf16 = TextFile::open(
            &path,
            Layout::Lines { width: 81 },
            Decoding::Utf16 { big_endian: false },
        );
        assert_eq!(all_rows(&utf16.unwrap()), ["Ár", "l"]);
        std::fs::remove_file(&path).unwrap();

        let path = temp("utf8", "Árvíztűrő\n".as_bytes());
        assert_eq!(detect(&path).unwrap(), Detected::Utf8);
        std::fs::remove_file(&path).unwrap();
        let path = temp("cp1250", b"\xc1rv\xedzt\xfbr\xf5\r\n");
        assert_eq!(detect(&path).unwrap(), Detected::Plain);
        let page = TextFile::open(&path, Layout::Lines { width: 81 }, Decoding::Page(1250));
        assert_eq!(all_rows(&page.unwrap()), ["Árvíztűrő"]);
        std::fs::remove_file(&path).unwrap();

        let html = TextFile::text("Heading\n\nFirst".into(), Layout::Lines { width: 81 });
        assert_eq!(all_rows(&html), ["Heading", "", "First"]);
    }

    #[test]
    fn reads_rows_of_a_file_bigger_than_a_chunk() {
        let mut text = String::new();
        for i in 0..100_000 {
            text += &format!("line {i}\n");
        }
        text += &"x".repeat(LONG_ROW + 10);
        assert!(text.len() > CHUNK);
        let path = temp("big", text.as_bytes());
        let file = TextFile::open(&path, Layout::Lines { width: LONG_ROW }, AUTO).unwrap();
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

    #[test]
    fn shows_binary_as_hex() {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend([0xFF; 3]);
        let path = temp("hex", &bytes);
        let file = TextFile::open(&path, Layout::Hex, AUTO).unwrap();
        assert_eq!(file.indexed(), (2, 19, true));
        assert_eq!(
            file.rows(0..5),
            [
                "00000000  89 50 4E 47 0D 0A 1A 0A  00 00 00 0D 49 48 44 52  .PNG........IHDR",
                "00000010  FF FF FF                                          ...",
            ]
        );
        std::fs::remove_file(&path).unwrap();
    }
}
