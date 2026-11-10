//! PDF pages, rendered by PDFium as they scroll into view, as wide as the
//! window; beside them the minimap's page thumbnails.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::pdf::{self, PageSize};

const GAP: Pixels = px(12.);
/// Pages kept rendered on each side of the one on screen.
const KEPT: usize = 6;
/// The minimap: as wide as the text one, thumbnails inside its margins.
const STRIP: f32 = 110.;
const THUMB: f32 = 98.;
const THUMB_GAP: f32 = 4.;
/// Thumbnails kept beyond the strip's visible ones.
const THUMBS_KEPT: usize = 60;

/// Rendered pages and thumbnails, and the ones on their way.
#[derive(Default)]
struct Pages {
    /// The image, and the pixel width it was rendered at.
    images: HashMap<usize, (u32, Arc<RenderImage>)>,
    thumbs: HashMap<usize, Arc<RenderImage>>,
    pending: HashSet<(usize, u32)>,
}

pub struct PdfView {
    path: PathBuf,
    sizes: Rc<Vec<PageSize>>,
    list: ListState,
    pages: Rc<RefCell<Pages>>,
    /// The width pages are shown at, from the last layout.
    width: Rc<Cell<Pixels>>,
    strip: Rc<Cell<Bounds<Pixels>>>,
    error: Option<String>,
}

impl PdfView {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let read = path.clone();
        cx.spawn(async move |this, cx| {
            let sizes = cx
                .background_executor()
                .spawn(async move { pdf::pages(&read) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match sizes {
                    Ok(sizes) => {
                        this.list.reset(sizes.len());
                        this.sizes = Rc::new(sizes);
                    }
                    Err(err) => this.error = Some(err.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            path,
            sizes: Rc::default(),
            list: ListState::new(0, ListAlignment::Top, px(800.)),
            pages: Rc::default(),
            width: Rc::new(Cell::new(px(800.))),
            strip: Rc::default(),
            error: None,
        }
    }

    /// `Page 3 / 12`, the topmost page shown.
    pub fn status(&self) -> String {
        match (self.sizes.len(), &self.error) {
            (_, Some(_)) => String::new(),
            (0, None) => "Loading...".into(),
            (pages, None) => format!(
                "Page {} / {pages}",
                self.list.logical_scroll_top().item_ix + 1
            ),
        }
    }

    pub fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.list.scroll_by(distance);
        cx.notify();
    }

    /// The first page, or the last.
    pub fn scroll_to_end(&mut self, end: bool, cx: &mut Context<Self>) {
        let item_ix = match end {
            true => self.sizes.len().saturating_sub(1),
            false => 0,
        };
        self.list.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    /// The viewport's height, for Page Up and Down.
    pub fn page_height(&self) -> Pixels {
        self.list.viewport_bounds().size.height
    }

    fn ratio(&self, page: usize) -> f32 {
        let size = self.sizes[page];
        size.height / size.width.max(1.)
    }

    /// A page's height in the list, its gap below included.
    fn main_height(&self, page: usize) -> f32 {
        f32::from(self.width.get()) * self.ratio(page) + f32::from(GAP)
    }

    fn thumb_height(&self, page: usize) -> f32 {
        THUMB * self.ratio(page) + THUMB_GAP
    }

    /// Where the strip shows each page; the last is the total height.
    fn thumb_tops(&self) -> Vec<f32> {
        let mut tops = Vec::with_capacity(self.sizes.len() + 1);
        let mut top = 0.;
        for page in 0..self.sizes.len() {
            tops.push(top);
            top += self.thumb_height(page);
        }
        tops.push(top);
        tops
    }

    /// The strip's scroll, and the marked part's top and height.
    fn strip_geometry(&self, tops: &[f32]) -> (f32, f32, f32) {
        let scroll = self.list.logical_scroll_top();
        let page = scroll.item_ix.min(self.sizes.len().saturating_sub(1));
        let scale = self.thumb_height(page) / self.main_height(page);
        let marked = tops[page] + f32::from(scroll.offset_in_item) * scale;
        let height = f32::from(self.page_height()) * THUMB / f32::from(self.width.get()).max(1.);
        let (total, shown) = (
            tops[tops.len() - 1],
            f32::from(self.strip.get().size.height),
        );
        // Longer than the strip: it scrolls as far through as the pages do.
        let offset = match total > shown {
            true => marked / (total - height).max(1.) * (total - shown),
            false => 0.,
        };
        (offset.clamp(0., (total - shown).max(0.)), marked, height)
    }

    /// A click or drag on the strip: the pages there come into view.
    fn jump(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.sizes.is_empty() {
            return;
        }
        let tops = self.thumb_tops();
        let (offset, _, height) = self.strip_geometry(&tops);
        let y = f32::from(position.y - self.strip.get().origin.y) + offset - height / 2.;
        let y = y.clamp(0., tops[tops.len() - 1]);
        let page = tops
            .partition_point(|&top| top <= y)
            .saturating_sub(1)
            .min(self.sizes.len() - 1);
        let into = (y - tops[page]) * self.main_height(page) / self.thumb_height(page);
        self.list.scroll_to(ListOffset {
            item_ix: page,
            offset_in_item: px(into),
        });
        cx.notify();
    }

    fn render_strip(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tops = self.thumb_tops();
        let (offset, marked, height) = self.strip_geometry(&tops);
        let shown = f32::from(self.strip.get().size.height);
        let pixels = (THUMB * window.scale_factor()).round() as u32;
        let (view, path) = (cx.entity().downgrade(), &self.path);
        let visible: Vec<usize> = (0..self.sizes.len())
            .filter(|&page| tops[page + 1] > offset && tops[page] < offset + shown)
            .collect();
        let mut thumbs = Vec::new();
        for &page in &visible {
            let image = self.pages.borrow().thumbs.get(&page).cloned();
            if image.is_none() {
                request(&view, &self.pages, path, page, pixels, Kind::Thumb, cx);
            }
            let box_height = self.thumb_height(page) - THUMB_GAP;
            let thumb = div()
                .absolute()
                .left(px((STRIP - THUMB) / 2.))
                .top(px(tops[page] - offset))
                .w(px(THUMB))
                .h(px(box_height))
                .bg(gpui_kit::white())
                .when_some(image, |el, image| el.child(img(image).size_full()));
            thumbs.push(thumb);
        }
        // Far thumbnails give their memory back.
        if let (Some(&first), Some(&last)) = (visible.first(), visible.last()) {
            let mut pages = self.pages.borrow_mut();
            if pages.thumbs.len() > THUMBS_KEPT {
                pages.thumbs.retain(|&page, _| {
                    page + THUMBS_KEPT / 2 >= first && page <= last + THUMBS_KEPT / 2
                });
            }
        }
        let strip = self.strip.clone();
        let theme = cx.theme();
        let slider = theme.foreground.opacity(0.16);
        div()
            .id("pdf-minimap")
            .w(px(STRIP))
            .h_full()
            .flex_none()
            .relative()
            .overflow_hidden()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.jump(event.position, cx)),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.jump(event.position, cx);
                }
            }))
            .child(
                canvas(move |bounds, _, _| strip.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(thumbs)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .top(px(marked - offset))
                    .h(px(height))
                    .bg(slider),
            )
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Page,
    Thumb,
}

/// Starts rendering `page` at `pixels` wide, unless it is on its way.
fn request(
    view: &WeakEntity<PdfView>,
    pages: &Rc<RefCell<Pages>>,
    path: &Path,
    page: usize,
    pixels: u32,
    kind: Kind,
    cx: &mut App,
) {
    if !pages.borrow_mut().pending.insert((page, pixels)) {
        return;
    }
    let (view, path) = (view.clone(), path.to_owned());
    cx.spawn(async move |cx| {
        let render = cx
            .background_executor()
            .spawn(async move { pdf::render(&path, page, pixels) });
        let image = render.await.ok().and_then(|bitmap| {
            let buffer = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.bgra)?;
            Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
        });
        let _ = view.update(cx, |this, cx| {
            let mut pages = this.pages.borrow_mut();
            pages.pending.remove(&(page, pixels));
            match (kind, image) {
                (Kind::Page, Some(image)) => {
                    pages.images.insert(page, (pixels, image));
                    // Far pages give their memory back.
                    let top = this.list.logical_scroll_top().item_ix;
                    pages
                        .images
                        .retain(|&kept, _| kept + KEPT >= top && kept <= top + 2 * KEPT);
                }
                (Kind::Thumb, Some(image)) => {
                    pages.thumbs.insert(page, image);
                }
                (_, None) => {}
            }
            cx.notify();
        });
    })
    .detach();
}

impl Render for PdfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        if let Some(error) = &self.error {
            return div()
                .p_4()
                .child(format!("This PDF cannot be shown: {error}"))
                .into_any_element();
        }
        let (sizes, pages, width) = (self.sizes.clone(), self.pages.clone(), self.width.clone());
        let (view, path, scale) = (
            cx.entity().downgrade(),
            self.path.clone(),
            window.scale_factor(),
        );
        let paper = theme.muted;
        let list = list(self.list.clone(), move |ix, _, cx| {
            let size = sizes[ix];
            let shown = width.get();
            let height = shown * (size.height / size.width.max(1.));
            let pixels = (f32::from(shown) * scale).round().max(1.) as u32;
            let image = pages.borrow().images.get(&ix).cloned();
            // A page rendered at another width shows scaled until its new rendering comes.
            if image
                .as_ref()
                .is_none_or(|(at, _)| at.abs_diff(pixels) > pixels / 20)
            {
                request(&view, &pages, &path, ix, pixels, Kind::Page, cx);
            }
            let page = div().w(shown).h(height).bg(gpui_kit::white());
            let page = match image {
                Some((_, image)) => page.child(img(image).size_full()),
                None => page,
            };
            div()
                .w_full()
                .flex()
                .justify_center()
                .pb(GAP)
                .child(page)
                .into_any_element()
        })
        .size_full();
        let width = self.width.clone();
        let pages = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .bg(paper)
            .pt(GAP)
            .child(
                canvas(
                    move |bounds, _, _| width.set((bounds.size.width - GAP * 2.).max(px(100.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(list)
            .child(div().absolute().inset_0().child(Scrollbar::new(&self.list)));
        let strip = (crate::minimap_shown(cx) && !self.sizes.is_empty())
            .then(|| self.render_strip(window, cx));
        h_flex()
            .size_full()
            .child(pages)
            .children(strip)
            .into_any_element()
    }
}
