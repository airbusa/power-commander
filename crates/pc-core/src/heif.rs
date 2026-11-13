//! How a HEIF picture (HEIC, AVIF) is to be turned: its main item's `irot`
//! and `imir` properties, which mpv leaves out on iPhones' tiled photos.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// A step of the turn, in the file's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    /// This many quarter turns anticlockwise.
    Rotate(u8),
    /// Top and bottom swap, else left and right: `imir` 0 and 1, as libheif reads them.
    Mirror { top_bottom: bool },
}

/// The main item's turn; none for a picture as stored, or a file not read.
pub fn turns(path: &Path) -> Vec<Turn> {
    read_meta(path).map(|meta| parse(&meta)).unwrap_or_default()
}

/// The top-level `meta` box's content, after its version and flags.
fn read_meta(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let end = file.metadata()?.len();
    let mut at = 0;
    while at + 8 <= end {
        file.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; 8];
        file.read_exact(&mut head)?;
        let (mut size, kind) = (u64::from(u32::from_be_bytes(head[..4].try_into().unwrap())), &head[4..]);
        let mut header = 8;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large)?;
            (size, header) = (u64::from_be_bytes(large), 16);
        } else if size == 0 {
            size = end - at;
        }
        if size < header {
            break;
        }
        if kind == b"meta" {
            // Big enough for its item tables; the pictures are in `mdat`.
            let mut meta = vec![0u8; (size - header).min(16 << 20) as usize];
            file.read_exact(&mut meta)?;
            return Ok(meta.get(4..).unwrap_or_default().to_vec());
        }
        at += size;
    }
    Err(io::ErrorKind::NotFound.into())
}

/// The boxes in `data`: their type and content.
fn boxes(data: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut found = Vec::new();
    let mut rest = data;
    while rest.len() >= 8 {
        let size = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        let size = if size == 0 { rest.len() } else { size };
        if size < 8 || size > rest.len() {
            break;
        }
        found.push((&rest[4..8], &rest[8..size]));
        rest = &rest[size..];
    }
    found
}

fn child<'a>(data: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|(k, _)| *k == kind).map(|(_, content)| content)
}

fn parse(meta: &[u8]) -> Vec<Turn> {
    parse_turns(meta).unwrap_or_default()
}

fn parse_turns(meta: &[u8]) -> Option<Vec<Turn>> {
    let pitm = child(meta, b"pitm")?;
    let primary = match pitm[0] {
        0 => u32::from(u16::from_be_bytes(pitm.get(4..6)?.try_into().ok()?)),
        _ => u32::from_be_bytes(pitm.get(4..8)?.try_into().ok()?),
    };
    let iprp = child(meta, b"iprp")?;
    let properties = boxes(child(iprp, b"ipco")?);
    let ipma = child(iprp, b"ipma")?;
    let (version, wide) = (ipma[0], ipma[3] & 1 == 1);
    let count = u32::from_be_bytes(ipma.get(4..8)?.try_into().ok()?);
    let mut at = 8;
    for _ in 0..count {
        let item = match version {
            0 => u32::from(u16::from_be_bytes(ipma.get(at..at + 2)?.try_into().ok()?)),
            _ => u32::from_be_bytes(ipma.get(at..at + 4)?.try_into().ok()?),
        };
        at += if version == 0 { 2 } else { 4 };
        let associations = *ipma.get(at)? as usize;
        at += 1;
        let mut indexes = Vec::new();
        for _ in 0..associations {
            let index = match wide {
                true => usize::from(u16::from_be_bytes(ipma.get(at..at + 2)?.try_into().ok()?) & 0x7fff),
                false => usize::from(ipma.get(at)? & 0x7f),
            };
            at += if wide { 2 } else { 1 };
            indexes.push(index);
        }
        if item != primary {
            continue;
        }
        // 1-based; 0 is none.
        let turns = indexes.into_iter().filter_map(|index| properties.get(index.checked_sub(1)?).copied());
        return Some(
            turns
                .filter_map(|(kind, content)| match kind {
                    b"irot" => Some(Turn::Rotate(content.first()? & 3)).filter(|turn| *turn != Turn::Rotate(0)),
                    b"imir" => Some(Turn::Mirror { top_bottom: content.first()? & 1 == 0 }),
                    _ => None,
                })
                .collect(),
        );
    }
    Some(Vec::new())
}

/// `pixels` (4 bytes each, `width` by `height`) turned; the new width and height.
pub fn apply(turns: &[Turn], mut pixels: Vec<u8>, mut width: u32, mut height: u32) -> (Vec<u8>, u32, u32) {
    for turn in turns {
        if *turn == Turn::Rotate(0) {
            continue;
        }
        let (w, h) = (width as usize, height as usize);
        let mut out = vec![0u8; pixels.len()];
        // Where the pixel at (x, y) goes, in the turned picture of width `to_w`.
        let (to_w, place): (usize, Box<dyn Fn(usize, usize) -> (usize, usize)>) = match *turn {
            Turn::Rotate(1) => (h, Box::new(move |x, y| (y, w - 1 - x))),
            Turn::Rotate(2) => (w, Box::new(move |x, y| (w - 1 - x, h - 1 - y))),
            Turn::Rotate(_) => (h, Box::new(move |x, y| (h - 1 - y, x))),
            Turn::Mirror { top_bottom: false } => (w, Box::new(move |x, y| (w - 1 - x, y))),
            Turn::Mirror { top_bottom: true } => (w, Box::new(move |x, y| (x, h - 1 - y))),
        };
        for y in 0..h {
            for x in 0..w {
                let (tx, ty) = place(x, y);
                let (from, to) = ((y * w + x) * 4, (ty * to_w + tx) * 4);
                out[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
            }
        }
        if to_w != w {
            (width, height) = (height, width);
        }
        pixels = out;
    }
    (pixels, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(kind: &[u8], content: &[u8]) -> Vec<u8> {
        let mut out = ((content.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(content);
        out
    }

    /// An iPhone's: item 49 the grid, turned once and mirrored, its tiles not.
    #[test]
    fn reads_the_main_items_turn() {
        let pitm = make(b"pitm", &[0, 0, 0, 0, 0, 49]);
        let ipco = make(b"ipco", &[make(b"ispe", &[0; 12]), make(b"irot", &[1]), make(b"imir", &[0])].concat());
        // Version 0, narrow: item 2 has property 1; item 49 has 1, 2 (essential) and 3.
        let ipma = make(b"ipma", &[0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 1, 1, 0, 49, 3, 1, 0x82, 3].as_slice());
        let meta = [pitm, make(b"iprp", &[ipco, ipma].concat())].concat();
        assert_eq!(parse(&meta), [Turn::Rotate(1), Turn::Mirror { top_bottom: true }]);
        assert_eq!(parse(&meta[..10]), [], "cut short");
    }

    /// 2 by 1: [a b] turned a quarter anticlockwise is [b] over [a].
    #[test]
    fn turns_pixels() {
        let (a, b) = ([1u8; 4], [2u8; 4]);
        let pixels = [a, b].concat();
        assert_eq!(apply(&[Turn::Rotate(1)], pixels.clone(), 2, 1), ([b, a].concat(), 1, 2));
        assert_eq!(apply(&[Turn::Rotate(3)], pixels.clone(), 2, 1), ([a, b].concat(), 1, 2));
        assert_eq!(apply(&[Turn::Rotate(2)], pixels.clone(), 2, 1), ([b, a].concat(), 2, 1));
        assert_eq!(apply(&[Turn::Rotate(0)], pixels.clone(), 2, 1), ([a, b].concat(), 2, 1), "none");
        assert_eq!(apply(&[Turn::Mirror { top_bottom: false }], pixels.clone(), 2, 1), ([b, a].concat(), 2, 1));
        assert_eq!(apply(&[Turn::Mirror { top_bottom: true }], pixels, 2, 1), ([a, b].concat(), 2, 1));
    }
}
