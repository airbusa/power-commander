//! Compare By Content: two text files line by line, side by side, changed
//! lines paired; other files byte by byte, 16 to a row.

use std::fs::File;
use std::io::{self, Read};
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, TextDiff};

use crate::Panel;
use crate::origin::Origin;

/// Larger files are compared as bytes only.
const TEXT_LIMIT: u64 = 32 << 20;
pub const BYTES_PER_ROW: usize = 16;
/// shortcut: further runs of differing rows are not listed; enough for any real file.
const MAX_RUNS: usize = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub ignore_case: bool,
    /// Runs of spaces and tabs count as one, and none at the line's ends.
    pub ignore_spaces: bool,
    /// Bytes even for text.
    pub binary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Same,
    Changed,
    /// Only on the left.
    Removed,
    /// Only on the right.
    Added,
}

/// A row of the two columns: each side's line number and text, if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub kind: Kind,
}

#[derive(Debug)]
pub enum Compared {
    Text(Vec<Row>),
    Bytes(Bytes),
}

impl Compared {
    pub fn rows(&self) -> usize {
        match self {
            Compared::Text(rows) => rows.len(),
            Compared::Bytes(bytes) => bytes.rows(),
        }
    }

    /// The runs of rows that differ.
    pub fn runs(&self) -> Vec<Range<usize>> {
        match self {
            Compared::Text(rows) => {
                let mut runs = Vec::new();
                for (ix, _) in rows.iter().enumerate().filter(|(_, r)| r.kind != Kind::Same) {
                    extend(&mut runs, ix);
                }
                runs
            }
            Compared::Bytes(bytes) => bytes.runs.clone(),
        }
    }
}

/// Two files byte by byte; their rows are read when shown.
#[derive(Debug)]
pub struct Bytes {
    files: [File; 2],
    pub sizes: (u64, u64),
    /// Runs of rows with a byte that differs.
    pub runs: Vec<Range<usize>>,
    pub first: Option<u64>,
    pub differing: u64,
}

impl Bytes {
    pub fn rows(&self) -> usize {
        self.sizes.0.max(self.sizes.1).div_ceil(BYTES_PER_ROW as u64) as usize
    }

    /// The row as offset, hex and characters on each side, with the
    /// byte ranges of the text that differ from the other side.
    pub fn row(&self, ix: usize) -> [(String, Vec<Range<usize>>); 2] {
        let at = (ix * BYTES_PER_ROW) as u64;
        let [a, b] = self.files.each_ref().map(|file| {
            let mut buf = [0; BYTES_PER_ROW];
            let read = read_at(file, at, &mut buf).unwrap_or(0);
            buf[..read].to_vec()
        });
        [hex_line(at, &a, &b), hex_line(at, &b, &a)]
    }
}

/// `00000010  41 42 …  AB…`: missing bytes are blank.
pub fn hex_line(at: u64, bytes: &[u8], other: &[u8]) -> (String, Vec<Range<usize>>) {
    let mut text = format!("{at:08X}  ");
    let mut differs = Vec::new();
    let hex_start = text.len();
    let chars_start = hex_start + 3 * BYTES_PER_ROW + 1;
    for i in 0..BYTES_PER_ROW {
        match bytes.get(i) {
            Some(byte) => text.push_str(&format!("{byte:02X} ")),
            None => text.push_str("   "),
        }
        if bytes.get(i).is_some() && bytes.get(i) != other.get(i) {
            differs.push(hex_start + 3 * i..hex_start + 3 * i + 2);
            differs.push(chars_start + i..chars_start + i + 1);
        }
    }
    text.push(' ');
    for byte in bytes {
        text.push(match byte {
            0x20..0x7f => *byte as char,
            _ => '.',
        });
    }
    differs.sort_by_key(|range| range.start);
    (text, differs)
}

#[cfg(unix)]
fn read_at(file: &File, at: u64, buf: &mut [u8]) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, at)
}

#[cfg(windows)]
fn read_at(file: &File, at: u64, buf: &mut [u8]) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, at)
}

/// The files to compare, as Total Commander: two marked here; else this
/// panel's marked file or cursor's against the other's, or its namesake there.
pub fn pair(this: &Panel, other: &Panel) -> Option<[Origin; 2]> {
    let files = |panel: &Panel| -> Vec<Origin> {
        let marked = panel.entries().iter().filter(|e| panel.is_marked(e));
        marked.filter_map(|e| panel.origin_of(e)).collect()
    };
    let mut marked = files(this);
    if marked.len() == 2 {
        let b = marked.pop()?;
        return Some([marked.pop()?, b]);
    }
    let one = |panel: &Panel, mut marked: Vec<Origin>| match marked.len() {
        1 => marked.pop(),
        _ => panel.origin(),
    };
    let here = one(this, marked)?;
    let there = one(other, files(other)).or_else(|| {
        let name = &this.current_entry()?.name;
        let namesake = other.entries().iter().find(|e| &e.name == name)?;
        other.origin_of(namesake)
    })?;
    Some([here, there])
}

pub fn compare_files(left: &Path, right: &Path, options: Options) -> io::Result<Compared> {
    let sizes = (
        std::fs::metadata(left)?.len(),
        std::fs::metadata(right)?.len(),
    );
    if !options.binary && sizes.0 <= TEXT_LIMIT && sizes.1 <= TEXT_LIMIT {
        let (a, b) = (std::fs::read(left)?, std::fs::read(right)?);
        if let (Some(a), Some(b)) = (text(&a), text(&b)) {
            return Ok(Compared::Text(compare_text(&a, &b, options)));
        }
    }
    compare_bytes(left, right, sizes)
}

/// Text without NUL bytes, other code pages read as Windows-1252.
fn text(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    Some(match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    })
}

pub fn compare_text(left: &str, right: &str, options: Options) -> Vec<Row> {
    let (a, b): (Vec<&str>, Vec<&str>) = (left.lines().collect(), right.lines().collect());
    // Lines are compared by these keys and shown as they are.
    let key = |line: &&str| {
        let line = match options.ignore_spaces {
            true => line.split_whitespace().collect::<Vec<_>>().join(" "),
            false => line.to_string(),
        };
        match options.ignore_case {
            true => line.to_lowercase(),
            false => line,
        }
    };
    let (keys_a, keys_b): (Vec<String>, Vec<String>) =
        (a.iter().map(key).collect(), b.iter().map(key).collect());
    let deadline = Some(Instant::now() + Duration::from_secs(5));
    let ops = similar::capture_diff_slices_deadline(Algorithm::Myers, &keys_a, &keys_b, deadline);
    let line = |lines: &[&str], ix: usize| lines.get(ix).map(|text| (ix + 1, (*text).to_owned()));
    let mut rows = Vec::new();
    for op in ops {
        let (old, new) = (op.old_range(), op.new_range());
        let kind = match op {
            DiffOp::Equal { .. } => Kind::Same,
            _ => Kind::Changed,
        };
        // Paired as far as both have lines; the rest is one side's.
        for i in 0..old.len().max(new.len()) {
            let l = (i < old.len()).then(|| line(&a, old.start + i)).flatten();
            let r = (i < new.len()).then(|| line(&b, new.start + i)).flatten();
            let kind = match (&l, &r) {
                (Some(_), Some(_)) => kind,
                (Some(_), None) => Kind::Removed,
                _ => Kind::Added,
            };
            rows.push(Row {
                left: l,
                right: r,
                kind,
            });
        }
    }
    rows
}

/// Where two changed lines differ: byte ranges in each.
pub fn changed_parts(left: &str, right: &str) -> [Vec<Range<usize>>; 2] {
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(20))
        .diff_chars(left, right);
    // Char positions to byte offsets.
    let offsets = |text: &str| -> Vec<usize> {
        let starts = text.char_indices().map(|(at, _)| at);
        starts.chain([text.len()]).collect()
    };
    let (a, b) = (offsets(left), offsets(right));
    let mut parts = [Vec::new(), Vec::new()];
    for op in diff.ops() {
        if let DiffOp::Equal { .. } = op {
            continue;
        }
        let (old, new) = (op.old_range(), op.new_range());
        if !old.is_empty() {
            parts[0].push(a[old.start]..a[old.end]);
        }
        if !new.is_empty() {
            parts[1].push(b[new.start]..b[new.end]);
        }
    }
    parts
}

fn compare_bytes(left: &Path, right: &Path, sizes: (u64, u64)) -> io::Result<Compared> {
    let (mut a, mut b) = (File::open(left)?, File::open(right)?);
    let (mut buf_a, mut buf_b) = (vec![0; 1 << 20], vec![0; 1 << 20]);
    let (mut at, mut first, mut differing, mut runs) = (0u64, None, 0u64, Vec::new());
    loop {
        let read_a = read_full(&mut a, &mut buf_a)?;
        let read_b = read_full(&mut b, &mut buf_b)?;
        let (both, most) = (read_a.min(read_b), read_a.max(read_b));
        let same = read_a == read_b && buf_a[..both] == buf_b[..both];
        for i in (0..most).filter(|_| !same) {
            if i >= both || buf_a[i] != buf_b[i] {
                first.get_or_insert(at + i as u64);
                differing += 1;
                extend(&mut runs, (at as usize + i) / BYTES_PER_ROW);
            }
        }
        if read_a < buf_a.len() || read_b < buf_b.len() {
            // A shorter file ends here; the other's rest all differs.
            differing += sizes.0.abs_diff(sizes.1) - (most - both) as u64;
            let rows = (sizes.0.max(sizes.1) as usize).div_ceil(BYTES_PER_ROW);
            if let Some(last) = runs.last_mut().filter(|_| sizes.0 != sizes.1) {
                last.end = last.end.max(rows);
            }
            break;
        }
        at += most as u64;
    }
    Ok(Compared::Bytes(Bytes {
        files: [File::open(left)?, File::open(right)?],
        sizes,
        runs,
        first,
        differing,
    }))
}

/// Adds a row to the runs: the last one's next row lengthens it.
fn extend(runs: &mut Vec<Range<usize>>, row: usize) {
    if let Some(last) = runs.last_mut().filter(|last| last.end >= row) {
        last.end = last.end.max(row + 1);
    } else if runs.len() < MAX_RUNS {
        runs.push(row..row + 1);
    }
}

fn read_full(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

/// The rows shown: all, or only those of the differences with `context`
/// rows around them, a gap between each part.
#[derive(Debug, Default)]
pub struct Visible {
    parts: Vec<Range<usize>>,
    /// Where each part begins among the shown.
    starts: Vec<usize>,
}

impl Visible {
    pub fn new(rows: usize, runs: &[Range<usize>], context: Option<usize>) -> Self {
        let mut parts: Vec<Range<usize>> = Vec::new();
        match context {
            None if rows > 0 => parts.push(0..rows),
            None => {}
            Some(context) => {
                for run in runs {
                    let part = run.start.saturating_sub(context)..(run.end + context).min(rows);
                    match parts.last_mut() {
                        Some(last) if last.end >= part.start => last.end = part.end,
                        _ => parts.push(part),
                    }
                }
            }
        }
        let starts = parts
            .iter()
            .scan(0, |at, part| {
                let start = *at;
                *at += part.len() + 1;
                Some(start)
            })
            .collect();
        Visible { parts, starts }
    }

    pub fn len(&self) -> usize {
        let rows: usize = self.parts.iter().map(|part| part.len()).sum();
        rows + self.parts.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The row shown at `ix`; none for a gap.
    pub fn row(&self, ix: usize) -> Option<usize> {
        let part = self.starts.partition_point(|&start| start <= ix).checked_sub(1)?;
        let offset = ix - self.starts[part];
        let range = &self.parts[part];
        (offset < range.len()).then(|| range.start + offset)
    }

    /// Where a row is shown, if it is.
    pub fn index_of(&self, row: usize) -> Option<usize> {
        let part = self.parts.partition_point(|part| part.end <= row);
        let range = self.parts.get(part).filter(|range| range.contains(&row))?;
        Some(self.starts[part] + row - range.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_changed_lines() {
        let exact = Options::default();
        let rows = compare_text("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n", exact);
        let kinds: Vec<Kind> = rows.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Same,
                Kind::Changed,
                Kind::Same,
                Kind::Same,
                Kind::Added
            ]
        );
        assert_eq!(rows[1].left, Some((2, "b".into())));
        assert_eq!(rows[1].right, Some((2, "B".into())));
        assert_eq!(rows[4].right, Some((5, "e".into())));
        assert_eq!(Compared::Text(rows).runs(), [1..2, 4..5]);
        let removed = compare_text("x\ny\n", "y\n", exact);
        assert_eq!(removed[0].kind, Kind::Removed);
        let loose = Options {
            ignore_case: true,
            ignore_spaces: true,
            binary: false,
        };
        let rows = compare_text("A  b\n c\n", "a b\nc \n", loose);
        assert!(rows.iter().all(|row| row.kind == Kind::Same));
        assert_eq!(rows[0].left, Some((1, "A  b".into())), "shown as they are");
        assert_eq!(changed_parts("abcd", "abXd"), [vec![2..3], vec![2..3]]);
        assert_eq!(changed_parts("é1", "é22"), [vec![2..3], vec![2..4]]);
    }

    #[test]
    fn compares_bytes() {
        let root = std::env::temp_dir().join(format!("pc-compare-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut a = vec![7u8; 40];
        a[1] = 0;
        let mut b = a.clone();
        b[2] = 9;
        b.extend([5, 5]);
        std::fs::write(root.join("a"), &a).unwrap();
        std::fs::write(root.join("b"), &b).unwrap();
        let Compared::Bytes(found) =
            compare_files(&root.join("a"), &root.join("b"), Options::default()).unwrap()
        else {
            panic!("bytes");
        };
        assert_eq!((found.first, found.differing, found.sizes), (Some(2), 3, (40, 42)));
        assert_eq!(found.runs, [0..1, 2..3]);
        assert_eq!(found.rows(), 3);
        let [(left, differs), (right, _)] = found.row(2);
        assert_eq!(left, "00000020  07 07 07 07 07 07 07 07                          ........");
        assert!(right.ends_with(".........."));
        assert!(differs.is_empty(), "only the right has more");
        let [(_, differs), _] = found.row(0);
        assert_eq!(differs, [16..18, 61..62]);
        std::fs::write(root.join("c"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("d"), "one\nthree\n").unwrap();
        let text = compare_files(&root.join("c"), &root.join("d"), Options::default()).unwrap();
        assert_eq!(text.rows(), 2);
        let binary = Options {
            binary: true,
            ..Options::default()
        };
        let bytes = compare_files(&root.join("c"), &root.join("d"), binary).unwrap();
        assert!(matches!(bytes, Compared::Bytes(_)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shows_only_differences() {
        let all = Visible::new(5, &[1..2], None);
        assert_eq!((all.len(), all.row(4), all.index_of(4)), (5, Some(4), Some(4)));
        let only = Visible::new(20, &[2..3, 4..5, 15..16], Some(1));
        // Rows 1..6, a gap, rows 14..17.
        assert_eq!(only.len(), 5 + 1 + 3);
        assert_eq!(only.row(0), Some(1));
        assert_eq!(only.row(5), None);
        assert_eq!(only.row(6), Some(14));
        assert_eq!(only.row(9), None);
        assert_eq!(only.index_of(15), Some(7));
        assert_eq!(only.index_of(10), None);
        assert!(Visible::new(0, &[], None).is_empty());
    }
}
