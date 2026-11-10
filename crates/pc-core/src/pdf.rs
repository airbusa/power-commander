//! PDF pages and text, by the bundled PDFium (vendor/pdfium). PDFium is not
//! thread-safe: a thread of its own does all of it, keeping the last
//! document open while its pages are asked for.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc::{self, Receiver, Sender};

use pdfium_render::prelude::*;

/// A page's size, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// A rendered page: rows of opaque BGRA pixels, as GPUI takes them.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

type Reply<T> = Sender<io::Result<T>>;

enum Job {
    Pages(PathBuf, Reply<Vec<PageSize>>),
    Render(PathBuf, usize, u32, Reply<Bitmap>),
    Text(PathBuf, Reply<String>),
}

static WORKER: LazyLock<Sender<Job>> = LazyLock::new(|| {
    let (jobs, work) = mpsc::channel();
    std::thread::Builder::new()
        .name("pdf".into())
        .spawn(move || serve(work))
        .expect("a thread for PDFium");
    jobs
});

/// Beside the executable in releases, in vendor/pdfium when built.
fn library() -> PathBuf {
    let name = Pdfium::pdfium_platform_library_name();
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("pdfium").join(&name)));
    match beside {
        Some(path) if path.exists() => path,
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/pdfium")
            .join(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
            .join(name),
    }
}

/// PDFium is there to show PDFs with.
pub fn available() -> bool {
    library().exists()
}

/// Page sizes; blocks, so not on the UI thread, as the calls below.
pub fn pages(path: &Path) -> io::Result<Vec<PageSize>> {
    ask(|reply| Job::Pages(path.to_owned(), reply))
}

/// Page `page`, `width` pixels wide.
pub fn render(path: &Path, page: usize, width: u32) -> io::Result<Bitmap> {
    ask(|reply| Job::Render(path.to_owned(), page, width, reply))
}

/// Every page's text, pages a blank line apart.
pub fn text(path: &Path) -> io::Result<String> {
    ask(|reply| Job::Text(path.to_owned(), reply))
}

fn ask<T>(job: impl FnOnce(Reply<T>) -> Job) -> io::Result<T> {
    let (reply, answer) = mpsc::channel();
    let stopped = || io::Error::other("the PDF thread stopped");
    WORKER.send(job(reply)).map_err(|_| stopped())?;
    answer.recv().map_err(|_| stopped())?
}

fn failed(err: PdfiumError) -> io::Error {
    io::Error::other(err.to_string())
}

fn serve(jobs: Receiver<Job>) {
    let pdfium = Pdfium::bind_to_library(library())
        .map(Pdfium::new)
        .map_err(|err| {
            format!(
                "PDFium ({}): {err}; run vendor/pdfium/fetch.sh or fetch.bat",
                library().display()
            )
        });
    // The file as it was opened: a changed one opens again.
    let stamp = |path: &Path| {
        let meta = std::fs::metadata(path).ok();
        let changed = meta.as_ref().map(|meta| (meta.len(), meta.modified().ok()));
        (path.to_owned(), changed)
    };
    let mut open: Option<(_, PdfDocument)> = None;
    for job in jobs {
        let path = match &job {
            Job::Pages(path, _) | Job::Render(path, ..) | Job::Text(path, _) => path.clone(),
        };
        let now = stamp(&path);
        let document = match &pdfium {
            Err(why) => Err(io::Error::other(why.clone())),
            Ok(_) if open.as_ref().is_some_and(|(at, _)| *at == now) => Ok(()),
            Ok(pdfium) => {
                open = None;
                pdfium
                    .load_pdf_from_file(&path, None)
                    .map_err(failed)
                    .map(|document| {
                        open = Some((now, document));
                    })
            }
        };
        let document = document.map(|()| &open.as_ref().expect("just opened").1);
        match job {
            Job::Pages(_, reply) => {
                let _ = reply.send(document.map(|document| {
                    let pages = document.pages().iter();
                    let size = |page: PdfPage| PageSize {
                        width: page.width().value,
                        height: page.height().value,
                    };
                    pages.map(size).collect()
                }));
            }
            Job::Render(_, page, width, reply) => {
                let _ =
                    reply.send(document.and_then(|document| render_page(document, page, width)));
            }
            Job::Text(_, reply) => {
                let _ = reply.send(document.and_then(|document| {
                    let texts = document
                        .pages()
                        .iter()
                        .map(|page| page.text().map(|text| text.all()));
                    let texts: Result<Vec<String>, _> = texts.collect();
                    // PDFium marks a word hyphenated at a line's end so.
                    texts
                        .map(|texts| texts.join("\n\n").replace('\u{2}', ""))
                        .map_err(failed)
                }));
            }
        }
    }
}

fn render_page(document: &PdfDocument, page: usize, width: u32) -> io::Result<Bitmap> {
    let page = document.pages().get(page as PdfPageIndex).map_err(failed)?;
    let config = PdfRenderConfig::new()
        .set_target_width(width as Pixels)
        .set_format(PdfBitmapFormat::BGRA)
        .set_clear_color(PdfColor::WHITE)
        .render_form_data(true)
        .render_annotations(true);
    let bitmap = page.render_with_config(&config).map_err(failed)?;
    let (width, height) = (bitmap.width() as u32, bitmap.height() as u32);
    let raw = bitmap.as_raw_bytes();
    // Rows may be padded: keep each row's pixels only.
    let row = width as usize * 4;
    let stride = raw.len() / height.max(1) as usize;
    let bgra = match stride == row {
        true => raw,
        false => raw
            .chunks(stride)
            .flat_map(|line| &line[..row])
            .copied()
            .collect(),
    };
    Ok(Bitmap {
        width,
        height,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF saying `text`, 200 by 100 points.
    fn pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 24 Tf 20 40 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
                .into(),
            format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        let trailer = format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        );
        out.extend(trailer.bytes());
        out
    }

    #[test]
    fn renders_pages_and_reads_their_text() {
        assert!(available(), "run vendor/pdfium/fetch.sh or fetch.bat first");
        let path = std::env::temp_dir().join(format!("pc-pdf-test-{}.pdf", std::process::id()));
        std::fs::write(&path, pdf("Hello PDF")).unwrap();
        assert_eq!(
            pages(&path).unwrap(),
            [PageSize {
                width: 200.,
                height: 100.
            }]
        );
        let page = render(&path, 0, 400).unwrap();
        assert_eq!(
            (page.width, page.height, page.bgra.len()),
            (400, 200, 400 * 200 * 4)
        );
        assert!(page.bgra.chunks(4).all(|pixel| pixel[3] == 255), "opaque");
        assert!(page.bgra.chunks(4).any(|pixel| pixel[0] < 128), "some ink");
        assert_eq!(text(&path).unwrap().trim(), "Hello PDF");
        assert!(render(&path, 5, 100).is_err(), "no such page");
        std::fs::write(&path, b"not a pdf").unwrap();
        assert!(pages(&path).is_err(), "changed: opened again");
        std::fs::remove_file(&path).unwrap();
        assert!(pages(Path::new("/nonexistent.pdf")).is_err());
    }
}
