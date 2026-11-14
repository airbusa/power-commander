//! VS Code's minimap, as in Water Data Studio: the text in miniature, two
//! pixels a row, the part in view marked. A click or a drag scrolls.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

/// Characters a row shows, a pixel each.
pub const COLUMNS: usize = 110;
const WIDTH: f32 = COLUMNS as f32;
const ROW: f32 = 2.;
/// Columns a tab takes.
const TAB: usize = 4;

/// The view a minimap goes beside.
pub trait Source: 'static {
    /// None before the view is laid out.
    fn position(&self, height: f32, cx: &App) -> Option<Position>;
    fn lines(&self, rows: Range<usize>, cx: &App) -> Vec<String>;
    /// Scrolls so `first` is the first row in view.
    fn scroll_to(&self, first: f32, cx: &mut App);
    fn wheel(&self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App);
}

pub struct Position {
    /// The first row in view; fractions as the view scrolls by pixel.
    pub first: f32,
    pub visible: f32,
    /// How far `first` goes.
    pub max_first: f32,
}

pub struct Minimap {
    source: Rc<dyn Source>,
    /// Where in the marked part the pointer holds it, while dragging.
    grab: Option<f32>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

/// Where the view is, in minimap pixels.
struct Geometry {
    max_first: f32,
    /// How far the minimap itself is scrolled.
    offset: f32,
    slider_top: f32,
    slider_height: f32,
    height: f32,
    fits: bool,
}

impl Minimap {
    pub fn new(source: impl Source) -> Self {
        Self {
            source: Rc::new(source),
            grab: None,
            bounds: Rc::default(),
        }
    }

    fn geometry(&self, cx: &App) -> Option<Geometry> {
        let height = f32::from(self.bounds.get().size.height);
        let Position {
            first,
            visible,
            max_first,
        } = self.source.position(height, cx)?;
        Some(geometry(first, visible, max_first, height))
    }

    /// Scrolls the view so the marked part starts at `top`.
    fn scroll_to(&mut self, top: f32, cx: &mut Context<Self>) {
        let Some(g) = self.geometry(cx) else {
            return;
        };
        let first = match g.fits {
            true => top / ROW,
            false => top / (g.height - g.slider_height).max(1.) * g.max_first,
        };
        self.source.scroll_to(first.clamp(0., g.max_first), cx);
    }

    fn local_y(&self, position: Point<Pixels>) -> f32 {
        f32::from(position.y - self.bounds.get().origin.y)
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(g) = self.geometry(cx) else {
            return;
        };
        let y = self.local_y(event.position);
        let grab = match y >= g.slider_top && y <= g.slider_top + g.slider_height {
            true => y - g.slider_top,
            false => g.slider_height / 2.,
        };
        self.grab = Some(grab);
        self.scroll_to(y - grab, cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        match (self.grab, event.pressed_button) {
            (Some(grab), Some(MouseButton::Left)) => {
                let y = self.local_y(event.position);
                self.scroll_to(y - grab, cx);
            }
            _ => self.grab = None,
        }
    }
}

fn geometry(first: f32, visible: f32, max_first: f32, height: f32) -> Geometry {
    let slider_height = visible * ROW;
    // All the view scrolls through fits.
    let fits = (max_first + visible) * ROW <= height;
    // Else the marked part moves as far through the minimap as the view
    // through the text, and the minimap scrolls under it.
    let slider_top = match fits {
        true => first * ROW,
        false => (first / max_first.max(1.)).clamp(0., 1.) * (height - slider_height),
    };
    Geometry {
        max_first,
        offset: (first * ROW - slider_top).max(0.),
        slider_top,
        slider_height,
        height,
        fits,
    }
}

/// A line's runs of characters: start column and length.
fn blocks(line: &str) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut column = 0;
    for c in line.chars() {
        if column as f32 >= WIDTH {
            break;
        }
        if !c.is_whitespace() {
            match blocks.last_mut() {
                Some((start, len)) if *start + *len == column => *len += 1,
                _ => blocks.push((column, 1)),
            }
        }
        column += if c == '\t' { TAB } else { 1 };
    }
    blocks
}

impl Render for Minimap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let ink = theme.foreground.opacity(0.45);
        let slider = theme
            .foreground
            .opacity(if self.grab.is_some() { 0.28 } else { 0.16 });
        let bounds = self.bounds.clone();
        let view = cx.entity().downgrade();
        div()
            .id("minimap")
            .w(px(WIDTH))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.grab = None),
            )
            .on_scroll_wheel(
                cx.listener(|this, event, window, cx| this.source.wheel(event, window, cx)),
            )
            .child(
                canvas(
                    move |area, _, _| bounds.set(area),
                    move |area, _, window, cx| {
                        let Some(map) = view.upgrade() else {
                            return;
                        };
                        let map = map.read(cx);
                        let Some(g) = map.geometry(cx) else {
                            return;
                        };
                        let first = (g.offset / ROW) as usize;
                        let last = ((g.offset + f32::from(area.size.height)) / ROW) as usize + 1;
                        let lines = map.source.lines(first..last, cx);
                        window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                            for (row, line) in (first..).zip(&lines) {
                                let y = area.origin.y + px(row as f32 * ROW - g.offset);
                                for (column, len) in blocks(line) {
                                    let origin = point(area.origin.x + px(column as f32), y);
                                    let size = size(px(len as f32), px(ROW));
                                    window.paint_quad(fill(Bounds { origin, size }, ink));
                                }
                            }
                            let top = area.origin.y + px(g.slider_top);
                            let size = size(area.size.width, px(g.slider_height));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(area.origin.x, top),
                                    size,
                                },
                                slider,
                            ));
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` macro would replace `#[test]`.
    use super::{blocks, geometry};

    #[test]
    fn marks_the_part_in_view() {
        // 100 rows fit in 300 pixels: the minimap does not scroll.
        let g = geometry(10., 20., 80., 300.);
        assert!(g.fits);
        assert_eq!((g.slider_top, g.slider_height, g.offset), (20., 40., 0.));
        // A million rows: the marked part halfway, the minimap under it.
        let g = geometry(500_000., 20., 1_000_000., 300.);
        assert!(!g.fits);
        assert_eq!(g.slider_top, 130.);
        assert_eq!(g.offset, 1_000_000. - 130.);
        assert_eq!(blocks("ab  c\td"), [(0, 2), (4, 1), (9, 1)]);
    }
}
