//! macOS: Finder's tags, as its extended attribute keeps them.

use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    /// Finder's label: 0 none, 1 grey, 2 green, 3 purple, 4 blue, 5 yellow, 6 red, 7 orange.
    pub colour: u8,
}

/// A label's colour as Finder draws it.
pub fn rgb(colour: u8) -> Option<[u8; 3]> {
    Some(match colour {
        1 => [160, 162, 162],
        2 => [155, 220, 121],
        3 => [204, 148, 223],
        4 => [113, 187, 242],
        5 => [248, 214, 110],
        6 => [233, 109, 102],
        7 => [241, 172, 97],
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
pub fn read(path: &Path) -> Vec<Tag> {
    attribute(path).map(|data| parse(&data)).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
pub fn read(_: &Path) -> Vec<Tag> {
    Vec::new()
}

/// The labels' colours, for the list.
pub fn colours(path: &Path) -> Vec<u8> {
    read(path).into_iter().map(|tag| tag.colour).filter(|&colour| colour != 0).collect()
}

#[cfg(target_os = "macos")]
fn attribute(path: &Path) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = c"com.apple.metadata:_kMDItemUserTags";
    let size = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    if size <= 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    let read = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), data.as_mut_ptr().cast(), data.len(), 0, 0) };
    data.truncate(read.max(0) as usize);
    Some(data)
}

/// `Name\nColour` each, in a binary property list's array.
fn parse(data: &[u8]) -> Vec<Tag> {
    let tag = |text: String| match text.rsplit_once('\n') {
        Some((name, colour)) => Tag { name: name.to_owned(), colour: colour.parse().unwrap_or(0) },
        None => Tag { name: text, colour: 0 },
    };
    strings(data).unwrap_or_default().into_iter().map(tag).collect()
}

fn strings(data: &[u8]) -> Option<Vec<String>> {
    if !data.starts_with(b"bplist00") || data.len() < 40 {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let (offset_size, ref_size) = (trailer[6] as usize, trailer[7] as usize);
    let (count, top, table) = (number(&trailer[8..16])?, number(&trailer[16..24])?, number(&trailer[24..32])?);
    let offset = |object: usize| {
        let at = table.checked_add(object.checked_mul(offset_size)?)?;
        (object < count).then(|| number(data.get(at..at.checked_add(offset_size)?)?)).flatten()
    };
    let (kind, len, items) = object(data, offset(top)?)?;
    if kind != 0xA {
        return None;
    }
    (0..len)
        .map(|ix| {
            let at = items.checked_add(ix.checked_mul(ref_size)?)?;
            let reference = number(data.get(at..at.checked_add(ref_size)?)?)?;
            let (kind, len, at) = object(data, offset(reference)?)?;
            match kind {
                0x5 => Some(String::from_utf8_lossy(data.get(at..at.checked_add(len)?)?).into_owned()),
                0x6 => {
                    let units: Vec<u16> =
                        data.get(at..at.checked_add(len.checked_mul(2)?)?)?.chunks(2).map(|unit| u16::from_be_bytes([unit[0], unit[1]])).collect();
                    Some(String::from_utf16_lossy(&units))
                }
                _ => None,
            }
        })
        .collect()
}

/// An object's kind, length and where what it holds starts.
fn object(data: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let marker = *data.get(at)?;
    let (kind, len) = (marker >> 4, (marker & 0xF) as usize);
    if len != 0xF {
        return Some((kind, len, at + 1));
    }
    let int = *data.get(at + 1)?;
    let size = 1 << (int & 0xF);
    (int >> 4 == 1).then(|| Some((kind, number(data.get(at + 2..at + 2 + size)?)?, at + 2 + size))).flatten()
}

fn number(bytes: &[u8]) -> Option<usize> {
    (bytes.len() <= 8).then(|| bytes.iter().fold(0, |n, &byte| n << 8 | byte as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_finders_list() {
        // plutil's binary1 of ["Red\n6", "Work", "Zöld\n2"].
        let data = [
            0x62, 0x70, 0x6c, 0x69, 0x73, 0x74, 0x30, 0x30, 0xa3, 0x01, 0x02, 0x03, 0x55, 0x52, 0x65, 0x64, 0x0a, 0x36,
            0x54, 0x57, 0x6f, 0x72, 0x6b, 0x66, 0x00, 0x5a, 0x00, 0xf6, 0x00, 0x6c, 0x00, 0x64, 0x00, 0x0a, 0x00, 0x32,
            0x08, 0x0c, 0x12, 0x17, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x24,
        ];
        let tag = |name: &str, colour| Tag { name: name.into(), colour };
        assert_eq!(parse(&data), [tag("Red", 6), tag("Work", 0), tag("Zöld", 2)]);
        assert_eq!(parse(b"not a plist"), []);
        assert_eq!(parse(&data[..50]), []);
    }
}
