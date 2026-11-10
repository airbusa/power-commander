//! Pictures and PDF pages in the viewer: Sixel, kitty or iTerm2 images
//! where the terminal draws them, else half blocks, which any terminal can.

use std::path::PathBuf;
use std::sync::LazyLock;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, RgbaImage};
use pc_core::pdf;
use pc_core::viewer::Mode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use crate::ui::PANEL;
use crate::viewer::Viewer;
use crate::{App, Done};

/// Extensions of the pictures the terminal viewer decodes.
pub static PICTURES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    ImageFormat::all()
        .filter(|format| format.reading_enabled())
        .flat_map(|format| format.extensions_str())
        .copied()
        .collect()
});

/// How the terminal draws pictures. Asked before any key is read, as its
/// answers come on the same line.
pub fn ask_terminal() -> Picker {
    // tmux answers only part of the query, which then waits two seconds.
    if std::env::var_os("TMUX").is_some() {
        return Picker::halfblocks();
    }
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    // The library takes Konsole's Sixel for broken (24.12); 26.11 draws it
    // right, as the viewer clears the screen between pictures.
    let konsole = std::env::var("KONSOLE_VERSION").ok();
    if konsole.and_then(|version| version.parse::<u32>().ok()) >= Some(261100) {
        picker.set_protocol_type(ProtocolType::Sixel);
    }
    picker
}

/// What a picture shows: the file, the PDF's page, and the pixels it fits.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub path: PathBuf,
    pub page: Option<usize>,
    pub room: (u32, u32),
}

pub struct Picture {
    pub key: Key,
    /// The image, and its size in pixels; or why there is none.
    pub shown: Result<(StatefulProtocol, String), String>,
}

struct Loaded {
    image: DynamicImage,
    about: String,
    pages: usize,
}

/// Decodes the picture or renders the page, no larger than `key.room`.
fn load(key: &Key) -> Result<Loaded, String> {
    let (width, height) = (key.room.0.max(1), key.room.1.max(1));
    let Some(page) = key.page else {
        let reader = ImageReader::open(&key.path).and_then(|reader| reader.with_guessed_format());
        let image = reader.map_err(|err| err.to_string())?;
        let image = image.decode().map_err(|err| err.to_string())?;
        let about = format!("{} × {}", image.width(), image.height());
        // Fitted here, off the drawing thread.
        let image = match image.width() > width || image.height() > height {
            true => image.resize(width, height, FilterType::Triangle),
            false => image,
        };
        return Ok(Loaded {
            image,
            about,
            pages: 0,
        });
    };
    let sizes = pdf::pages(&key.path).map_err(|err| err.to_string())?;
    let size = sizes.get(page).ok_or("the PDF has no such page")?;
    let bitmap = pdf::render(&key.path, page, pdf::fit_width(*size, width, height))
        .map_err(|err| err.to_string())?;
    let (width, height) = (bitmap.width, bitmap.height);
    let image = RgbaImage::from_raw(width, height, bitmap.into_rgba()).ok_or("a broken page")?;
    Ok(Loaded {
        image: DynamicImage::ImageRgba8(image),
        about: String::new(),
        pages: sizes.len(),
    })
}

impl App {
    /// Starts loading the picture the viewer wants, unless it has it.
    pub fn want_picture(&mut self) {
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        if !matches!(viewer.shown.mode, Mode::Picture | Mode::Pdf) {
            return;
        }
        // Half blocks when the terminal was not asked, as in tests.
        let picker = self.picker.get_or_insert_with(Picker::halfblocks);
        let font = picker.font_size();
        let key = Key {
            path: viewer.shown.path().to_owned(),
            page: (viewer.shown.mode == Mode::Pdf).then_some(viewer.page),
            room: (
                viewer.width as u32 * u32::from(font.width),
                viewer.height as u32 * u32::from(font.height),
            ),
        };
        let has = viewer
            .picture
            .as_ref()
            .is_some_and(|picture| picture.key == key);
        if has || viewer.loading.as_ref() == Some(&key) {
            return;
        }
        viewer.loading = Some(key.clone());
        self.background(move || {
            let loaded = load(&key);
            Box::new(move |app: &mut App| {
                let shown = loaded.map(|loaded| {
                    let picker = app.picker.as_ref().expect("asked before loading");
                    let protocol = picker.new_resize_protocol(loaded.image);
                    (protocol, loaded.about, loaded.pages)
                });
                let Some(viewer) = app.viewer.as_mut() else {
                    return;
                };
                // Another file, page or size is wanted by now.
                if viewer.loading.as_ref() != Some(&key) {
                    return;
                }
                viewer.loading = None;
                let shown = shown.map(|(protocol, about, pages)| {
                    viewer.pages = pages;
                    (protocol, about)
                });
                viewer.picture = Some(Picture { key, shown });
            }) as Done
        });
    }
}

/// The picture in the middle of `area`, or why there is none.
pub fn draw(frame: &mut Frame, area: Rect, viewer: &mut Viewer) {
    let shown = viewer.picture.as_mut().map(|picture| &mut picture.shown);
    let note = match shown {
        Some(Ok((protocol, _))) => {
            let resize = Resize::Fit(None);
            let size = protocol.size_for(resize.clone(), area.as_size());
            let at = Rect::new(
                area.x + area.width.saturating_sub(size.width) / 2,
                area.y + area.height.saturating_sub(size.height) / 2,
                size.width.min(area.width),
                size.height.min(area.height),
            );
            frame.render_stateful_widget(StatefulImage::default().resize(resize), at, protocol);
            return;
        }
        Some(Err(err)) => err.clone(),
        None => "Loading…".into(),
    };
    let text = Paragraph::new(note)
        .style(PANEL)
        .centered()
        .wrap(Wrap { trim: true });
    let middle = Rect::new(
        area.x,
        area.y + area.height / 2,
        area.width,
        2.min(area.height),
    );
    frame.render_widget(text, middle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_pictures_into_the_room() {
        let dir = std::env::temp_dir().join(format!("pc-tui-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        RgbaImage::new(400, 100).save(&path).unwrap();
        let key = |room| Key {
            path: path.clone(),
            page: None,
            room,
        };
        let loaded = load(&key((100, 100))).unwrap();
        assert_eq!((loaded.image.width(), loaded.image.height()), (100, 25));
        assert_eq!(loaded.about, "400 × 100");
        let small = load(&key((1000, 1000))).unwrap();
        assert_eq!(small.image.width(), 400, "small ones keep their size");
        assert!(PICTURES.contains(&"png") && PICTURES.contains(&"jpg"));
        std::fs::write(&path, b"not a picture").unwrap();
        assert!(load(&key((100, 100))).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
