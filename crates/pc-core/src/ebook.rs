//! Books for the viewer, read as one HTML document, as the UI kit's text view
//! renders it: an EPUB's chapters in reading order, an FB2's sections, their
//! pictures inline as `data:` URLs. Books with DRM cannot be read.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use base64::Engine as _;
use regex::{Captures, Regex};

pub const EXTENSIONS: &[&str] = &["epub", "fb2"];

pub fn is_book(path: &Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    extension.is_some_and(|extension| EXTENSIONS.contains(&extension.as_str()))
}

/// The book as one HTML document.
pub fn html(path: &Path) -> io::Result<String> {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    match extension.as_deref() {
        Some("fb2") => fb2(&String::from_utf8_lossy(&fs::read(path)?)),
        _ => epub(&Zip::open(fs::read(path)?)?),
    }
}

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

/// A zip's files, stored or deflated, as an EPUB has them (never zip64).
struct Zip {
    data: Vec<u8>,
    /// By name: the local header's offset, the method and the compressed size.
    entries: HashMap<String, (usize, u16, usize)>,
}

impl Zip {
    fn open(data: Vec<u8>) -> io::Result<Zip> {
        let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
        let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        // The end of the central directory, within the last 64 KiB and its own 22 bytes.
        let last = data.len().saturating_sub(22);
        let end = (last.saturating_sub(65535)..=last)
            .rev()
            .find(|&at| data.get(at..at + 4) == Some(&[0x50, 0x4b, 0x05, 0x06]))
            .ok_or_else(|| invalid("not a zip: a damaged or a DRM-protected book?"))?;
        let short = || invalid("a damaged zip");
        let (count, mut at) = (u16_at(end + 10).ok_or_else(short)?, u32_at(end + 16).ok_or_else(short)?);
        let mut entries = HashMap::new();
        for _ in 0..count {
            if data.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
                return Err(short());
            }
            let method = u16_at(at + 10).ok_or_else(short)? as u16;
            let size = u32_at(at + 20).ok_or_else(short)?;
            let (name, extra, comment) = (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32));
            let (name, extra, comment) = (name.ok_or_else(short)?, extra.ok_or_else(short)?, comment.ok_or_else(short)?);
            let local = u32_at(at + 42).ok_or_else(short)?;
            let name = data.get(at + 46..at + 46 + name).ok_or_else(short)?;
            entries.insert(String::from_utf8_lossy(name).into_owned(), (local, method, size));
            at += 46 + name.len() + extra + comment;
        }
        Ok(Zip { data, entries })
    }

    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, format!("{name} is not in the book"));
        let &(local, method, size) = self.entries.get(name).ok_or_else(missing)?;
        let field = |at: usize| self.data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
        let (name_len, extra) = (field(local + 26), field(local + 28));
        let start = local + 30 + name_len.ok_or_else(missing)? + extra.ok_or_else(missing)?;
        let raw = self.data.get(start..start + size).ok_or_else(|| invalid("a damaged zip"))?;
        match method {
            0 => Ok(raw.to_vec()),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut out)?;
                Ok(out)
            }
            _ => Err(invalid(format!("{name}: compressed as zip cannot read here ({method})"))),
        }
    }

    fn text(&self, name: &str) -> io::Result<String> {
        Ok(String::from_utf8_lossy(&self.read(name)?).into_owned())
    }
}

/// `href` from the folder `base` (with its `/`), as a zip names it: `..`
/// and `.` resolved, `%20` decoded, the `#fragment` gone.
fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for part in format!("{base}{}", percent_decode(href)).split('/') {
        match part {
            "" | "." => {}
            ".." => drop(parts.pop()),
            part => parts.push(part.to_owned()),
        }
    }
    parts.join("/")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes.get(at + 1..at + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The folder of a path inside the book, with its `/`; empty at the top.
fn folder_of(path: &str) -> &str {
    path.rfind('/').map_or("", |slash| &path[..=slash])
}

fn data_url(name: &str, bytes: &[u8]) -> String {
    let extension = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn epub(zip: &Zip) -> io::Result<String> {
    let container = zip.text("META-INF/container.xml")?;
    let container = roxmltree::Document::parse(&container).map_err(|err| invalid(err.to_string()))?;
    let package = container
        .descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .ok_or_else(|| invalid("no package file in the book"))?
        .to_owned();
    let opf = zip.text(&package)?;
    let opf = roxmltree::Document::parse(&opf).map_err(|err| invalid(err.to_string()))?;
    let base = folder_of(&package);
    let manifest: HashMap<&str, (&str, &str)> = opf
        .descendants()
        .filter(|node| node.tag_name().name() == "item")
        .filter_map(|item| Some((item.attribute("id")?, (item.attribute("href")?, item.attribute("media-type")?))))
        .collect();
    let spine: Vec<&str> = opf
        .descendants()
        .filter(|node| node.tag_name().name() == "itemref")
        .filter_map(|itemref| itemref.attribute("idref"))
        .collect();
    let mut html = String::new();
    for id in spine {
        let Some((href, kind)) = manifest.get(id) else {
            continue;
        };
        if !kind.contains("html") {
            continue;
        }
        let chapter = resolve(base, href);
        // A chapter that is not there leaves a gap, not an unreadable book.
        if let Ok(xhtml) = zip.text(&chapter) {
            html.push_str(&chapter_html(zip, &chapter, &xhtml));
        }
    }
    if html.trim().is_empty() {
        return Err(invalid("the book has no text"));
    }
    Ok(html)
}

/// Elements that have no end, `<br/>` and the like.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// A chapter's body, its pictures inline, its links only text.
fn chapter_html(zip: &Zip, chapter: &str, xhtml: &str) -> String {
    use std::sync::LazyLock;
    static BODY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<body[^>]*>(.*)</body\s*>").unwrap());
    static DROPPED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<(script|style)\b.*?</(script|style)\s*>").unwrap());
    // A cover is often an SVG holding one picture.
    static SVG_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<svg\b.*?<image\b[^>]*?\bhref\s*=\s*(["'])(.*?)["'][^>]*>.*?</svg\s*>"#).unwrap()
    });
    static IMG_SRC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?is)(<img\b[^>]*?\bsrc\s*=\s*)(["'])(.*?)["']"#).unwrap());
    // XHTML's `<a id="x"/>` never ends as HTML: all that follows would be its link.
    static SELF_CLOSED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<([a-z][a-z0-9]*)\b([^<>]*?)/\s*>").unwrap());
    // Links lead nowhere from the viewer: their text stays.
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<a\b[^>]*>|</a\s*>").unwrap());
    let body = BODY.captures(xhtml).and_then(|body| body.get(1)).map_or(xhtml, |body| body.as_str());
    let body = DROPPED.replace_all(body, "");
    let folder = folder_of(chapter);
    let inline = |src: &str| {
        let name = resolve(folder, src);
        zip.read(&name).map_or_else(|_| src.to_owned(), |bytes| data_url(&name, &bytes))
    };
    let body = SVG_IMAGE.replace_all(&body, |found: &Captures| format!(r#"<img src="{}"/>"#, inline(&found[2])));
    let body = IMG_SRC.replace_all(&body, |found: &Captures| format!(r#"{}"{}""#, &found[1], inline(&found[3])));
    let body = SELF_CLOSED.replace_all(&body, |found: &Captures| {
        let tag = found[1].to_lowercase();
        match VOID.contains(&tag.as_str()) {
            true => found[0].to_owned(),
            false => format!("<{tag}{}></{tag}>", &found[2]),
        }
    });
    let body = LINK.replace_all(&body, "");
    format!("{body}\n<br/>\n")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// FictionBook 2: its bodies' sections as HTML, its pictures from its binaries.
fn fb2(xml: &str) -> io::Result<String> {
    let doc = roxmltree::Document::parse(xml).map_err(|err| invalid(err.to_string()))?;
    let pictures: HashMap<&str, String> = doc
        .descendants()
        .filter(|node| node.tag_name().name() == "binary")
        .filter_map(|binary| {
            let kind = binary.attribute("content-type").unwrap_or("image/jpeg");
            let data: String = binary.text()?.split_whitespace().collect();
            Some((binary.attribute("id")?, format!("data:{kind};base64,{data}")))
        })
        .collect();
    let mut html = String::new();
    for body in doc.root_element().children().filter(|node| node.tag_name().name() == "body") {
        fb2_node(body, &pictures, &mut html);
    }
    if html.trim().is_empty() {
        return Err(invalid("the book has no text"));
    }
    Ok(html)
}

fn fb2_node(node: roxmltree::Node, pictures: &HashMap<&str, String>, html: &mut String) {
    if node.is_text() {
        html.push_str(&escape(node.text().unwrap_or_default()));
        return;
    }
    let children = |html: &mut String| {
        for child in node.children() {
            fb2_node(child, pictures, html);
        }
    };
    let wrapped = |tag: &str, html: &mut String| {
        html.push_str(&format!("<{tag}>"));
        children(html);
        html.push_str(&format!("</{tag}>"));
    };
    match node.tag_name().name() {
        "title" => wrapped("h2", html),
        "subtitle" => wrapped("h3", html),
        "p" | "v" | "text-author" => wrapped("p", html),
        "emphasis" => wrapped("i", html),
        "strong" => wrapped("b", html),
        "strikethrough" => wrapped("s", html),
        "code" => wrapped("code", html),
        "epigraph" | "cite" => wrapped("blockquote", html),
        "table" | "tr" | "td" | "th" => wrapped(node.tag_name().name(), html),
        "empty-line" => html.push_str("<br/>"),
        "image" => {
            let href = node.attributes().find(|attribute| attribute.name() == "href").map(|a| a.value());
            if let Some(picture) = href.and_then(|href| pictures.get(href.trim_start_matches('#'))) {
                html.push_str(&format!(r#"<img src="{picture}"/>"#));
            }
        }
        // The description is the book's card, not its text.
        "description" | "binary" => {}
        _ => children(html),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip of `files`, stored: what the reader needs (no CRCs).
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let (mut out, mut directory) = (Vec::new(), Vec::new());
        for (name, data) in files {
            let local = out.len() as u32;
            let header = |signature: &[u8], out: &mut Vec<u8>| {
                out.extend_from_slice(signature);
                // Time, date, CRC.
                out.extend_from_slice(&[0; 8]);
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            };
            header(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0], &mut out);
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0]);
            directory.extend_from_slice(&[0; 8]);
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&local.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reads_an_epub_in_reading_order() {
        let container = br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let opf = br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest>
            <item id="b" href="text/two.xhtml" media-type="application/xhtml+xml"/>
            <item id="a" href="text/one%20page.xhtml" media-type="application/xhtml+xml"/>
            <item id="p" href="images/dot.png" media-type="image/png"/>
            </manifest><spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#;
        let one = br#"<html><head><style>p{}</style></head><body><a id="top"/><h1>One</h1><p>First&nbsp;page <a href="two.xhtml#x">on</a></p><img src="../images/dot.png" alt="dot"/></body></html>"#;
        let two = br#"<html><body><p>Second</p></body></html>"#;
        let book = zip(&[
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/text/one page.xhtml", one),
            ("OEBPS/text/two.xhtml", two),
            ("OEBPS/images/dot.png", b"PNG"),
        ]);
        let html = epub(&Zip::open(book).unwrap()).unwrap();
        let (first, second) = (html.find("First").unwrap(), html.find("Second").unwrap());
        assert!(first < second, "spine order: {html}");
        assert!(html.contains(r#"src="data:image/png;base64,UE5H""#), "the picture inline: {html}");
        assert!(!html.contains("<style>") && !html.contains("two.xhtml"), "{html}");
        assert!(!html.contains("<a") && html.contains("page on</p>"), "links only text: {html}");
    }

    #[test]
    fn reads_fiction_book() {
        let xml = r##"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
<description><title-info><book-title>Card</book-title></title-info></description>
<body><section><title><p>Chapter 1</p></title><p>It was <emphasis>dark</emphasis> &amp; cold.</p><empty-line/><image l:href="#pic"/></section></body>
<binary id="pic" content-type="image/png">UE5H</binary></FictionBook>"##;
        let html = fb2(xml).unwrap();
        assert!(html.contains("<h2><p>Chapter 1</p></h2>"), "{html}");
        assert!(html.contains("It was <i>dark</i> &amp; cold."), "{html}");
        assert!(html.contains(r#"<img src="data:image/png;base64,UE5H"/>"#), "{html}");
        assert!(!html.contains("Card"), "the description is not text: {html}");
    }
}
