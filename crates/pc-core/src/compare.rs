//! Compare By Content: two text files line by line, side by side, changed
//! lines paired; other files byte by byte.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use similar::{DiffOp, TextDiff};

/// Larger files are compared as bytes only.
const TEXT_LIMIT: u64 = 32 << 20;

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

#[derive(Debug, PartialEq)]
pub enum Compared {
    Text(Vec<Row>),
    /// Where they first differ, if anywhere, and how many bytes differ.
    Bytes {
        first: Option<u64>,
        differing: u64,
        sizes: (u64, u64),
    },
}

pub fn compare_files(left: &Path, right: &Path) -> io::Result<Compared> {
    let sizes = (
        std::fs::metadata(left)?.len(),
        std::fs::metadata(right)?.len(),
    );
    if sizes.0 <= TEXT_LIMIT && sizes.1 <= TEXT_LIMIT {
        let (a, b) = (std::fs::read(left)?, std::fs::read(right)?);
        if let (Some(a), Some(b)) = (text(&a), text(&b)) {
            return Ok(Compared::Text(compare_text(&a, &b)));
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

pub fn compare_text(left: &str, right: &str) -> Vec<Row> {
    let diff = TextDiff::from_lines(left, right);
    let (a, b): (Vec<&str>, Vec<&str>) = (left.lines().collect(), right.lines().collect());
    let line = |lines: &[&str], ix: usize| lines.get(ix).map(|text| (ix + 1, (*text).to_owned()));
    let mut rows = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    rows.push(Row {
                        left: line(&a, old_index + i),
                        right: line(&b, new_index + i),
                        kind: Kind::Same,
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                rows.extend((0..old_len).map(|i| Row {
                    left: line(&a, old_index + i),
                    right: None,
                    kind: Kind::Removed,
                }));
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                rows.extend((0..new_len).map(|i| Row {
                    left: None,
                    right: line(&b, new_index + i),
                    kind: Kind::Added,
                }));
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                // Paired as far as both have lines; the rest is one side's.
                for i in 0..old_len.max(new_len) {
                    let (l, r) = (
                        (i < old_len).then(|| line(&a, old_index + i)).flatten(),
                        (i < new_len).then(|| line(&b, new_index + i)).flatten(),
                    );
                    let kind = match (&l, &r) {
                        (Some(_), Some(_)) => Kind::Changed,
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
        }
    }
    rows
}

/// The rows where a run of differences starts, for Next and Previous.
pub fn differences(rows: &[Row]) -> Vec<usize> {
    (0..rows.len())
        .filter(|&ix| rows[ix].kind != Kind::Same && (ix == 0 || rows[ix - 1].kind == Kind::Same))
        .collect()
}

fn compare_bytes(left: &Path, right: &Path, sizes: (u64, u64)) -> io::Result<Compared> {
    let (mut a, mut b) = (File::open(left)?, File::open(right)?);
    let (mut buf_a, mut buf_b) = (vec![0; 1 << 20], vec![0; 1 << 20]);
    let (mut at, mut first, mut differing) = (0u64, None, 0u64);
    loop {
        let read_a = read_full(&mut a, &mut buf_a)?;
        let read_b = read_full(&mut b, &mut buf_b)?;
        let both = read_a.min(read_b);
        for i in 0..both {
            if buf_a[i] != buf_b[i] {
                first.get_or_insert(at + i as u64);
                differing += 1;
            }
        }
        if read_a != read_b {
            first.get_or_insert(at + both as u64);
            differing += sizes.0.abs_diff(sizes.1);
            break;
        }
        if read_a == 0 {
            break;
        }
        at += both as u64;
    }
    Ok(Compared::Bytes {
        first,
        differing,
        sizes,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_changed_lines() {
        let rows = compare_text("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
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
        assert_eq!(differences(&rows), [1, 4]);
        let removed = compare_text("x\ny\n", "y\n");
        assert_eq!(removed[0].kind, Kind::Removed);
    }

    #[test]
    fn compares_bytes() {
        let root = std::env::temp_dir().join(format!("pc-compare-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a"), [1, 0, 3, 4]).unwrap();
        std::fs::write(root.join("b"), [1, 0, 9, 4, 5]).unwrap();
        let found = compare_files(&root.join("a"), &root.join("b")).unwrap();
        assert_eq!(
            found,
            Compared::Bytes {
                first: Some(2),
                differing: 2,
                sizes: (4, 5)
            }
        );
        std::fs::write(root.join("c"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("d"), "one\nthree\n").unwrap();
        let Compared::Text(rows) = compare_files(&root.join("c"), &root.join("d")).unwrap() else {
            panic!("text");
        };
        assert_eq!(rows.len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
