//! Office documents as PDF, by the Craft tools (WordCraft, GridCraft,
//! DeckCraft) when they are on the PATH; converted once, then cached.

use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A converting program and the files it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Converter {
    /// For messages: "WordCraft".
    pub name: &'static str,
    pub program: PathBuf,
}

/// Each tool's name, program and files; each writes PDF by the output's extension.
const TOOLS: [(&str, &str, &[&str]); 3] = [
    ("WordCraft", "wordcraft-cli", &["docx", "docm", "dotx", "doc", "odt", "rtf"]),
    ("GridCraft", "gridcraft-cli", &["xlsx", "xlsm", "xltx", "xls", "xlsb", "ods"]),
    ("DeckCraft", "deckcraft-cli", &["pptx", "pptm", "potx"]),
];

/// The installed tool for this file, if any.
pub fn converter_for(path: &Path) -> Option<Converter> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    let (name, program, _) = TOOLS
        .iter()
        .find(|(_, _, extensions)| extensions.contains(&extension.as_str()))?;
    Some(Converter {
        name,
        program: on_path(program)?,
    })
}

pub(crate) fn on_path(program: &str) -> Option<PathBuf> {
    let folders = std::env::var_os("PATH")?;
    std::env::split_paths(&folders)
        .map(|folder| folder.join(program))
        .find(|path| path.is_file())
}

/// The file as PDF: from the cache when converted before, with the same
/// size and time; else converted now. Blocks.
pub fn to_pdf(converter: &Converter, path: &Path) -> io::Result<PathBuf> {
    let cache = dirs::cache_dir().ok_or_else(|| io::Error::other("no cache folder"))?;
    convert_into(&cache.join("powercommander/converted"), converter, path)
}

fn convert_into(folder: &Path, converter: &Converter, path: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let pdf = folder.join(format!("{}.pdf", key(path)?));
    if pdf.is_file() {
        return Ok(pdf);
    }
    // The tool picks the format by the extension, so the part ends in .pdf too.
    let part = pdf.with_extension("part.pdf");
    let done = Command::new(&converter.program)
        .arg("convert")
        .arg(path)
        .arg(&part)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !done.status.success() || !part.is_file() {
        let _ = std::fs::remove_file(&part);
        let why = String::from_utf8_lossy(&done.stderr).trim().to_owned();
        let why = match why.is_empty() {
            true => format!("{} could not convert it ({})", converter.name, done.status),
            false => format!("{}: {why}", converter.name),
        };
        return Err(io::Error::other(why));
    }
    std::fs::rename(&part, &pdf)?;
    Ok(pdf)
}

/// The cached copy's name: the file's path, size and time, hashed.
fn key(path: &Path) -> io::Result<String> {
    let meta = std::fs::metadata(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    OsString::from(path).hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in tool that copies its input, and fails on "bad".
    #[test]
    fn converts_once_and_caches() {
        let dir = std::env::temp_dir().join(format!("pc-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("fake-cli");
        std::fs::write(&tool, "#!/bin/sh\ncase \"$2\" in *bad*) echo broken >&2; exit 3;; esac\ncp \"$2\" \"$3\"\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let converter = Converter {
            name: "Fake",
            program: tool,
        };
        let cache = dir.join("cache");
        let doc = dir.join("report.docx");
        std::fs::write(&doc, "first").unwrap();
        let pdf = convert_into(&cache, &converter, &doc).unwrap();
        assert_eq!(std::fs::read_to_string(&pdf).unwrap(), "first");
        assert_eq!(convert_into(&cache, &converter, &doc).unwrap(), pdf, "from the cache");
        std::fs::write(&doc, "second!").unwrap();
        let again = convert_into(&cache, &converter, &doc).unwrap();
        assert_ne!(again, pdf, "a changed file is converted again");
        let bad = dir.join("bad.docx");
        std::fs::write(&bad, "x").unwrap();
        let err = convert_into(&cache, &converter, &bad).unwrap_err().to_string();
        assert_eq!(err, "Fake: broken");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
