//! Brief and Thumbnails, Total Commander's other panel views; Full is the
//! panel's rows. Pictures' thumbnails are made in the background.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::h_flex;
use gpui_kit::*;
use pc_core::View;

use crate::{Commander, ROW, SCROLLBAR, Side};

pub(crate) const TILE_WIDTH: f32 = 116.;
pub(crate) const TILE_HEIGHT: f32 = 132.;
/// A thumbnail's longer side in pixels: sharp at twice the scale.
const THUMB_PIXELS: u32 = 192;
/// Thumbnails kept; then they are made again as they show.
const THUMBS_KEPT: usize = 3000;

/// Pictures' thumbnails by path: none while being made, or where it failed;
/// and how many libmpv is drawing (HEIC, AVIF, a video's frame).
#[derive(Default)]
pub(crate) struct Thumbnails(HashMap<PathBuf, Option<Arc<RenderImage>>>, usize);

/// Each holds its full picture in memory while drawn: the rest wait.
const STILLS_AT_ONCE: usize = 2;

impl Side {
    /// The rows a Brief column has, as the panel is high.
    pub(crate) fn brief_rows(&self) -> usize {
        ((self.area.get().height / ROW).floor() as usize).max(1)
    }

    /// Tiles side by side in Thumbnails.
    pub(crate) fn tiles_per_row(&self) -> usize {
        (f32::from(self.area.get().width - SCROLLBAR) / TILE_WIDTH).floor().max(1.) as usize
    }

    /// A Brief column's width, from the longest name.
    fn brief_width(&self) -> f32 {
        let entries = self.tabs.current().entries();
        let longest = entries.iter().map(|entry| entry.display_name().chars().count()).max().unwrap_or(0);
        (longest as f32 * 7.5 + 34.).clamp(110., 360.)
    }

    /// The cursor moves this much for Up and Down.
    pub(crate) fn vertical_step(&self) -> isize {
        match self.tabs.current().view() {
            View::Thumbnails => self.tiles_per_row() as isize,
            View::Full | View::Brief => 1,
        }
    }

    /// Left and Right: a column in Brief, a tile in Thumbnails; none in Full.
    pub(crate) fn sideways_step(&self) -> Option<isize> {
        match self.tabs.current().view() {
            View::Full => None,
            View::Brief => Some(self.brief_rows() as isize),
            View::Thumbnails => Some(1),
        }
    }

    /// Page Up and Page Down: what shows at once.
    pub(crate) fn page_step(&self) -> isize {
        let size = self.area.get();
        let step = match self.tabs.current().view() {
            View::Full => (size.height / ROW) as usize,
            View::Brief => self.brief_rows() * (f32::from(size.width) / self.brief_width()).floor().max(1.) as usize,
            View::Thumbnails => self.tiles_per_row() * (f32::from(size.height) / TILE_HEIGHT).floor().max(1.) as usize,
        };
        (step.max(2) - 1) as isize
    }

    pub(crate) fn scroll_to_cursor(&self) {
        let cursor = self.tabs.current().cursor();
        match self.tabs.current().view() {
            View::Full => self.scroll.scroll_to_item(cursor, ScrollStrategy::Nearest),
            View::Thumbnails => self.scroll.scroll_to_item(cursor / self.tiles_per_row(), ScrollStrategy::Nearest),
            View::Brief => {
                // The cursor's column, wholly in sight.
                let width = self.brief_width();
                let left = (cursor / self.brief_rows()) as f32 * width;
                let shown = f32::from(self.area.get().width);
                let scrolled = -f32::from(self.brief.offset().x);
                let scrolled = scrolled.min(left).max(left + width - shown).max(0.);
                self.brief.set_offset(point(px(-scrolled), px(0.)));
            }
        }
    }
}

/// Keeps `area` at the size the rows have; a change renders `view` again with it.
pub(crate) fn measure(area: Rc<Cell<Size<Pixels>>>, view: EntityId) -> impl IntoElement {
    let seen = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
        // A notice while drawing is lost: the next frame takes it.
        if area.replace(bounds.size) != bounds.size {
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
    };
    canvas(seen, |_, _, _, _| {}).absolute().top_0().left_0().size_full()
}

impl Commander {
    /// Brief: names in columns of the panel's height, side by side; only the
    /// columns in sight are drawn.
    pub(crate) fn render_brief(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let side = &self.panels[ix];
        let count = side.tabs.current().entries().len();
        let (rows, width) = (side.brief_rows(), side.brief_width());
        let columns = count.div_ceil(rows);
        let scrolled = -f32::from(side.brief.offset().x);
        let first = (scrolled / width).floor().max(0.) as usize;
        let shown = (f32::from(side.area.get().width) / width).ceil() as usize + 1;
        let column = |column: usize| {
            let entries = column * rows..((column + 1) * rows).min(count);
            let left = px(width * column as f32);
            let column = div().absolute().top_0().left(left).w(px(width)).flex().flex_col();
            column.children(entries.map(|row| self.render_brief_entry(ix, row, cx)))
        };
        let content = div()
            .relative()
            .h_full()
            .w(px(width * columns as f32))
            .children((first..(first + shown).min(columns)).map(column));
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .pb(SCROLLBAR)
            .child(measure(side.area.clone(), cx.entity_id()))
            .child(
                div()
                    .id(("brief", ix))
                    .size_full()
                    .overflow_x_scroll()
                    .track_scroll(&side.brief)
                    .child(content),
            )
            .child(div().absolute().inset_0().child(Scrollbar::horizontal(&side.brief)))
            .into_any_element()
    }

    fn render_brief_entry(&self, ix: usize, row: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let icon = self.entry_icon(ix, entry, px(16.), cx);
        self.entry_base(ix, row, false, cx)
            .flex()
            .items_center()
            .h(ROW)
            .px_1()
            .gap_1()
            .child(icon)
            .child(div().min_w_0().truncate().child(entry.display_name()))
    }

    /// Thumbnails: a grid of tiles, a row of them an item of the list.
    pub(crate) fn render_thumbnails(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let side = &self.panels[ix];
        let count = side.tabs.current().entries().len();
        let per_row = side.tiles_per_row();
        let tiles = uniform_list(
            ("thumbnails", ix),
            count.div_ceil(per_row),
            cx.processor(move |this, lines: std::ops::Range<usize>, _, cx| {
                let line = |line: usize| {
                    let tiles = line * per_row..((line + 1) * per_row).min(count);
                    let tiles: Vec<_> = tiles.map(|row| this.render_tile(ix, row, cx)).collect();
                    h_flex().id(("tiles", line)).children(tiles)
                };
                lines.map(line).collect()
            }),
        )
        .track_scroll(&side.scroll)
        .size_full()
        .pr(SCROLLBAR);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(measure(side.area.clone(), cx.entity_id()))
            .child(tiles)
            .child(div().absolute().inset_0().child(Scrollbar::vertical(&side.scroll)))
            .into_any_element()
    }

    fn render_tile(&mut self, ix: usize, row: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let local = panel.remote().is_none() && !panel.in_archive() && !entry.is_dir;
        let path = panel.folder().join(&entry.name);
        let (name, icon) = (entry.display_name(), self.entry_icon(ix, entry, px(56.), cx));
        let picture = local.then(|| self.thumbnail(&path, cx)).flatten();
        let preview = match picture {
            Some(picture) => img(picture).max_w(px(96.)).max_h(px(96.)).object_fit(ObjectFit::Contain).into_any_element(),
            None => icon,
        };
        self.entry_base(ix, row, false, cx)
            .w(px(TILE_WIDTH))
            .h(px(TILE_HEIGHT))
            .p_1()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(div().size(px(100.)).flex().items_center().justify_center().child(preview))
            .child(div().w_full().text_xs().text_center().truncate().child(name))
    }

    /// The picture's thumbnail once made; the first ask starts making it.
    fn thumbnail(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        if let Some(made) = self.thumbnails.0.get(path) {
            return made.clone();
        }
        let still = (pc_core::media::is_still(path) || pc_core::media::is_video(path)) && pc_core::media::available();
        let readable = image::ImageFormat::from_path(path).is_ok_and(|format| format.reading_enabled()) || is_pdf(path);
        if !readable && !still || still && self.thumbnails.1 >= STILLS_AT_ONCE {
            return None;
        }
        self.thumbnails.1 += usize::from(still);
        self.thumbnails.0.insert(path.to_owned(), None);
        let (path, read) = (path.to_owned(), path.to_owned());
        let making = cx.background_executor().spawn(async move { make_thumbnail(&read) });
        cx.spawn(async move |this, cx| {
            let made = making.await;
            let _ = this.update(cx, |this, cx| {
                this.thumbnails.1 -= usize::from(still);
                if made.is_some() {
                    this.thumbnails.0.insert(path, made);
                }
                // Also for the stills waiting.
                cx.notify();
            });
        })
        .detach();
        None
    }

    /// Past the limit, the thumbnails go, their textures too.
    pub(crate) fn trim_thumbnails(&mut self, window: &mut Window) {
        if self.thumbnails.0.len() > THUMBS_KEPT {
            for image in self.thumbnails.0.drain().filter_map(|(_, made)| made) {
                let _ = window.drop_image(image);
            }
        }
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pdf")) && pc_core::pdf::available()
}

/// GPUI takes images as BGRA.
fn make_thumbnail(path: &Path) -> Option<Arc<RenderImage>> {
    if pc_core::media::is_still(path) || pc_core::media::is_video(path) {
        let frame = pc_core::media::still(path, THUMB_PIXELS).ok()?;
        let pixels = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
        return Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])));
    }
    // A PDF's first page.
    if is_pdf(path) {
        let page = *pc_core::pdf::pages(path).ok()?.first()?;
        let bitmap = pc_core::pdf::render(path, 0, pc_core::pdf::fit_width(page, THUMB_PIXELS, THUMB_PIXELS)).ok()?;
        let (width, height) = (bitmap.width, bitmap.height);
        let mut pixels = image::RgbaImage::from_raw(width, height, bitmap.into_rgba())?;
        pixels.pixels_mut().for_each(|pixel| pixel.0.swap(0, 2));
        return Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])));
    }
    let picture = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let mut pixels = picture.thumbnail(THUMB_PIXELS, THUMB_PIXELS).into_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])))
}
