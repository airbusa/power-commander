// PowerCommander patch: files on the clipboard as file managers put them
// there, a URI list with GNOME's and KDE's mark of a cut, so that files
// copied or cut here paste in them, and theirs here.

use std::path::PathBuf;

use gpui::{ClipboardEntry, ClipboardItem, ClipboardString, ExternalPaths};
use url::Url;

pub(crate) const URI_LIST: &str = "text/uri-list";
pub(crate) const GNOME_FILES: &str = "x-special/gnome-copied-files";
pub(crate) const KDE_CUT: &str = "application/x-kde-cutselection";
/// Offered beside the text when the item has files.
pub(crate) const FILE_MIME_TYPES: [&str; 3] = [URI_LIST, GNOME_FILES, KDE_CUT];
/// A string entry's metadata saying the files are cut, not copied.
pub(crate) const CUT: &str = "cut";

/// The item's files, and whether they are cut.
pub(crate) fn files_of(item: &ClipboardItem) -> Option<(&[PathBuf], bool)> {
    let paths = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
        _ => None,
    })?;
    let cut = item.entries().iter().any(|entry| {
        matches!(entry, ClipboardEntry::String(text) if text.metadata.as_deref() == Some(CUT))
    });
    Some((paths, cut))
}

/// What a file manager reads as `mime_type`; none for other types.
pub(crate) fn file_bytes(item: &ClipboardItem, mime_type: &str) -> Option<Vec<u8>> {
    let (paths, cut) = files_of(item)?;
    let uris: Vec<String> = paths
        .iter()
        .filter_map(|path| Url::from_file_path(path).ok())
        .map(String::from)
        .collect();
    let text = match mime_type {
        URI_LIST => uris.iter().map(|uri| format!("{uri}\r\n")).collect(),
        GNOME_FILES => format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n")),
        KDE_CUT => (if cut { "1" } else { "0" }).to_owned(),
        _ => return None,
    };
    Some(text.into_bytes())
}

/// Another program's files: its URI list, cut when its KDE or GNOME mark
/// says so; none when an entry is no file of this computer.
pub(crate) fn files_item(
    uri_list: &[u8],
    kde_cut: Option<Vec<u8>>,
    gnome: Option<Vec<u8>>,
) -> Option<ClipboardItem> {
    let text = std::str::from_utf8(uri_list).ok()?;
    let lines = text.lines().map(str::trim);
    let uris = lines.filter(|line| !line.is_empty() && !line.starts_with('#'));
    let paths: Vec<PathBuf> = uris
        .map(|uri| Url::parse(uri).ok()?.to_file_path().ok())
        .collect::<Option<_>>()?;
    if paths.is_empty() {
        return None;
    }
    let cut = kde_cut.is_some_and(|mark| mark.starts_with(b"1"))
        || gnome.is_some_and(|files| files.starts_with(b"cut"));
    let shown: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
    Some(ClipboardItem {
        entries: vec![
            ClipboardEntry::ExternalPaths(ExternalPaths(paths.into())),
            ClipboardEntry::String(ClipboardString {
                text: shown.join("\n"),
                metadata: cut.then(|| CUT.to_owned()),
            }),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_both_ways() {
        let paths = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/é")];
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::ExternalPaths(ExternalPaths(paths.clone().into())),
                ClipboardEntry::String(ClipboardString {
                    text: "/tmp/a b.txt\n/tmp/é".into(),
                    metadata: Some(CUT.into()),
                }),
            ],
        };
        let list = file_bytes(&item, URI_LIST).unwrap();
        assert_eq!(list, b"file:///tmp/a%20b.txt\r\nfile:///tmp/%C3%A9\r\n");
        let gnome = file_bytes(&item, GNOME_FILES).unwrap();
        assert!(gnome.starts_with(b"cut\nfile:///tmp/a%20b.txt\n"));
        assert_eq!(file_bytes(&item, KDE_CUT).unwrap(), b"1");
        assert_eq!(file_bytes(&item, "text/plain"), None);
        let back = files_item(&list, Some(b"1".to_vec()), None).unwrap();
        assert_eq!(files_of(&back), Some((paths.as_slice(), true)));
        let copied = files_item(&list, None, Some(b"copy\n".to_vec())).unwrap();
        assert_eq!(files_of(&copied).map(|(_, cut)| cut), Some(false));
        assert!(files_item(b"https://example.com/\r\n", None, None).is_none());
    }
}
