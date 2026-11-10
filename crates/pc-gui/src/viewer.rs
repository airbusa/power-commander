use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis, ScrollbarHandle};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use pc_core::viewer::TextFile;

actions!(
    viewer,
    [
        Close,
        LineUp,
        LineDown,
        PageUp,
        PageDown,
        Top,
        Bottom,
        Left,
        Right,
        ShowText,
        ShowHex,
        ShowPicture
    ]
);

const CONTEXT: &str = "Viewer";
const ROW: Pixels = px(20.);
/// Rows drawn before the viewport's height is known.
const FIRST_DRAWN: usize = 60;
/// The scrollbar's height in rows: small enough for f32 pixels to stay exact.
const VIRTUAL_HEIGHT: f32 = 1_000_000.;

pub fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", Close, keys),
        KeyBinding::new("f3", Close, keys),
        KeyBinding::new("q", Close, keys),
        KeyBinding::new("up", LineUp, keys),
        KeyBinding::new("down", LineDown, keys),
        KeyBinding::new("pageup", PageUp, keys),
        KeyBinding::new("pagedown", PageDown, keys),
        KeyBinding::new("home", Top, keys),
        KeyBinding::new("end", Bottom, keys),
        KeyBinding::new("ctrl-home", Top, keys),
        KeyBinding::new("ctrl-end", Bottom, keys),
        KeyBinding::new("left", Left, keys),
        KeyBinding::new("right", Right, keys),
        // Total Commander's Lister keys.
        KeyBinding::new("1", ShowText, keys),
        KeyBinding::new("3", ShowHex, keys),
        KeyBinding::new("4", ShowPicture, keys),
    ]);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Text,
    Hex,
    Picture,
}

/// Pictures GPUI decodes open as pictures, other files as text.
pub fn open(path: PathBuf, cx: &mut App) -> io::Result<()> {
    let text = TextFile::open(&path)?;
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let picture = extension.is_some_and(|e| Img::extensions().contains(&e.as_str()));
    let title = format!("{} — Power Commander", path.display());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1000.), px(700.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Viewer::new(path, text, picture, window, cx))
    })
    .map(drop)
    .map_err(|err| io::Error::other(err.to_string()))
}

struct Viewer {
    path: PathBuf,
    text: TextFile,
    mode: Mode,
    /// GPUI can show it as a picture.
    picture: bool,
    view: ViewHandle,
    focus: FocusHandle,
    _poll: Task<()>,
}

/// Scrolls by row, not by pixel, so a file of any size scrolls exactly.
#[derive(Default)]
struct Viewport {
    top: usize,
    rows: usize,
    x: Pixels,
    /// Of the widest row drawn.
    width: Pixels,
    bounds: Bounds<Pixels>,
    /// Wheel pixels not yet a whole row.
    wheel: Pixels,
}

impl Viewport {
    fn visible(&self) -> usize {
        (self.bounds.size.height / ROW) as usize
    }

    fn max_top(&self) -> usize {
        self.rows.saturating_sub(self.visible())
    }

    fn max_x(&self) -> Pixels {
        (self.width - self.bounds.size.width).max(px(0.))
    }

    fn row_px(&self) -> f32 {
        (VIRTUAL_HEIGHT / self.rows.max(1) as f32).min(f32::from(ROW))
    }

    fn scroll_rows(&mut self, rows: isize) {
        self.top = self.top.saturating_add_signed(rows).min(self.max_top());
    }

    fn scroll_x(&mut self, by: Pixels) {
        self.x = (self.x + by).clamp(px(0.), self.max_x());
    }
}

#[derive(Clone, Default)]
struct ViewHandle(Rc<RefCell<Viewport>>);

impl ScrollbarHandle for ViewHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
    }

    fn offset(&self) -> Point<Pixels> {
        let view = self.0.borrow();
        point(-view.x, px(-(view.top as f32 * view.row_px())))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let mut view = self.0.borrow_mut();
        let row = (-f32::from(offset.y) / view.row_px()).round().max(0.) as usize;
        view.top = row.min(view.max_top());
        view.x = (-offset.x).clamp(px(0.), view.max_x());
    }

    fn content_size(&self) -> Size<Pixels> {
        let view = self.0.borrow();
        let height = view.bounds.size.height + px(view.max_top() as f32 * view.row_px());
        size(view.bounds.size.width.max(view.width), height)
    }
}

impl Viewer {
    fn new(
        path: PathBuf,
        text: TextFile,
        picture: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Shows the rows as the background index finds them.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let done = this.update(cx, |this, cx| {
                    cx.notify();
                    this.text.indexed().2
                });
                if done.unwrap_or(true) {
                    break;
                }
            }
        });
        Self {
            path,
            text,
            mode: if picture { Mode::Picture } else { Mode::Text },
            picture,
            view: ViewHandle::default(),
            focus,
            _poll: poll,
        }
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if mode == self.mode || mode == Mode::Picture && !self.picture {
            return;
        }
        self.mode = mode;
        let mut view = self.view.0.borrow_mut();
        (view.top, view.x) = (0, px(0.));
        cx.notify();
    }

    fn scroll(&mut self, by: impl FnOnce(&mut Viewport), cx: &mut Context<Self>) {
        by(&mut self.view.0.borrow_mut());
        cx.notify();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(ROW);
        self.scroll(
            |view| {
                view.wheel += delta.y;
                let rows = (view.wheel / ROW).trunc();
                view.wheel -= ROW * rows;
                view.scroll_rows(-(rows as isize));
                view.scroll_x(-delta.x);
            },
            cx,
        );
    }

    /// The rows on screen, and where they are in the file.
    fn render_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, String) {
        let theme = cx.theme();
        let (rows, scanned, done) = match self.mode {
            Mode::Hex => (self.text.hex_row_count(), self.text.len(), true),
            _ => self.text.indexed(),
        };
        let top = {
            let mut view = self.view.0.borrow_mut();
            view.rows = rows;
            view.top = view.top.min(view.max_top());
            view.top
        };
        let drawn = match self.view.0.borrow().visible() {
            0 => FIRST_DRAWN,
            visible => visible + 1,
        };
        let lines = match self.mode {
            Mode::Hex => self.text.hex_rows(top..top + drawn),
            _ => self.text.rows(top..top + drawn),
        };
        let font = font(theme.mono_font_family.clone());
        let text_system = window.text_system();
        let em = text_system
            .em_advance(text_system.resolve_font(&font), theme.mono_font_size)
            .unwrap_or(px(8.));
        let widest = lines.iter().map(|line| line.chars().count()).max();
        let x = {
            let mut view = self.view.0.borrow_mut();
            view.width = em * widest.unwrap_or(0) as f32 + px(24.);
            view.x = view.x.min(view.max_x());
            view.x
        };
        let shown = self.view.0.borrow().visible().min(rows.saturating_sub(top));
        let status = match done {
            true => format!(
                "{}–{} of {} rows",
                pc_core::grouped(top as u64 + 1),
                pc_core::grouped((top + shown) as u64),
                pc_core::grouped(rows as u64)
            ),
            false => format!(
                "{} rows so far, {}% read",
                pc_core::grouped(rows as u64),
                scanned * 100 / self.text.len().max(1)
            ),
        };
        let bounds_of = self.view.clone();
        let rows = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .font_family(theme.mono_font_family.clone())
            .text_size(theme.mono_font_size)
            .on_scroll_wheel(cx.listener(Self::wheel))
            .on_action(
                cx.listener(move |this, _: &Left, _, cx| this.scroll(|v| v.scroll_x(-em * 8.), cx)),
            )
            .on_action(
                cx.listener(move |this, _: &Right, _, cx| this.scroll(|v| v.scroll_x(em * 8.), cx)),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds_of.0.borrow_mut().bounds = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                v_flex().absolute().top_0().left(-x).pl_2().children(
                    lines
                        .into_iter()
                        .map(|line| div().h(ROW).whitespace_nowrap().child(line)),
                ),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&self.view).axis(ScrollbarAxis::Both)),
            );
        (rows.into_any_element(), status)
    }

    fn modes(&self) -> String {
        let mut modes = vec![(Mode::Text, "1 Text"), (Mode::Hex, "3 Hex")];
        if self.picture {
            modes.push((Mode::Picture, "4 Picture"));
        }
        let names = modes
            .into_iter()
            .map(|(mode, name)| match mode == self.mode {
                true => format!("[{name}]"),
                false => name.to_owned(),
            });
        names.collect::<Vec<_>>().join("  ")
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (content, status) = match self.mode {
            Mode::Picture => {
                let picture = img(self.path.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .with_loading(|| div().p_4().child("Loading…").into_any_element())
                    .with_fallback(|| {
                        div()
                            .p_4()
                            .child("This picture cannot be shown; 1 shows it as text.")
                            .into_any_element()
                    });
                (
                    div().size_full().p_2().child(picture).into_any_element(),
                    String::new(),
                )
            }
            _ => self.render_rows(window, cx),
        };
        let theme = cx.theme();
        v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|_, _: &Close, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &ShowText, _, cx| this.set_mode(Mode::Text, cx)))
            .on_action(cx.listener(|this, _: &ShowHex, _, cx| this.set_mode(Mode::Hex, cx)))
            .on_action(cx.listener(|this, _: &ShowPicture, _, cx| this.set_mode(Mode::Picture, cx)))
            .on_action(
                cx.listener(|this, _: &LineUp, _, cx| this.scroll(|v| v.scroll_rows(-1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &LineDown, _, cx| this.scroll(|v| v.scroll_rows(1), cx)),
            )
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                this.scroll(|v| v.scroll_rows(-(v.visible().max(2) as isize - 1)), cx)
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                this.scroll(|v| v.scroll_rows(v.visible().max(2) as isize - 1), cx)
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| this.scroll(|v| v.top = 0, cx)))
            .on_action(
                cx.listener(|this, _: &Bottom, _, cx| this.scroll(|v| v.top = v.max_top(), cx)),
            )
            .child(div().flex_1().min_h_0().child(content))
            .child(status_bar(
                &self.path,
                self.text.len(),
                status,
                self.modes(),
                cx,
            ))
    }
}

fn status_bar(path: &Path, len: u64, status: String, modes: String, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .px_2()
        .gap_4()
        .border_t_1()
        .border_color(theme.border)
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(path.display().to_string()),
        )
        .child(format!("{} bytes", pc_core::grouped(len)))
        .child(status)
        .child(modes)
}
