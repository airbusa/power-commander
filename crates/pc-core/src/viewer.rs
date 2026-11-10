//! Files of any size as Lister shows them: rows indexed in the background, only the shown ones read.

use std::fmt::Write;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::settings::{StartFont, StartMode, ViewerSettings};
use crate::{codepage, html, settings};

/// Unwrapped lines are cut into rows of this many characters.
pub const LONG_ROW: usize = 1024;
/// The index keeps the start of every `STEP`th row.
const STEP: usize = 64;
const CHUNK: usize = 1 << 20;
/// Bytes in a row of the hex dump.
pub const HEX_ROW: usize = 16;
/// The most Copy puts on the clipboard.
pub const COPY_LIMIT: u64 = 64 << 20;
/// Find reads this much past each chunk, for matches across its end.
const OVERLAP: usize = 4 << 10;

/// What Lister's Find dialog asks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub text: String,
    pub whole_words: bool,
    pub case_sensitive: bool,
    pub backwards: bool,
    /// `text` is bytes as hex digits: `77 6F 72 64`.
    pub hex: bool,
    pub regex: bool,
}

/// Searches the Find dialog's list keeps.
const HISTORY: usize = 20;

/// The last Find and the texts searched before, kept between runs: F3
/// and F5 go on with it in any viewer.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Searches {
    pub query: Query,
    pub history: Vec<String>,
}

impl Searches {
    pub fn load() -> io::Result<Self> {
        settings::read_json("searches.json")
    }

    pub fn save(&self) -> io::Result<()> {
        settings::write_json("searches.json", self)
    }

    /// `query` becomes the last one, its text the history's first.
    pub fn add(&mut self, query: Query) {
        if !query.text.is_empty() {
            self.history.retain(|text| *text != query.text);
            self.history.insert(0, query.text.clone());
            self.history.truncate(HISTORY);
        }
        self.query = query;
    }
}

pub enum Pattern {
    Text(regex::bytes::Regex),
    Bytes(Vec<u8>),
}

impl Query {
    pub fn pattern(&self) -> Result<Pattern, String> {
        if self.text.is_empty() {
            return Err("Nothing to find.".into());
        }
        if self.hex {
            let digits: String = self.text.split_whitespace().collect();
            let bytes = match digits.len().is_multiple_of(2)
                && digits.chars().all(|c| c.is_ascii_hexdigit())
            {
                true => (0..digits.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap())
                    .collect(),
                false => return Err(format!("Not a hex string: {}", self.text)),
            };
            return Ok(Pattern::Bytes(bytes));
        }
        let text = match self.regex {
            true => self.text.clone(),
            false => regex::escape(&self.text),
        };
        let text = match self.whole_words {
            true => format!(r"\b(?:{text})\b"),
            false => text,
        };
        regex::bytes::RegexBuilder::new(&text)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .map(Pattern::Text)
            .map_err(|err| err.to_string())
    }
}

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

impl Decoding {
    /// The code page of Binary and Hex.
    fn page(self) -> u16 {
        match self {
            Decoding::Auto(page) | Decoding::Page(page) => page,
            Decoding::Utf8 | Decoding::Utf16 { .. } => 1252,
        }
    }
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

/// A row as shown, and the bytes each of its characters comes from.
#[derive(Debug)]
pub struct Row {
    pub text: String,
    /// One a character of `text`; empty for the hex dump's spacing.
    pub cells: Vec<Range<u64>>,
    /// Where the row's text ends, before its line end.
    pub end: u64,
}

impl Row {
    fn new(end: u64) -> Self {
        Self {
            text: String::new(),
            cells: Vec::new(),
            end,
        }
    }

    fn push(&mut self, c: char, cell: Range<u64>) {
        self.text.push(c);
        self.cells.push(cell);
    }

    /// Where a click on the `column`th character puts the cursor.
    pub fn offset(&self, column: usize) -> u64 {
        self.cells.get(column).map_or(self.end, |cell| cell.start)
    }

    /// The word around the `column`th character, as a double click selects it.
    pub fn word(&self, column: usize) -> Option<Range<u64>> {
        let chars: Vec<char> = self.text.chars().collect();
        let column = column.min(chars.len().checked_sub(1)?);
        let word = |at: &usize| chars[*at].is_alphanumeric() || chars[*at] == '_';
        if !word(&column) {
            return None;
        }
        let first = (0..column).rev().take_while(word).last().unwrap_or(column);
        let last = (column..chars.len()).take_while(word).last()?;
        Some(self.cells[first].start..self.cells[last].end)
    }

    /// Byte ranges of `text` whose characters are in `selection`.
    pub fn highlights(&self, selection: &Range<u64>) -> Vec<Range<usize>> {
        let mut runs: Vec<Range<usize>> = Vec::new();
        for ((at, c), cell) in self.text.char_indices().zip(&self.cells) {
            if cell.start < selection.end && selection.start < cell.end {
                let end = at + c.len_utf8();
                match runs.last_mut() {
                    Some(run) if run.end == at => run.end = end,
                    _ => runs.push(at..end),
                }
            }
        }
        runs
    }
}

#[cfg(test)]
impl PartialEq<&str> for Row {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[cfg(test)]
impl PartialEq<String> for Row {
    fn eq(&self, other: &String) -> bool {
        self.text == *other
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
    pub fn rows(&self, range: Range<usize>) -> Vec<Row> {
        match self.layout {
            Layout::Lines { width } => {
                self.lines(range, width, |bytes, at| display(bytes, self.decoding, at))
            }
            Layout::Fixed { width } => self.fixed(range, width, |start, bytes| {
                let mut row = Row::new(start + bytes.len() as u64);
                let mut at = start;
                for (c, len) in codepage::chars(self.decoding.page(), bytes) {
                    let len = len as u64;
                    row.push(if c.is_control() { '.' } else { c }, at..at + len);
                    at += len;
                }
                row
            }),
            Layout::Hex => self.fixed(range, HEX_ROW, hex_row),
        }
    }

    /// The rows' first `columns` characters, as a minimap draws them: far
    /// cheaper than `rows` for long ones.
    pub fn sketch(&self, range: Range<usize>, columns: usize) -> Vec<String> {
        let Layout::Lines { width } = self.layout else {
            let rows = self.rows(range).into_iter();
            return rows
                .map(|row| row.text.chars().take(columns).collect())
                .collect();
        };
        self.lines(range, width, |bytes, at| {
            let head = &bytes[..bytes.len().min(columns * 4)];
            let chars = chars(head, self.decoding).into_iter();
            let chars = chars.skip(usize::from(at == 0 && head.starts_with(b"\xef\xbb\xbf")));
            chars.take(columns).map(|(c, _)| c).collect()
        })
    }

    /// The bytes of `range`, as in the file.
    pub fn bytes(&self, range: Range<u64>) -> io::Result<Vec<u8>> {
        let end = range.end.min(self.len);
        let mut buf = vec![0; end.saturating_sub(range.start) as usize];
        let read = read_at(&mut self.data.lock().unwrap(), range.start, &mut buf)?;
        buf.truncate(read);
        Ok(buf)
    }

    /// `range` as Lister's Copy as text gives it: line ends kept, other
    /// control characters as `.`.
    pub fn text_of(&self, range: Range<u64>) -> io::Result<String> {
        let decoding = self.text_decoding();
        let bytes = self.bytes(range.clone())?;
        let chars = chars(&bytes, decoding).into_iter().map(|(c, _)| c);
        let text = chars.map(|c| match c {
            '\t' | '\r' | '\n' => c,
            c if c.is_control() => '.',
            c => c,
        });
        let text: String = text.collect();
        Ok(match range.start {
            0 => text.trim_start_matches('\u{feff}').to_owned(),
            _ => text,
        })
    }

    /// How Copy and Find read the text: Binary and Hex in the code page.
    fn text_decoding(&self) -> Decoding {
        match self.layout {
            Layout::Lines { .. } => self.decoding,
            Layout::Fixed { .. } | Layout::Hex => Decoding::Page(self.decoding.page()),
        }
    }

    /// The first match starting at or after `from`, or `backwards` the last
    /// one ending at or before it. Setting `stop` gives up.
    pub fn find(
        &self,
        pattern: &Pattern,
        from: u64,
        backwards: bool,
        stop: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        let decoding = self.text_decoding();
        let (step, end) = (CHUNK as u64, if backwards { from } else { self.len });
        let mut start = if backwards {
            from.saturating_sub(step)
        } else {
            from
        };
        let mut buf = vec![0; CHUNK + OVERLAP];
        while !stop.load(Ordering::Relaxed) {
            let window = end.saturating_sub(start).min((CHUNK + OVERLAP) as u64) as usize;
            let read = read_at(&mut self.data.lock().unwrap(), start, &mut buf[..window])?;
            // Matches starting in the overlap are the next chunk's, but at the end.
            let last = start + read as u64 >= end;
            let mut found = matches(pattern, &buf[..read], decoding)
                .into_iter()
                .filter(|m| last || m.start < CHUNK)
                .map(|m| start + m.start as u64..start + m.end as u64);
            let found = match backwards {
                true => found.next_back(),
                false => found.next(),
            };
            if found.is_some() {
                return Ok(found);
            }
            match backwards {
                true if start == 0 => break,
                true => start = start.saturating_sub(step),
                false if last => break,
                false => start += step,
            }
        }
        Ok(None)
    }

    /// The row `offset` is in; none while the index has not reached it.
    pub fn row_of(&self, offset: u64) -> Option<usize> {
        let width = match self.layout {
            Layout::Lines { width } => width,
            Layout::Fixed { width } => return Some((offset / width as u64) as usize),
            Layout::Hex => return Some((offset / HEX_ROW as u64) as usize),
        };
        let (start, first) = {
            let progress = self.index.progress.lock().unwrap();
            if !progress.done && progress.scanned <= offset {
                return None;
            }
            let block = progress
                .starts
                .partition_point(|&start| start <= offset)
                .max(1)
                - 1;
            (progress.starts[block], block * STEP)
        };
        let size = (STEP * (width * 4 + 2)).min((self.len - start) as usize);
        let mut buf = vec![0; size];
        let read = read_at(&mut self.data.lock().unwrap(), start, &mut buf).ok()?;
        let eof = start + read as u64 >= self.len;
        let (mut at, mut row) = (0, first);
        while let Some(len) = row_len(&buf[at..read], eof, width, self.decoding) {
            if start + (at + len) as u64 > offset {
                break;
            }
            at += len;
            row += 1;
        }
        Some(row)
    }

    /// `range` as Copy as Hex characters gives it: `77 6F 72 64`.
    pub fn hex_of(&self, range: Range<u64>) -> io::Result<String> {
        let bytes = self.bytes(range)?;
        let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
        Ok(hex.join(" "))
    }

    fn lines<T>(
        &self,
        range: Range<usize>,
        width: usize,
        show: impl Fn(&[u8], u64) -> T,
    ) -> Vec<T> {
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
                rows.push(show(&buf[at..at + len], start + at as u64));
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
        show: impl Fn(u64, &[u8]) -> Row,
    ) -> Vec<Row> {
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

/// The characters of `bytes`, each with how many bytes it takes.
fn chars(bytes: &[u8], decoding: Decoding) -> Vec<(char, usize)> {
    let utf8 = |text: &str| text.chars().map(|c| (c, c.len_utf8())).collect::<Vec<_>>();
    match decoding {
        Decoding::Auto(page) => match std::str::from_utf8(bytes) {
            Ok(text) => utf8(text),
            // Binary as UTF-8 would be rare characters, each slow to draw.
            Err(_) => codepage::chars(page, bytes),
        },
        Decoding::Utf8 => bytes
            .utf8_chunks()
            .flat_map(|chunk| {
                let mut chars = utf8(chunk.valid());
                if !chunk.invalid().is_empty() {
                    chars.push((char::REPLACEMENT_CHARACTER, chunk.invalid().len()));
                }
                chars
            })
            .collect(),
        Decoding::Utf16 { big_endian } => {
            let units = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| match big_endian {
                    true => u16::from_be_bytes(pair),
                    false => u16::from_le_bytes(pair),
                });
            char::decode_utf16(units)
                .map(|c| match c {
                    Ok(c) => (c, 2 * c.len_utf16()),
                    Err(_) => (char::REPLACEMENT_CHARACTER, 2),
                })
                .collect()
        }
        Decoding::Page(page) => codepage::chars(page, bytes),
    }
}

/// Where `pattern` matches in `bytes`, read as `decoding` says.
fn matches(pattern: &Pattern, bytes: &[u8], decoding: Decoding) -> Vec<Range<usize>> {
    let regex = match pattern {
        Pattern::Bytes(needle) => {
            let found = memchr::memmem::find_iter(bytes, needle);
            return found.map(|at| at..at + needle.len()).collect();
        }
        Pattern::Text(regex) => regex,
    };
    // A chunk may start inside a UTF-8 character.
    let cut = bytes
        .iter()
        .take(3)
        .take_while(|&&b| b & 0xC0 == 0x80)
        .count();
    let utf8 = match decoding {
        Decoding::Utf8 => true,
        Decoding::Auto(_) => is_utf8(&bytes[cut..], true),
        Decoding::Utf16 { .. } | Decoding::Page(_) => false,
    };
    if utf8 {
        return regex.find_iter(bytes).map(|m| m.range()).collect();
    }
    // Searched as UTF-8 text, each of its bytes knowing its character's offset.
    let mut text = String::with_capacity(bytes.len());
    let mut source = Vec::with_capacity(bytes.len() + 1);
    let mut at = 0;
    for (c, len) in chars(bytes, decoding) {
        text.push(c);
        source.extend(std::iter::repeat_n(at, c.len_utf8()));
        at += len;
    }
    source.push(at);
    let found = regex.find_iter(text.as_bytes());
    found.map(|m| source[m.start()]..source[m.end()]).collect()
}

/// Tabs expanded, control characters as `·`, the line end and BOM hidden.
fn display(bytes: &[u8], decoding: Decoding, start: u64) -> Row {
    let mut chars = chars(bytes, decoding);
    while let Some(('\n' | '\r', _)) = chars.last() {
        chars.pop();
    }
    let mut row = Row::new(start);
    let mut at = start;
    for (c, len) in chars {
        let cell = at..at + len as u64;
        at = cell.end;
        match c {
            '\u{feff}' if cell.start == 0 => {}
            '\t' => {
                let spaces = 8 - row.cells.len() % 8;
                (0..spaces).for_each(|_| row.push(' ', cell.clone()));
            }
            c if c.is_control() => row.push('·', cell),
            c => row.push(c, cell),
        }
    }
    row.end = at;
    row
}

/// `00000010  0D 0A 1A 0A 00 00 00 0D  49 48 44 52 00 00 04 B0  ........IHDR....`
fn hex_row(offset: u64, bytes: &[u8]) -> Row {
    let end = offset + bytes.len() as u64;
    let mut row = Row::new(end);
    let gap = |row: &mut Row, text: &str, at: u64| text.chars().for_each(|c| row.push(c, at..at));
    gap(&mut row, &format!("{offset:08X} "), offset);
    for i in 0..HEX_ROW {
        let at = offset + i as u64;
        if i == HEX_ROW / 2 {
            gap(&mut row, " ", at);
        }
        match bytes.get(i) {
            Some(byte) => {
                gap(&mut row, " ", at);
                format!("{byte:02X}")
                    .chars()
                    .for_each(|c| row.push(c, at..at + 1));
            }
            None => gap(&mut row, "   ", end),
        }
    }
    gap(&mut row, "  ", end);
    for (i, &byte) in bytes.iter().enumerate() {
        let at = offset + i as u64;
        let c = match byte {
            0x20..=0x7E => byte as char,
            _ => '.',
        };
        row.push(c, at..at + 1);
    }
    row
}

/// Google's search for `text`, as Lister's context menu opens it.
pub fn search_url(text: &str) -> String {
    let mut url = String::from("https://www.google.com/search?q=");
    for byte in text.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                url.push(byte as char)
            }
            b' ' => url.push('+'),
            _ => write!(url, "%{byte:02X}").unwrap(),
        }
    }
    url
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

/// Larger Markdown is shown as text: the renderer lays out all of it.
pub const MARKDOWN_LIMIT: u64 = 4 << 20;
/// Larger HTML is shown as plain text.
const HTML_LIMIT: u64 = 64 << 20;

/// Lister's views, by the keys 1 to 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    Binary,
    Hex,
    Picture,
    Html,
    Unicode,
    Utf8,
    /// Not Lister's: rendered, as the UI kit does.
    Markdown,
    /// Not Lister's: pages rendered by PDFium.
    Pdf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Ansi,
    Ascii,
    Variable,
}

/// Which file a viewer shows, and how.
pub struct Shown {
    pub files: Vec<std::path::PathBuf>,
    pub at: usize,
    pub settings: ViewerSettings,
    pub mode: Mode,
    pub font: Font,
    /// Chosen in the Encoding menu.
    pub page: Option<u16>,
    pub wrap: bool,
    /// UTF-16's byte order, as its BOM says.
    pub big_endian: bool,
    /// The viewer can show it as a picture.
    pub picture: bool,
    /// A PDF, and PDFium is there.
    pub pdf: bool,
    /// Extensions of the pictures the viewer can show.
    pictures: &'static [&'static str],
}

impl Shown {
    pub fn new(
        files: Vec<std::path::PathBuf>,
        at: usize,
        settings: ViewerSettings,
        pictures: &'static [&'static str],
    ) -> Self {
        let font = match settings.font {
            StartFont::Ascii => Font::Ascii,
            StartFont::Variable => Font::Variable,
            StartFont::Ansi | StartFont::Auto => Font::Ansi,
        };
        Self {
            files,
            at,
            wrap: settings.wrap,
            settings,
            mode: Mode::Text,
            font,
            page: None,
            big_endian: false,
            picture: false,
            pdf: false,
            pictures,
        }
    }

    pub fn path(&self) -> &Path {
        &self.files[self.at]
    }

    /// The mode the settings start with; Autodetect as Lister's.
    pub fn start(&mut self) -> io::Result<TextFile> {
        let path = self.path();
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        let extension = extension.as_deref().unwrap_or_default();
        let picture = self.pictures.contains(&extension);
        let html = matches!(extension, "htm" | "html" | "xhtml");
        let markdown = matches!(extension, "md" | "markdown")
            && std::fs::metadata(path)?.len() <= MARKDOWN_LIMIT;
        let detected = detect(path)?;
        self.picture = picture;
        self.pdf = extension == "pdf" && crate::pdf::available();
        self.big_endian = detected == Detected::Utf16 { big_endian: true };
        self.page = None;
        self.mode = match self.settings.start {
            StartMode::Text => Mode::Text,
            StartMode::Binary => Mode::Binary,
            StartMode::Hex => Mode::Hex,
            StartMode::Auto if picture => Mode::Picture,
            StartMode::Auto if self.pdf => Mode::Pdf,
            StartMode::Auto if html => Mode::Html,
            StartMode::Auto if markdown => Mode::Markdown,
            StartMode::Auto => match detected {
                Detected::Plain => Mode::Text,
                Detected::Utf8 => Mode::Utf8,
                Detected::Utf16 { .. } => Mode::Unicode,
            },
        };
        // NFO art is always in the IBM PC's code page.
        if matches!(extension, "nfo" | "diz") && self.mode == Mode::Text {
            self.page = Some(437);
        }
        self.text()
    }

    /// The chosen code page, else the font's.
    pub fn page(&self) -> u16 {
        let (ansi, dos) = codepage::system();
        self.page.unwrap_or(match self.font {
            Font::Ascii => dos,
            Font::Ansi | Font::Variable => ansi,
        })
    }

    pub fn text(&self) -> io::Result<TextFile> {
        let path = self.path();
        let page = self.page();
        let width = match self.wrap {
            true => self.settings.text_width.max(1),
            false => LONG_ROW,
        };
        let lines = Layout::Lines { width };
        let plain = match self.page {
            Some(page) => Decoding::Page(page),
            None => Decoding::Auto(page),
        };
        Ok(match self.mode {
            Mode::Binary => {
                let width = self.settings.binary_width.max(1);
                TextFile::open(path, Layout::Fixed { width }, Decoding::Page(page))?
            }
            Mode::Hex => TextFile::open(path, Layout::Hex, Decoding::Page(page))?,
            Mode::Html if std::fs::metadata(path)?.len() <= HTML_LIMIT => {
                let source = match String::from_utf8(std::fs::read(path)?) {
                    Ok(source) => source,
                    Err(err) => codepage::decode(page, err.as_bytes()),
                };
                TextFile::text(html::to_text(&source), lines)
            }
            Mode::Unicode => {
                let big_endian = self.big_endian;
                TextFile::open(path, lines, Decoding::Utf16 { big_endian })?
            }
            Mode::Utf8 | Mode::Markdown => TextFile::open(path, lines, Decoding::Utf8)?,
            // A PDF's text is what its pages say, not its bytes.
            Mode::Text if self.pdf => TextFile::text(crate::pdf::text(path)?, lines),
            Mode::Text | Mode::Picture | Mode::Html | Mode::Pdf => {
                TextFile::open(path, lines, plain)?
            }
        })
    }

    /// For the status bar.
    pub fn describe(&self) -> String {
        let page = self.page();
        match self.mode {
            Mode::Text => format!("Text · {page}"),
            Mode::Binary => format!("Binary · {page}"),
            Mode::Hex => format!("Hex · {page}"),
            Mode::Picture => "Image".into(),
            Mode::Html => "HTML text".into(),
            Mode::Unicode => "UTF-16".into(),
            Mode::Utf8 => "UTF-8".into(),
            Mode::Markdown => "Markdown".into(),
            Mode::Pdf => "PDF".into(),
        }
    }

    /// False when it is the mode already, or one this file cannot have.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if mode == self.mode
            || mode == Mode::Picture && !self.picture
            || mode == Mode::Pdf && !self.pdf
        {
            return false;
        }
        self.mode = mode;
        true
    }

    /// ANSI and ASCII show text in their code page, as Lister does.
    pub fn set_font(&mut self, font: Font, page: Option<u16>) {
        self.font = font;
        self.page = page;
        if !matches!(self.mode, Mode::Binary | Mode::Hex) {
            self.mode = Mode::Text;
        }
    }

    /// The next (or with -1 the previous) of `files`; false at either end.
    pub fn step(&mut self, by: isize) -> bool {
        let at = self.at.checked_add_signed(by);
        let Some(at) = at.filter(|&at| at < self.files.len()) else {
            return false;
        };
        self.at = at;
        true
    }

    /// Rows, not pictures or pages, which Find goes through.
    pub fn has_rows(&self) -> bool {
        !matches!(self.mode, Mode::Picture | Mode::Markdown | Mode::Pdf)
    }
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
        let rows = file.rows(0..file.indexed().0);
        rows.into_iter().map(|row| row.text).collect()
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
        assert_eq!(display(b"a\tb\x01\r\n", AUTO, 1), "a       b\u{b7}");
        assert_eq!(display("é€".as_bytes(), AUTO, 1), "é€", "UTF-8");
        assert_eq!(
            display(b"\x89PNG\x80\xe9\xff", AUTO, 1),
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
        let first = &binary.rows(0..1)[0].text;
        assert!(
            first.starts_with("short line...Tab indented..0123"),
            "{first}"
        );
        assert_eq!(first.len(), 75);
        let hex = TextFile::open(&path, Layout::Hex, AUTO).unwrap();
        assert!(hex.rows(0..1)[0].text.starts_with("00000000  73 68 6F 72"));
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
    fn selects_and_copies_as_lister_does() {
        let path = temp("select", "\u{feff}one tw\u{f6}\tx\r\nnext\x01".as_bytes());
        let file = TextFile::open(&path, Layout::Lines { width: 81 }, Decoding::Utf8).unwrap();
        while !file.indexed().2 {
            std::thread::yield_now();
        }
        let rows = file.rows(0..2);
        assert_eq!(rows[0].text, "one tw\u{f6} x");
        assert_eq!(
            (rows[0].offset(0), rows[0].offset(4), rows[0].end),
            (3, 7, 13)
        );
        assert_eq!(rows[0].word(5), Some(7..11), "tw\u{f6} is 4 bytes");
        assert_eq!(rows[0].word(3), None);
        assert_eq!(rows[0].word(usize::MAX), Some(12..13), "past the end: x");
        assert_eq!(
            rows[0].highlights(&(9..13)),
            vec![6..10],
            "ö, the tab's space and x"
        );
        assert_eq!(
            file.text_of(0..file.len()).unwrap(),
            "one tw\u{f6}\tx\r\nnext."
        );
        assert_eq!(file.hex_of(3..6).unwrap(), "6F 6E 65");
        assert_eq!(file.sketch(0..2, 3), ["one", "nex"]);
        let hex = TextFile::open(&path, Layout::Hex, AUTO).unwrap();
        let row = &hex.rows(0..1)[0];
        assert_eq!(row.highlights(&(1..2)), [13..15, 61..62]);
        assert_eq!(row.offset(0), 0);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            search_url(" a b&\u{e9} "),
            "https://www.google.com/search?q=a+b%26%C3%A9"
        );
    }

    #[test]
    fn finds_as_lister_does() {
        let query = |text: &str| Query {
            text: text.into(),
            ..Query::default()
        };
        let find = |file: &TextFile, query: &Query, from: u64, backwards: bool| {
            let pattern = query.pattern().unwrap();
            file.find(&pattern, from, backwards, &AtomicBool::new(false))
                .unwrap()
        };
        // Matches near each chunk's end, and one across it.
        let mut text = "x".repeat(CHUNK - 3);
        text += "needle";
        text += &"y".repeat(CHUNK);
        text += "Needle needles\n";
        let path = temp("find", text.as_bytes());
        let file = TextFile::open(&path, Layout::Lines { width: 81 }, AUTO).unwrap();
        let first = (CHUNK - 3) as u64..(CHUNK + 3) as u64;
        assert_eq!(find(&file, &query("needle"), 0, false), Some(first.clone()));
        let second = find(&file, &query("needle"), first.end, false).unwrap();
        assert_eq!(second.start, (2 * CHUNK + 3) as u64, "case-insensitive");
        let last = find(&file, &query("needle"), file.len(), true).unwrap();
        assert_eq!(last.start, second.start + 7);
        assert_eq!(
            find(&file, &query("needle"), second.start, true),
            Some(first.clone())
        );
        let exact = Query {
            case_sensitive: true,
            whole_words: true,
            ..query("needle")
        };
        assert_eq!(find(&file, &exact, first.end, false), None);
        let hex = Query {
            hex: true,
            ..query("4E 65")
        };
        assert_eq!(
            find(&file, &hex, 0, false).map(|m| m.start),
            Some(second.start)
        );
        while !file.indexed().2 {
            std::thread::yield_now();
        }
        assert_eq!(file.row_of(0), Some(0));
        assert_eq!(file.row_of(81), Some(1));
        assert_eq!(file.row_of(second.start), Some(file.indexed().0 - 1));
        std::fs::remove_file(&path).unwrap();

        let path = temp("find1250", b"line\r\n\xc1rv\xedzt\xfbr\xf5");
        let file =
            TextFile::open(&path, Layout::Lines { width: 81 }, Decoding::Page(1250)).unwrap();
        assert_eq!(find(&file, &query("ÍZTŰ"), 0, false), Some(9..13));
        let regex = Query {
            regex: true,
            ..query("^á")
        };
        assert_eq!(find(&file, &regex, 0, false), Some(6..7));
        std::fs::remove_file(&path).unwrap();

        let utf16: Vec<u8> = "\u{feff}a Für b"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let path = temp("find16", &utf16);
        let file = TextFile::open(
            &path,
            Layout::Lines { width: 81 },
            Decoding::Utf16 { big_endian: false },
        )
        .unwrap();
        assert_eq!(find(&file, &query("für"), 0, false), Some(6..12));
        std::fs::remove_file(&path).unwrap();

        let mut searches = Searches::default();
        for text in ["a", "b", "a"] {
            searches.add(query(text));
        }
        assert_eq!(
            (searches.history, searches.query.text),
            (vec!["a".into(), "b".into()], "a".into())
        );
        assert!(query("").pattern().is_err());
        assert!(
            Query {
                hex: true,
                ..query("4G")
            }
            .pattern()
            .is_err()
        );
        assert!(
            Query {
                regex: true,
                ..query("(")
            }
            .pattern()
            .is_err()
        );
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
