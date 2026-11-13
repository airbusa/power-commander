use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, MenuBuilder};
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::input::Rope;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis, ScrollbarHandle};
use gpui_kit::component::text::TextView;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::{codepage, ebook, resume};
use pc_core::settings::ViewerSettings;
use pc_core::menu::Command;
use pc_core::viewer::{
    COPY_LIMIT, Font, Layout, MARKDOWN_LIMIT, Mode, Query, Row, Searches, Shown, TextFile, search_url,
};

use crate::find_dialog;
use crate::menus::{Entries, Run};
use crate::minimap::{self, Minimap};
use crate::media_view::MediaView;
use crate::pdf_view::PdfView;

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
        ShowBinary,
        ShowHex,
        ShowPicture,
        ShowHtml,
        ShowUnicode,
        ShowUtf8,
        ShowMarkdown,
        ShowPdf,
        ShowBook,
        FontAnsi,
        FontAscii,
        FontVariable,
        ToggleWrap,
        FitImage,
        FitLarger,
        NextFile,
        PreviousFile,
        Reload,
        OpenFile,
        SaveAs,
        Configure,
        ToggleMinimap,
        ToggleSyntax,
        PlayPause,
        ToggleFullScreen,
        CopyText,
        CopyHex,
        SearchGoogle,
        SelectAll,
        Find,
        FindNext,
        FindPrevious,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);

/// Zoom's steps, as browsers have them.
const ZOOMS: [f32; 17] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5, 3., 4., 5.];
/// A touchpad's scrolling makes a zoom step every this much.
const WHEEL_STEP: Pixels = px(40.);

/// A code page of the Encoding menu; none goes by the font.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = viewer, no_json)]
struct SetPage(Option<u16>);

const CONTEXT: &str = "Viewer";
const ROW: Pixels = px(20.);
/// Left of the rows' text.
const PAD: Pixels = px(8.);
/// Rows drawn before the viewport's height is known.
const FIRST_DRAWN: usize = 60;
/// The scrollbar's height in rows: small enough for f32 pixels to stay exact.
const VIRTUAL_HEIGHT: f32 = 1_000_000.;
/// Arrow keys move PDF pages by this much.
const PDF_LINE: Pixels = px(48.);
/// DOS text draws boxes and blocks, which join only in rows as high as their characters.
const DOS_PAGES: [u16; 4] = [437, 850, 852, 866];

pub fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", Close, keys),
        // Lister's F3 is Find next, as F5; earlier bound, so menus show F5.
        KeyBinding::new("f3", FindNext, keys),
        KeyBinding::new("shift-f3", FindPrevious, keys),
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
        KeyBinding::new("2", ShowBinary, keys),
        KeyBinding::new("3", ShowHex, keys),
        KeyBinding::new("4", ShowPicture, keys),
        KeyBinding::new("5", ShowHtml, keys),
        KeyBinding::new("6", ShowUnicode, keys),
        KeyBinding::new("7", ShowUtf8, keys),
        KeyBinding::new("8", ShowMarkdown, keys),
        KeyBinding::new("9", ShowPdf, keys),
        KeyBinding::new("0", ShowBook, keys),
        KeyBinding::new("a", FontAnsi, keys),
        KeyBinding::new("s", FontAscii, keys),
        KeyBinding::new("v", FontVariable, keys),
        KeyBinding::new("w", ToggleWrap, keys),
        KeyBinding::new("f", FitImage, keys),
        KeyBinding::new("l", FitLarger, keys),
        KeyBinding::new("m", ToggleMinimap, keys),
        KeyBinding::new("c", ToggleSyntax, keys),
        KeyBinding::new("space", PlayPause, keys),
        KeyBinding::new("f11", ToggleFullScreen, keys),
        KeyBinding::new("n", NextFile, keys),
        KeyBinding::new("p", PreviousFile, keys),
        KeyBinding::new("f2", Reload, keys),
        KeyBinding::new("secondary-c", CopyText, keys),
        KeyBinding::new("secondary-shift-c", CopyHex, keys),
        KeyBinding::new("secondary-a", SelectAll, keys),
        KeyBinding::new("f7", Find, keys),
        KeyBinding::new("f5", FindNext, keys),
        KeyBinding::new("shift-f5", FindPrevious, keys),
        // + is Shift and = on most keyboards; the keypad's too.
        KeyBinding::new("secondary-=", ZoomIn, keys),
        KeyBinding::new("secondary-+", ZoomIn, keys),
        KeyBinding::new("secondary-shift-=", ZoomIn, keys),
        KeyBinding::new("secondary-add", ZoomIn, keys),
        KeyBinding::new("secondary--", ZoomOut, keys),
        KeyBinding::new("secondary-subtract", ZoomOut, keys),
        KeyBinding::new("secondary-0", ZoomReset, keys),
    ]);
}

/// Shows `files[at]`; Next and Previous go through `files`. `bounds` are
/// the main window's, which the viewer covers.
pub fn open(
    files: Vec<PathBuf>,
    at: usize,
    settings: ViewerSettings,
    over: crate::Over,
    cx: &mut App,
) -> io::Result<()> {
    open_with(files, at, settings, over, false, cx)
}

/// As `open`, at the first match of the last Find: Find Files' text.
pub(crate) fn open_finding(
    files: Vec<PathBuf>,
    at: usize,
    settings: ViewerSettings,
    over: crate::Over,
    cx: &mut App,
) -> io::Result<()> {
    open_with(files, at, settings, over, true, cx)
}

fn open_with(
    files: Vec<PathBuf>,
    at: usize,
    settings: ViewerSettings,
    over: crate::Over,
    find: bool,
    cx: &mut App,
) -> io::Result<()> {
    let mut shown = Shown::new(files, at, settings, Img::extensions(), true, true);
    let text = shown.start()?;
    let options = crate::over_main_window(title(shown.path()), over, cx);
    gpui_kit::open_window(options, cx, |window, cx| {
        let viewer = cx.new(|cx| Viewer::new(shown, text, false, window, cx));
        if find {
            viewer.update(cx, |viewer, cx| viewer.find_next(false, window, cx));
        }
        viewer
    })
    .map(drop)
    .map_err(|err| io::Error::other(err.to_string()))
}

fn title(path: &Path) -> String {
    format!("{} — Viewer", path.display())
}

/// Quick View: the viewer in a panel, showing what the other panel's cursor
/// is on; no menu bar, no status bar, media paused.
pub(crate) struct Quick(Entity<Viewer>);

impl Quick {
    pub fn new(path: PathBuf, settings: ViewerSettings, window: &mut Window, cx: &mut App) -> Self {
        let mut shown = Shown::new(vec![path], 0, settings, Img::extensions(), true, true);
        let started = shown.start();
        let viewer = cx.new(|cx| {
            let empty = || TextFile::text(String::new(), Layout::Lines { width: 1 });
            let (text, problem) = match started {
                Ok(text) => (text, None),
                Err(err) => (empty(), Some(err)),
            };
            let mut viewer = Viewer::new(shown, text, true, window, cx);
            if let Some(err) = problem {
                viewer.failed(err, window, cx);
            }
            viewer
        });
        Quick(viewer)
    }

    /// Frees what it drew into the main window, before it goes.
    pub fn release(&self, window: &mut Window, cx: &mut App) {
        self.0.update(cx, |viewer, _| {
            let still = viewer.still.take().and_then(|(_, image)| image.ok());
            for image in viewer.stale.drain(..).chain(still) {
                let _ = window.drop_image(image);
            }
        });
    }

    /// Shows `path` instead, as the viewer starts on a file.
    pub fn show(&self, path: PathBuf, window: &mut Window, cx: &mut App) {
        self.0.update(cx, |viewer, cx| {
            let settings = viewer.shown.settings.clone();
            viewer.shown = Shown::new(vec![path], 0, settings, Img::extensions(), true, true);
            viewer.restart(window, cx);
        });
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.0.read(cx).focus.clone()
    }

    pub fn view(&self) -> Entity<impl Render> {
        self.0.clone()
    }
}

/// The searches, shared by the viewers.
struct LastSearches(Searches);

impl Global for LastSearches {}

/// Find Files' text becomes the last Find: F3 in the viewer finds it.
pub(crate) fn remember_find(text: &pc_core::search::Text, cx: &mut App) {
    let query = Query {
        text: text.text.clone(),
        whole_words: text.whole_words,
        case_sensitive: text.case_sensitive,
        backwards: false,
        hex: text.hex,
        regex: text.regex,
    };
    searches(cx).add(query);
}

/// Read from disk at the first Find.
fn searches(cx: &mut App) -> &mut Searches {
    if !cx.has_global::<LastSearches>() {
        cx.set_global(LastSearches(Searches::load().unwrap_or_default()));
    }
    &mut cx.global_mut::<LastSearches>().0
}

struct Viewer {
    shown: Shown,
    /// Quick View's: inside a panel.
    quick: bool,
    /// Why Quick View shows nothing: it says so in place of the file.
    problem: Option<SharedString>,
    /// Text, pictures, PDF and Markdown show this times as large; pictures
    /// and PDF pages as they fit.
    zoom: f32,
    /// A touchpad's scrolling with Ctrl, toward the next zoom step.
    wheel: Pixels,
    /// The picture's view, measured; a zoomed picture pans in it.
    picture_area: Rc<Cell<Size<Pixels>>>,
    picture_scroll: ScrollHandle,
    menu_bar: Entity<AppMenuBar>,
    /// Video or sound being played.
    media: Option<Entity<MediaView>>,
    /// Shared with a running Find.
    text: Arc<TextFile>,
    /// The file's, HTML's text is shorter.
    size: u64,
    /// The source the Markdown mode renders.
    markdown: Option<SharedString>,
    /// The Book mode's book as HTML, or why it cannot be read.
    book: Option<Result<SharedString, String>>,
    /// The book's text as its minimap shows it.
    book_map: Option<SharedString>,
    /// The book's fingerprint, by which its place is kept (pc_core::resume).
    book_print: Option<String>,
    /// Where the book was left, until scrolled there once it is laid out.
    book_resume: Option<f64>,
    /// "Continuing at 40%", for a moment.
    book_continued: Option<f64>,
    /// The PDF mode's pages.
    pdf: Option<Entity<PdfView>>,
    /// An office document being converted for them: the tool, or why it failed.
    converting: Option<Result<&'static str, String>>,
    _converting: Option<Task<()>>,
    /// A HEIC's or AVIF's picture, drawn by libmpv in the background, or why not.
    still: Option<(PathBuf, Result<Arc<RenderImage>, String>)>,
    _still: Option<Task<()>>,
    /// Pictures no longer shown, freed at the next frame.
    stale: Vec<Arc<RenderImage>>,
    fit: bool,
    only_larger: bool,
    /// Anchor and head, as file offsets: what Copy copies.
    selection: (u64, u64),
    /// The left button is down for a selection.
    selecting: bool,
    /// A running Find; setting it stops the search.
    finding: Option<Arc<AtomicBool>>,
    minimap: Entity<Minimap>,
    /// Code in its language's colours, as the settings say; the file parsed.
    colours: bool,
    syntax: Option<Arc<SyntaxHighlighter>>,
    _syntax: Option<Task<()>>,
    /// The Markdown mode's: its source beside the rendered page.
    markdown_minimap: Entity<Minimap>,
    markdown_scroll: ScrollHandle,
    view: ViewHandle,
    focus: FocusHandle,
    _poll: Task<()>,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.stop_finding();
        self.leave_book();
    }
}

/// The minimap's view of the rows.
const MENUS: [&str; 4] = ["File", "Edit", "Options", "Encoding"];

/// File, Edit, Options and Encoding, as Lister's; each built as it opens.
fn menu_bar(viewer: WeakEntity<Viewer>, cx: &mut App) -> Entity<AppMenuBar> {
    let menus = MENUS.map(|name| {
        let viewer = viewer.clone();
        let build: MenuBuilder = Rc::new(move |menu, _, cx| match viewer.upgrade() {
            Some(viewer) => {
                let viewer = viewer.read(cx);
                viewer.menu(name, menu.action_context(viewer.focus.clone()), cx)
            }
            None => menu,
        });
        (name.into(), build)
    });
    AppMenuBar::built(menus.into(), cx)
}

/// How far a rendered page scrolls.
#[derive(Clone, Copy)]
enum PageScroll {
    Lines(f32),
    Pages(f32),
    Top,
    Bottom,
}

struct ViewerMap(WeakEntity<Viewer>);

/// The minimap's view of a rendered Markdown page or book: its source lines
/// or the book's text, scrolled as far through as the page.
struct MarkdownMap(WeakEntity<Viewer>);

impl Viewer {
    /// What the rendered page's minimap shows.
    fn page_text(&self) -> Option<&SharedString> {
        match self.shown.mode {
            Mode::Book => self.book_map.as_ref(),
            _ => self.markdown.as_ref(),
        }
    }
}

impl MarkdownMap {
    /// Source lines, and the page's scroll, visible and scrollable heights.
    fn page(&self, cx: &App) -> Option<(usize, f32, f32, f32)> {
        let viewer = self.0.upgrade()?.read(cx);
        let lines = viewer.page_text()?.lines().count().max(1);
        let scroll = &viewer.markdown_scroll;
        let shown = f32::from(scroll.bounds().size.height);
        let max = f32::from(scroll.max_offset().y);
        Some((lines, -f32::from(scroll.offset().y), shown, max))
    }
}

impl minimap::Source for MarkdownMap {
    fn position(&self, _: f32, cx: &App) -> Option<minimap::Position> {
        let (lines, scrolled, shown, max) = self.page(cx)?;
        let total = (shown + max).max(1.);
        let visible = lines as f32 * shown / total;
        Some(minimap::Position {
            first: lines as f32 * scrolled / total,
            visible,
            max_first: (lines as f32 - visible).max(0.),
        })
    }

    fn lines(&self, rows: std::ops::Range<usize>, cx: &App) -> Vec<String> {
        let Some(source) = self
            .0
            .upgrade()
            .and_then(|viewer| viewer.read(cx).page_text().cloned())
        else {
            return Vec::new();
        };
        let lines = source.lines().skip(rows.start).take(rows.len());
        lines
            .map(|line| line.chars().take(minimap::COLUMNS).collect())
            .collect()
    }

    fn scroll_to(&self, first: f32, cx: &mut App) {
        let Some((lines, _, shown, max)) = self.page(cx) else {
            return;
        };
        let y = (first / lines as f32 * (shown + max)).clamp(0., max);
        if let Some(viewer) = self.0.upgrade() {
            viewer.update(cx, |this, cx| {
                this.markdown_scroll.set_offset(point(px(0.), px(-y)));
                cx.notify();
            });
        }
    }

    fn wheel(&self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut App) {
        if let Some(viewer) = self.0.upgrade() {
            viewer.update(cx, |this, cx| {
                let scroll = &this.markdown_scroll;
                let (offset, max) = (scroll.offset(), scroll.max_offset());
                let y = (offset.y + event.delta.pixel_delta(ROW).y).clamp(-max.y, px(0.));
                scroll.set_offset(point(offset.x, y));
                cx.notify();
            });
        }
    }
}

impl minimap::Source for ViewerMap {
    fn position(&self, _: f32, cx: &App) -> Option<minimap::Position> {
        let viewer = self.0.upgrade()?;
        let view = viewer.read(cx).view.0.borrow();
        Some(minimap::Position {
            first: view.top as f32,
            visible: view.visible() as f32,
            max_first: view.max_top() as f32,
        })
    }

    fn lines(&self, rows: std::ops::Range<usize>, cx: &App) -> Vec<String> {
        let Some(viewer) = self.0.upgrade() else {
            return Vec::new();
        };
        viewer.read(cx).text.sketch(rows, minimap::COLUMNS)
    }

    fn scroll_to(&self, first: f32, cx: &mut App) {
        if let Some(viewer) = self.0.upgrade() {
            viewer.update(cx, |this, cx| {
                this.scroll(
                    |view| view.top = (first.round() as usize).min(view.max_top()),
                    cx,
                )
            });
        }
    }

    fn wheel(&self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
        if let Some(viewer) = self.0.upgrade() {
            viewer.update(cx, |this, cx| this.wheel(event, window, cx));
        }
    }
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
    /// From `top`, as drawn last: where clicks land.
    drawn: Vec<Row>,
    font: gpui_kit::Font,
    font_size: Pixels,
    /// A Find result to scroll to, once the index reaches it.
    reveal: Option<u64>,
    /// Set as rows are drawn; `ROW` before.
    row_height: Option<Pixels>,
}

impl Viewport {
    fn row(&self) -> Pixels {
        self.row_height.unwrap_or(ROW)
    }

    fn visible(&self) -> usize {
        (self.bounds.size.height / self.row()) as usize
    }

    fn max_top(&self) -> usize {
        self.rows.saturating_sub(self.visible())
    }

    fn max_x(&self) -> Pixels {
        (self.width - self.bounds.size.width).max(px(0.))
    }

    fn row_px(&self) -> f32 {
        (VIRTUAL_HEIGHT / self.rows.max(1) as f32).min(f32::from(self.row()))
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
    fn new(shown: Shown, text: TextFile, quick: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        if !quick {
            window.focus(&focus, cx);
        }
        let mut viewer = Self {
            quick,
            problem: None,
            zoom: 1.,
            wheel: px(0.),
            picture_area: Rc::default(),
            picture_scroll: ScrollHandle::new(),
            menu_bar: menu_bar(cx.entity().downgrade(), cx),
            media: media_view(&shown, None, quick, cx),
            size: file_size(&shown, &text),
            markdown: markdown(&shown, &text),
            book: None,
            book_map: None,
            book_print: None,
            book_resume: None,
            book_continued: None,
            pdf: pdf_view(&shown, cx),
            converting: None,
            _converting: None,
            still: None,
            _still: None,
            stale: Vec::new(),
            shown,
            text: Arc::new(text),
            fit: true,
            only_larger: false,
            selection: (0, 0),
            selecting: false,
            finding: None,
            minimap: {
                let viewer = ViewerMap(cx.entity().downgrade());
                cx.new(|_| Minimap::new(viewer))
            },
            markdown_minimap: {
                let viewer = MarkdownMap(cx.entity().downgrade());
                cx.new(|_| Minimap::new(viewer))
            },
            markdown_scroll: ScrollHandle::new(),
            view: ViewHandle::default(),
            focus,
            _poll: Self::poll(cx),
            colours: false,
            syntax: None,
            _syntax: None,
        };
        viewer.colours = viewer.shown.settings.syntax;
        viewer.convert(cx);
        viewer.decode_still(cx);
        viewer.open_book();
        viewer.parse(cx);
        viewer
    }

    /// The file parsed in the background for its colours, where it has a
    /// language and `pc_core::viewer::source` takes it.
    fn parse(&mut self, cx: &mut Context<Self>) {
        (self.syntax, self._syntax) = (None, None);
        if !self.colours {
            return;
        }
        let (path, mode) = (self.shown.path().to_owned(), self.shown.mode);
        let parsed = cx.background_executor().spawn(async move {
            let (language, text) = pc_core::viewer::source(&path, mode)?;
            let mut highlighter = SyntaxHighlighter::new(language);
            highlighter.update(None, &Rope::from(text.as_str()), None);
            Some(highlighter)
        });
        self._syntax = Some(cx.spawn(async move |this, cx| {
            if let Some(highlighter) = parsed.await {
                let _ = this.update(cx, |this, cx| {
                    this.syntax = Some(Arc::new(highlighter));
                    cx.notify();
                });
            }
        }));
    }

    fn toggle_syntax(&mut self, cx: &mut Context<Self>) {
        self.colours = !self.colours;
        self.parse(cx);
        let colours = self.colours;
        crate::change_settings(cx, move |settings| settings.viewer.syntax = colours);
        cx.notify();
    }

    /// Shows the rows as the background index finds them.
    fn poll(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
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
        })
    }

    /// Shows the file again as `shown` says, at the top unless `keep_top`.
    fn show(&mut self, keep_top: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.shown.text() {
            Ok(text) => self.replace(text, keep_top, window, cx),
            Err(err) => self.failed(err, window, cx),
        }
    }

    fn replace(
        &mut self,
        text: TextFile,
        keep_top: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.size = file_size(&self.shown, &text);
        self.markdown = markdown(&self.shown, &text);
        self.leave_book();
        self.open_book();
        self.pdf = pdf_view(&self.shown, cx);
        self.zoom_pdf(cx);
        self.convert(cx);
        self.decode_still(cx);
        self.media = media_view(&self.shown, self.media.take(), self.quick, cx);
        self.text = Arc::new(text);
        self.parse(cx);
        self.problem = None;
        self.selection = (0, 0);
        self.stop_finding();
        if !keep_top {
            let mut view = self.view.0.borrow_mut();
            (view.top, view.x) = (0, px(0.));
        }
        self._poll = Self::poll(cx);
        if !self.quick {
            window.set_window_title(&title(self.shown.path()));
        }
        cx.notify();
    }

    fn failed(&mut self, err: io::Error, window: &mut Window, cx: &mut Context<Self>) {
        let text = format!("{}: {err}", self.shown.path().display());
        match self.quick {
            true => {
                self.media = None;
                self.problem = Some(format!("Cannot show it: {err}").into());
                if crate::sidebar::lost_device(&err) {
                    crate::reconnect_devices(cx);
                }
            }
            false => window.push_notification(Notification::error(text), cx),
        }
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown.set_mode(mode) {
            self.show(false, window, cx);
        }
    }

    /// ANSI and ASCII show text in their code page, as Lister does.
    fn set_font(
        &mut self,
        font: Font,
        page: Option<u16>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shown.set_font(font, page);
        self.show(true, window, cx);
    }

    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown.wrap = !self.shown.wrap;
        self.show(false, window, cx);
    }

    /// The panel's next or previous file.
    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown.step(by) {
            self.restart(window, cx);
        }
    }

    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.shown.start() {
            Ok(text) => self.replace(text, false, window, cx),
            Err(err) => self.failed(err, window, cx),
        }
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                (this.shown.files, this.shown.at) = (paths, 0);
                this.restart(window, cx);
            });
        })
        .detach();
    }

    /// Copies the file elsewhere.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.shown.path().to_owned();
        let folder = from.parent().unwrap_or(Path::new("/"));
        let name = from
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let target = cx.prompt_for_new_path(folder, name.as_deref());
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(to))) = target.await else {
                return;
            };
            if let Err(err) = std::fs::copy(&from, &to) {
                let text = format!("Could not save {}: {err}", to.display());
                let _ =
                    cx.update(|window, cx| window.push_notification(Notification::error(text), cx));
            }
        })
        .detach();
    }

    fn selected(&self) -> std::ops::Range<u64> {
        let (anchor, head) = self.selection;
        anchor.min(head)..anchor.max(head)
    }

    /// The drawn row under `position`, and the column in it.
    fn hit(&self, position: Point<Pixels>, window: &mut Window) -> Option<(usize, usize)> {
        let view = self.view.0.borrow();
        let last = view.drawn.len().checked_sub(1)?;
        let y = ((position.y - view.bounds.origin.y) / view.row()).floor();
        if y > last as f32 {
            return Some((last, usize::MAX));
        }
        let index = y.max(0.) as usize;
        let row = &view.drawn[index];
        let run = TextRun {
            len: row.text.len(),
            font: view.font.clone(),
            ..Default::default()
        };
        let line =
            window
                .text_system()
                .shape_line(row.text.clone().into(), view.font_size, &[run], None);
        let x = position.x - view.bounds.origin.x + view.x - PAD;
        Some((
            index,
            row.text[..line.closest_index_for_x(x)].chars().count(),
        ))
    }

    fn offset_at(&self, position: Point<Pixels>, window: &mut Window) -> Option<u64> {
        let (row, column) = self.hit(position, window)?;
        Some(self.view.0.borrow().drawn[row].offset(column))
    }

    /// A click starts a selection, Shift extends it, a double click takes the word.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.offset_at(event.position, window) else {
            return;
        };
        if event.click_count == 2 {
            let (row, column) = self.hit(event.position, window).unwrap_or_default();
            if let Some(word) = self.view.0.borrow().drawn[row].word(column) {
                self.selection = (word.start, word.end);
            }
        } else if event.modifiers.shift {
            self.selection.1 = at;
        } else {
            self.selection = (at, at);
        }
        self.selecting = event.click_count == 1;
        cx.notify();
    }

    /// Dragging past the top or bottom scrolls, as in Lister.
    fn drag(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let bounds = self.view.0.borrow().bounds;
        if position.y < bounds.top() {
            self.view.0.borrow_mut().scroll_rows(-1);
        } else if position.y > bounds.bottom() {
            self.view.0.borrow_mut().scroll_rows(1);
        }
        if let Some(at) = self.offset_at(position, window) {
            self.selection.1 = at;
        }
        cx.notify();
    }

    fn copy(&mut self, hex: bool, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.selected();
        if range.is_empty() {
            return;
        }
        if range.end - range.start > COPY_LIMIT {
            let text = format!("Selections over {} MB are not copied.", COPY_LIMIT >> 20);
            window.push_notification(Notification::warning(text), cx);
            return;
        }
        let text = match hex {
            true => self.text.hex_of(range),
            false => self.text.text_of(range),
        };
        match text {
            Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Err(err) => self.failed(err, window, cx),
        }
    }

    fn search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.selected();
        // A search takes a phrase, not a file.
        let range = range.start..range.end.min(range.start + 1000);
        match self.text.text_of(range) {
            Ok(text) if !text.trim().is_empty() => cx.open_url(&search_url(&text)),
            Ok(_) => {}
            Err(err) => self.failed(err, window, cx),
        }
    }

    fn stop_finding(&mut self) {
        if let Some(stop) = self.finding.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// Where a new Find starts, as in Lister: the first row shown.
    fn screen_top(&self) -> u64 {
        self.view
            .0
            .borrow()
            .drawn
            .first()
            .map_or(0, |row| row.offset(0))
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shown.has_rows() {
            return;
        }
        let Searches { query, history } = searches(cx).clone();
        let viewer = cx.entity().downgrade();
        find_dialog::open(query, history, window, cx, move |query, window, cx| {
            let searches = searches(cx);
            searches.add(query.clone());
            if let Err(err) = searches.save() {
                let text = format!("Could not save the searches: {err}");
                window.push_notification(Notification::error(text), cx);
            }
            let _ = viewer.update(cx, |this, cx| {
                let from = this.screen_top();
                this.run_find(&query, from, query.backwards, window, cx);
            });
        });
    }

    /// F3 and F5 go on as the dialog said, with Shift the other way, from the selection.
    fn find_next(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shown.has_rows() {
            return;
        }
        let query = searches(cx).query.clone();
        if query.text.is_empty() {
            return self.open_find(window, cx);
        }
        let backwards = query.backwards != reverse;
        let selected = self.selected();
        let from = match (selected.is_empty(), backwards) {
            (true, _) => self.screen_top(),
            (false, true) => selected.start,
            (false, false) => selected.end,
        };
        self.run_find(&query, from, backwards, window, cx);
    }

    /// Searches in the background; the match becomes the selection.
    fn run_find(
        &mut self,
        query: &Query,
        from: u64,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pattern = match query.pattern() {
            Ok(pattern) => pattern,
            Err(err) => return window.push_notification(Notification::error(err), cx),
        };
        self.stop_finding();
        let stop = Arc::new(AtomicBool::new(false));
        self.finding = Some(stop.clone());
        let (text, searched) = (self.text.clone(), query.text.clone());
        cx.spawn_in(window, async move |this, cx| {
            let stopped = stop.clone();
            let search = async move { text.find(&pattern, from, backwards, &stopped) };
            let found = cx.background_executor().spawn(search).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Stopped for another search or file.
                if !this
                    .finding
                    .as_ref()
                    .is_some_and(|ours| Arc::ptr_eq(ours, &stop))
                {
                    return;
                }
                this.finding = None;
                match found {
                    Ok(Some(found)) => {
                        this.selection = (found.start, found.end);
                        this.view.0.borrow_mut().reveal = Some(found.start);
                    }
                    Ok(None) => {
                        let text = format!("Not found: {searched}");
                        window.push_notification(Notification::warning(text), cx);
                    }
                    Err(err) => this.failed(err, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn scroll(&mut self, by: impl FnOnce(&mut Viewport), cx: &mut Context<Self>) {
        by(&mut self.view.0.borrow_mut());
        cx.notify();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.view.0.borrow().row();
        let delta = event.delta.pixel_delta(row);
        self.scroll(
            |view| {
                view.wheel += delta.y;
                let rows = (view.wheel / row).trunc();
                view.wheel -= row * rows;
                view.scroll_rows(-(rows as isize));
                view.scroll_x(-delta.x);
            },
            cx,
        );
    }

    /// An office document's PDF, made in the background for the PDF mode.
    /// The next step of `ZOOMS` up (`steps` > 0) or down, or 100% for 0.
    fn zoom_by(&mut self, steps: i32, cx: &mut Context<Self>) {
        let zoom = match steps {
            0 => 1.,
            _ if steps > 0 => ZOOMS.iter().copied().find(|&zoom| zoom > self.zoom + 0.001).unwrap_or(self.zoom),
            _ => ZOOMS.iter().copied().rev().find(|&zoom| zoom < self.zoom - 0.001).unwrap_or(self.zoom),
        };
        if zoom == self.zoom {
            return;
        }
        // The picture's middle stays in the middle.
        let area = self.picture_area.get();
        let (before, offset) = (self.zoom, self.picture_scroll.offset());
        let keep = |offset: Pixels, side: Pixels| -((-offset + side / 2.) * (zoom / before) - side / 2.);
        self.picture_scroll.set_offset(point(keep(offset.x, area.width), keep(offset.y, area.height)));
        self.zoom = zoom;
        // Zooming a picture shows it as it fits, enlarged.
        self.fit = true;
        self.zoom_pdf(cx);
        cx.notify();
    }

    fn zoom_pdf(&self, cx: &mut Context<Self>) {
        let zoom = self.zoom;
        if let Some(pdf) = &self.pdf {
            pdf.update(cx, |pdf, cx| pdf.set_zoom(zoom, cx));
        }
    }

    /// − 100% +: the percentage goes back to 100%.
    fn render_zoom(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (smallest, largest) = (ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let step = |id: &'static str, icon: IconName, tip: &'static str, steps: i32, at_end: bool| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .disabled(at_end)
                .on_click(cx.listener(move |this, _, _, cx| this.zoom_by(steps, cx)))
        };
        h_flex()
            .gap_0p5()
            .child(step("zoom-out", IconName::Minus, "Zoom out (Ctrl+-)", -1, self.zoom <= smallest))
            .child(
                Button::new("zoom-reset")
                    .ghost()
                    .xsmall()
                    .min_w(px(44.))
                    .label(format!("{:.0}%", self.zoom * 100.))
                    .tooltip("Back to 100% (Ctrl+0)")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by(0, cx))),
            )
            .child(step("zoom-in", IconName::Plus, "Zoom in (Ctrl++)", 1, self.zoom >= largest))
    }

    /// Ctrl and the wheel zoom; caught before the views scroll with it.
    fn wheel_zoom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let viewer = cx.entity().downgrade();
        let listen = move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _: &mut App| {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture || !event.modifiers.secondary() || !bounds.contains(&event.position) {
                    return;
                }
                cx.stop_propagation();
                let _ = viewer.update(cx, |viewer, cx| {
                    let steps = match event.delta {
                        // Up zooms in.
                        ScrollDelta::Lines(lines) => lines.y.signum() as i32,
                        ScrollDelta::Pixels(pixels) => {
                            viewer.wheel += pixels.y;
                            let steps = (viewer.wheel / WHEEL_STEP) as i32;
                            viewer.wheel -= WHEEL_STEP * steps as f32;
                            steps
                        }
                    };
                    for _ in 0..steps.unsigned_abs() {
                        viewer.zoom_by(steps.signum(), cx);
                    }
                });
            });
        };
        canvas(|_, _, _| {}, listen).absolute().top_0().left_0().size_full()
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        let (Some(converter), Mode::Pdf) = (self.shown.office.clone(), self.shown.mode) else {
            (self.converting, self._converting) = (None, None);
            return;
        };
        let path = self.shown.path().to_owned();
        self.converting = Some(Ok(converter.name));
        self._converting = Some(cx.spawn(async move |this, cx| {
            let file = path.clone();
            let converted = cx
                .background_executor()
                .spawn(async move { pc_core::convert::to_pdf(&converter, &file) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Another file or mode since: not for it.
                if this.shown.path() != path || this.shown.mode != Mode::Pdf {
                    return;
                }
                match converted {
                    Ok(pdf) => {
                        this.pdf = Some(cx.new(|cx| PdfView::new(pdf, cx)));
                        this.zoom_pdf(cx);
                        this.converting = None;
                    }
                    Err(err) => this.converting = Some(Err(err.to_string())),
                }
                cx.notify();
            });
        }));
    }

    /// The Image mode of a picture only libmpv reads: drawn once, at its own size.
    fn decode_still(&mut self, cx: &mut Context<Self>) {
        let path = self.shown.path().to_owned();
        let wanted = self.shown.mode == Mode::Picture && pc_core::media::is_still(&path);
        if self.still.as_ref().is_some_and(|(shown, _)| *shown == path && wanted) {
            return;
        }
        if let Some((_, Ok(image))) = self.still.take() {
            self.stale.push(image);
        }
        self._still = wanted.then(|| {
            cx.spawn(async move |this, cx| {
                let file = path.clone();
                let drawn = cx.background_executor().spawn(async move { pc_core::media::still(&file, 16384) }).await;
                let image = drawn.and_then(|frame| {
                    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra);
                    buffer.map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)]))).ok_or_else(|| "no picture".into())
                });
                let _ = this.update(cx, |this, cx| {
                    // Another file since: not for it.
                    if this.shown.path() == path {
                        this.still = Some((path, image));
                        cx.notify();
                    }
                });
            })
        });
    }

    /// The player, when video or sound is shown.
    fn player(&self, cx: &App) -> Option<Rc<pc_core::media::Player>> {
        self.media.as_ref().and_then(|media| media.read(cx).player())
    }

    /// One of the menus, as it is now.
    fn menu<M: Entries>(&self, name: &str, menu: M, cx: &App) -> M {
        let Shown {
            mode,
            font,
            page,
            wrap,
            picture,
            pdf,
            ..
        } = self.shown;
        let (fit, only_larger) = (self.fit, self.only_larger);
        let minimap = crate::minimap_shown(cx);
        let columns = self.shown.settings.text_width;
        let none = self.selected().is_empty();
        let file = |menu: M| {
            menu.entry("Open...", false, Box::new(OpenFile), false)
                .entry("Save as...", false, Box::new(SaveAs), false)
                .separator()
                .entry("Next file", false, Box::new(NextFile), false)
                .entry("Previous file", false, Box::new(PreviousFile), false)
                .separator()
                .entry("Reload", false, Box::new(Reload), false)
                .separator()
                .entry("Close", false, Box::new(Close), false)
        };
        let picture_shown = matches!(mode, Mode::Picture | Mode::Markdown | Mode::Pdf);
        let edit = move |menu: M| {
            menu.entry("Copy as text", false, Box::new(CopyText), none)
                .entry("Select all", false, Box::new(SelectAll), false)
                .separator()
                .entry("Find...", false, Box::new(Find), picture_shown)
                .entry("Find next", false, Box::new(FindNext), picture_shown)
        };
        let options = move |mut menu: M| {
            let modes: [(&str, Mode, Box<dyn Action>); 10] = [
                ("Text only", Mode::Text, Box::new(ShowText)),
                (
                    "Binary (fixed line length)",
                    Mode::Binary,
                    Box::new(ShowBinary),
                ),
                ("Hex", Mode::Hex, Box::new(ShowHex)),
                ("Image/Multimedia", Mode::Picture, Box::new(ShowPicture)),
                ("HTML text (strip tags)", Mode::Html, Box::new(ShowHtml)),
                ("Unicode (UTF-16)", Mode::Unicode, Box::new(ShowUnicode)),
                ("UTF-8", Mode::Utf8, Box::new(ShowUtf8)),
                ("Markdown", Mode::Markdown, Box::new(ShowMarkdown)),
                ("PDF", Mode::Pdf, Box::new(ShowPdf)),
                ("Book (EPUB, FB2)", Mode::Book, Box::new(ShowBook)),
            ];
            let book = self.shown.book;
            for (label, this, action) in modes {
                let off = this == Mode::Picture && !picture || this == Mode::Pdf && !pdf || this == Mode::Book && !book;
                menu = menu.entry(label, mode == this, action, off);
            }
            menu = menu.separator();
            let fonts: [(&str, Font, Box<dyn Action>); 3] = [
                ("ANSI (Windows charset)", Font::Ansi, Box::new(FontAnsi)),
                ("ASCII (DOS charset)", Font::Ascii, Box::new(FontAscii)),
                (
                    "Variable char width",
                    Font::Variable,
                    Box::new(FontVariable),
                ),
            ];
            for (label, this, action) in fonts {
                menu = menu.entry(label, font == this, action, false);
            }
            let image = mode == Mode::Picture && !self.shown.media;
            let larger = "Fit only larger images";
            menu.separator()
                .entry(
                    format!("Wrap text ({columns} columns)"),
                    wrap,
                    Box::new(ToggleWrap),
                    false,
                )
                .separator()
                .entry(
                    "Fit image to window",
                    fit,
                    Box::new(FitImage),
                    !image,
                )
                .entry(larger, only_larger, Box::new(FitLarger), !image)
                .separator()
                .entry("Minimap", minimap, Box::new(ToggleMinimap), false)
                .entry("Syntax colors", self.colours, Box::new(ToggleSyntax), !matches!(mode, Mode::Text | Mode::Utf8))
                .separator()
                .entry("Configure...", false, Box::new(Configure), false)
        };
        let (ansi, dos) = codepage::system();
        let encoding = move |mut menu: M, off: bool| {
            menu = menu
                .entry(
                    "As set by the font",
                    page.is_none(),
                    Box::new(SetPage(None)),
                    off,
                )
                .separator();
            for &(label, number) in codepage::MENU {
                let local = match number {
                    _ if number == ansi => " (ANSI)",
                    _ if number == dos => " (DOS)",
                    _ => "",
                };
                let label = format!("{label}{local}");
                menu = menu.entry(
                    label,
                    page == Some(number),
                    Box::new(SetPage(Some(number))),
                    off,
                );
            }
            menu
        };
        // Text in UTF-8 or UTF-16 has no code page.
        let unpaged = matches!(
            mode,
            Mode::Unicode | Mode::Utf8 | Mode::Html | Mode::Picture | Mode::Markdown | Mode::Pdf
        );
        match name {
            "File" => file(menu),
            "Edit" => edit(menu),
            "Options" => options(menu),
            _ => encoding(menu, unpaged),
        }
    }

    /// The rows on screen, and where they are in the file.
    fn render_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> (AnyElement, String) {
        let theme = cx.theme();
        let (rows, scanned, done) = self.text.indexed();
        let (family, font_size) = match self.shown.font {
            Font::Variable => (theme.font_family.clone(), theme.font_size * self.zoom),
            Font::Ansi | Font::Ascii => (theme.mono_font_family.clone(), theme.mono_font_size * self.zoom),
        };
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font(family.clone()));
        let tight = matches!(self.shown.mode, Mode::Text | Mode::Binary)
            && DOS_PAGES.contains(&self.shown.page());
        let row = match tight {
            true => {
                let glyphs = text_system.ascent(font_id, font_size)
                    + text_system.descent(font_id, font_size);
                px(f32::from(glyphs).ceil())
            }
            false => (ROW * self.zoom).round(),
        };
        self.view.0.borrow_mut().row_height = Some(row);
        let top = {
            let mut view = self.view.0.borrow_mut();
            view.rows = rows;
            // A Find result shows on the fourth row, as in Lister.
            if let Some(row) = view.reveal.and_then(|at| self.text.row_of(at)) {
                (view.top, view.reveal) = (row.saturating_sub(3), None);
            }
            view.top = view.top.min(view.max_top());
            view.top
        };
        let drawn = match self.view.0.borrow().visible() {
            0 => FIRST_DRAWN,
            visible => visible + 1,
        };
        let lines = self.text.rows(top..top + drawn);
        let em = text_system.em_advance(font_id, font_size).unwrap_or(px(8.));
        let widest = lines.iter().map(|line| line.text.chars().count()).max();
        let x = {
            let mut view = self.view.0.borrow_mut();
            view.width = em * widest.unwrap_or(0) as f32 + px(24.);
            view.x = view.x.min(view.max_x());
            view.x
        };
        let shown = self.view.0.borrow().visible().min(rows.saturating_sub(top));
        let status = match done {
            _ if self.finding.is_some() => "Searching…".into(),
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
        let selection = self.selected();
        let highlight = HighlightStyle {
            background_color: Some(theme.selection),
            ..Default::default()
        };
        // The language's colours for the rows drawn, by the bytes in the file.
        let styles = match (&self.syntax, lines.first(), lines.last()) {
            (Some(syntax), Some(first), Some(last)) => {
                let start = first.cells.first().map_or(first.end, |cell| cell.start) as usize;
                syntax.styles(&(start..last.end as usize), &*theme.highlight_theme)
            }
            _ => Vec::new(),
        };
        let style_at = |at: u64| {
            let ix = styles.partition_point(|(range, _)| range.end as u64 <= at);
            styles.get(ix).filter(|(range, _)| range.start as u64 <= at).map(|(_, style)| *style)
        };
        let texts: Vec<_> = lines
            .iter()
            .map(|row| {
                let text = StyledText::new(row.text.clone());
                match styles.is_empty() {
                    true => text.with_highlights(row.highlights(&selection).into_iter().map(|run| (run, highlight))),
                    false => text.with_highlights(row.runs(|cell| {
                        let selected = cell.start < selection.end && selection.start < cell.end;
                        match (style_at(cell.start), selected) {
                            (Some(style), true) => Some(style.highlight(highlight)),
                            (None, true) => Some(highlight),
                            (style, false) => style,
                        }
                    })),
                }
            })
            .collect();
        {
            let mut view = self.view.0.borrow_mut();
            view.drawn = lines;
            (view.font, view.font_size) = (font(family.clone()), font_size);
        }
        let bounds_of = self.view.clone();
        let viewer = cx.entity().downgrade();
        let dragged = move |_, _, window: &mut Window, _: &mut App| {
            let moved = viewer.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left)
                {
                    let _ = moved.update(cx, |this, cx| this.drag(event.position, window, cx));
                }
            });
            let released = viewer.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = released.update(cx, |this, _| this.selecting = false);
                }
            });
        };
        let viewer = cx.entity().downgrade();
        let focus = self.focus.clone();
        let rows = div()
            .id("viewer-rows")
            .size_full()
            .relative()
            .overflow_hidden()
            .cursor_text()
            .on_scroll_wheel(cx.listener(Self::wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_action(
                cx.listener(move |this, _: &Left, _, cx| this.scroll(|v| v.scroll_x(-em * 8.), cx)),
            )
            .on_action(
                cx.listener(move |this, _: &Right, _, cx| this.scroll(|v| v.scroll_x(em * 8.), cx)),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds_of.0.borrow_mut().bounds = bounds,
                    dragged,
                )
                .absolute()
                .size_full(),
            )
            .child(
                v_flex()
                    .absolute()
                    .top_0()
                    .left(-x)
                    .pl(PAD)
                    // Not on the rows' parent: its context menu keeps the UI font.
                    .font_family(family)
                    .text_size(font_size)
                    .line_height(row)
                    .children(
                        texts
                            .into_iter()
                            .map(|text| div().h(row).whitespace_nowrap().child(text)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&self.view).axis(ScrollbarAxis::Both)),
            )
            .context_menu(move |menu, _, cx| {
                let none = viewer.read_with(cx, |this, _| this.selected().is_empty());
                let none = none.unwrap_or(true);
                let google = "Search for selected text with Google";
                menu.action_context(focus.clone())
                    .menu_with_disabled("Copy as text", Box::new(CopyText), none)
                    .menu_with_disabled("Copy as Hex characters", Box::new(CopyHex), none)
                    .menu_with_disabled(google, Box::new(SearchGoogle), none)
            });
        (rows.into_any_element(), status)
    }

    /// Markdown and books scroll their rendered page; false in other modes.
    fn scroll_page(&self, by: PageScroll, cx: &mut Context<Self>) -> bool {
        if !matches!(self.shown.mode, Mode::Markdown | Mode::Book) {
            return false;
        }
        let scroll = &self.markdown_scroll;
        let (offset, max) = (scroll.offset(), scroll.max_offset());
        let line = px(40.) * self.zoom;
        let page = (scroll.bounds().size.height - line).max(line);
        let y = match by {
            PageScroll::Lines(lines) => offset.y - line * lines,
            PageScroll::Pages(pages) => offset.y - page * pages,
            PageScroll::Top => px(0.),
            PageScroll::Bottom => -max.y,
        };
        scroll.set_offset(point(offset.x, y.clamp(-max.y, px(0.))));
        cx.notify();
        true
    }

    /// The Book mode's book read, and where it was left; nothing else is a book.
    fn open_book(&mut self) {
        (self.book, self.book_map, self.book_print, self.book_resume) = (None, None, None, None);
        if self.shown.mode != Mode::Book {
            return;
        }
        let path = self.shown.path().to_owned();
        self.book = Some(ebook::html(&path).map(SharedString::from).map_err(|err| err.to_string()));
        if let Some(Ok(html)) = &self.book {
            self.book_map = Some(ebook::minimap_text(html).into());
        }
        // Never in Quick View, which only shows it.
        if !self.quick {
            self.book_print = resume::fingerprint(&path).ok();
            self.book_resume = self.book_print.as_deref().and_then(resume::left_at);
        }
    }

    /// The share of the book read so far, to go on from next time; not before it went back there.
    fn leave_book(&self) {
        let (Some(print), None) = (&self.book_print, self.book_resume) else {
            return;
        };
        let (offset, max) = (self.markdown_scroll.offset(), self.markdown_scroll.max_offset());
        if max.y > px(0.) {
            let read = (f32::from(-offset.y) / f32::from(max.y)).clamp(0., 1.) as f64;
            let name = self.shown.path().file_name().unwrap_or_default().to_string_lossy();
            let _ = resume::leave(print, &name, read, 1.);
        }
    }

    /// The book, rendered as the Markdown mode is; scrolled to where it was left once laid out.
    fn render_book(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let html = match &self.book {
            Some(Ok(html)) => html.clone(),
            Some(Err(err)) => {
                let text = format!("This book cannot be read: {err}. 1 shows the file as text.");
                return div().p_4().child(text).into_any_element();
            }
            None => SharedString::default(),
        };
        if let Some(at) = self.book_resume {
            let max = self.markdown_scroll.max_offset();
            match max.y > px(0.) {
                true => {
                    self.markdown_scroll.set_offset(point(px(0.), -max.y * at as f32));
                    self.book_resume = None;
                    self.book_continued = Some(at);
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(std::time::Duration::from_secs(4)).await;
                        let _ = this.update(cx, |this, cx| {
                            this.book_continued = None;
                            cx.notify();
                        });
                    })
                    .detach();
                }
                // Laid out at the next frame.
                false => window.request_animation_frame(),
            }
        }
        let view = TextView::html("viewer-book", html).selectable(true);
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("viewer-book-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.markdown_scroll)
                    .child(v_flex().px_6().py_4().text_size(cx.theme().font_size * self.zoom).child(view)),
            )
            .child(div().absolute().inset_0().child(Scrollbar::new(&self.markdown_scroll)))
            .when_some(self.book_continued, |el, at| {
                el.child(
                    div()
                        .absolute()
                        .top_3()
                        .left_3()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(gpui_kit::black().opacity(0.6))
                        .text_color(gpui_kit::white())
                        .text_sm()
                        .child(format!("Continuing at {:.0}%", at * 100.)),
                )
            })
            .into_any_element()
    }

    fn render_markdown(&self, cx: &App) -> AnyElement {
        let source = self.markdown.clone().unwrap_or_else(|| {
            let limit = MARKDOWN_LIMIT >> 20;
            format!("*Over {limit} MB: 1 shows it as text.*").into()
        });
        // Scrolled here, not by the view, so the minimap can follow it.
        let view = TextView::markdown("viewer-markdown", source).selectable(true);
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("viewer-markdown-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.markdown_scroll)
                    .child(v_flex().px_4().py_2().text_size(cx.theme().font_size * self.zoom).child(view)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&self.markdown_scroll)),
            )
            .into_any_element()
    }

    fn render_picture(&self, cx: &mut Context<Self>) -> AnyElement {
        let fit = match (self.fit, self.only_larger) {
            (true, false) => ObjectFit::Contain,
            (true, true) => ObjectFit::ScaleDown,
            (false, _) => ObjectFit::None,
        };
        let zoomed = self.fit && self.zoom != 1.;
        let source = match &self.still {
            _ if !pc_core::media::is_still(self.shown.path()) => ImageSource::from(self.shown.path().to_owned()),
            Some((path, Ok(image))) if path == self.shown.path() => ImageSource::Render(image.clone()),
            Some((path, Err(err))) if path == self.shown.path() => {
                let text = format!("{err}. 1 shows the file as text.");
                return div().p_4().child(text).into_any_element();
            }
            _ => return div().p_4().child("Loading…").into_any_element(),
        };
        let picture = img(source)
            .size_full()
            // Else the picture's aspect ratio makes it as tall as wide allows.
            .min_w_0()
            .min_h_0()
            .object_fit(fit)
            .with_loading(|| div().p_4().child("Loading…").into_any_element())
            .with_fallback(|| {
                div()
                    .p_4()
                    .child("This picture cannot be shown; 1 shows it as text.")
                    .into_any_element()
            });
        // Zoomed, it fits a box that much larger, which pans.
        let area = self.picture_area.get();
        let picture = match zoomed {
            false => picture.into_any_element(),
            true => div()
                .id("picture-pan")
                .size_full()
                .flex()
                .overflow_scroll()
                .track_scroll(&self.picture_scroll)
                .child(div().flex_none().m_auto().w(area.width * self.zoom).h(area.height * self.zoom).child(picture))
                .into_any_element(),
        };
        v_flex()
            .absolute()
            .inset_0()
            .p_2()
            .child(crate::views::measure(self.picture_area.clone(), cx.entity_id()))
            .child(picture)
            .when(zoomed, |el| el.child(div().absolute().inset_0().child(Scrollbar::new(&self.picture_scroll))))
            .into_any_element()
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.stale.drain(..) {
            let _ = window.drop_image(image);
        }
        #[cfg(target_os = "macos")]
        if !self.quick {
            let menus = MENUS.map(|name| Menu::new(name).items(self.menu(name, Vec::new(), cx)));
            crate::menus::show(window, menus.into(), cx);
        }
        let (content, status) = match self.shown.mode {
            Mode::Picture => match &self.media {
                Some(media) => (media.clone().into_any_element(), String::new()),
                None => (self.render_picture(cx), String::new()),
            },
            Mode::Markdown => (self.render_markdown(cx), String::new()),
            Mode::Book => (self.render_book(window, cx), String::new()),
            Mode::Pdf => match (&self.pdf, &self.converting) {
                (Some(pdf), _) => (pdf.clone().into_any_element(), pdf.read(cx).status()),
                (None, Some(Ok(tool))) => {
                    let text = format!("Converting with {tool}…");
                    (div().p_4().child(text).into_any_element(), String::new())
                }
                (None, Some(Err(err))) => {
                    let text = format!("{err}. 1 shows the file as text.");
                    (div().p_4().child(text).into_any_element(), String::new())
                }
                (None, None) => (div().into_any_element(), String::new()),
            },
            _ => self.render_rows(window, cx),
        };
        let content = match &self.problem {
            Some(problem) => {
                let text = div().p_4().text_color(cx.theme().muted_foreground).child(problem.clone());
                text.into_any_element()
            }
            None => content,
        };
        // The PDF view has its own, of page thumbnails.
        let minimap = match self.shown.mode {
            Mode::Picture | Mode::Pdf => None,
            _ if !crate::minimap_shown(cx) => None,
            Mode::Markdown | Mode::Book => Some(self.markdown_minimap.clone()),
            _ => Some(self.minimap.clone()),
        };
        let header = (!self.quick).then(|| crate::header::title_bar(title(self.shown.path()), window, cx)).flatten();
        let theme = cx.theme();
        v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .children(header)
            .when(!self.quick, |el| {
                el.on_action(cx.listener(|_, _: &Close, window, _| window.remove_window()))
                    .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| {
                        window.toggle_fullscreen()
                    }))
            })
            // Keys the viewer leaves would act on the panel's file: only these go on.
            .when(self.quick, |el| {
                el.on_action(|Run(command): &Run, _, cx| {
                    let general = [Command::QuickViewPanel, Command::TerminalPanel, Command::Quit, Command::Settings, Command::Shortcuts];
                    let sidebar = [Command::SidebarTree, Command::SidebarConnections, Command::SidebarPlaces, Command::ToggleSidebar];
                    if general.contains(command) || sidebar.contains(command) {
                        cx.propagate();
                    }
                })
                .on_action(|_: &crate::Open, _, _| {})
                .on_action(|_: &crate::Parent, _, _| {})
            })
            .on_action(cx.listener(|this, _: &ShowText, w, cx| this.set_mode(Mode::Text, w, cx)))
            .on_action(
                cx.listener(|this, _: &ShowBinary, w, cx| this.set_mode(Mode::Binary, w, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowHex, w, cx| this.set_mode(Mode::Hex, w, cx)))
            .on_action(
                cx.listener(|this, _: &ShowPicture, w, cx| this.set_mode(Mode::Picture, w, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowHtml, w, cx| this.set_mode(Mode::Html, w, cx)))
            .on_action(
                cx.listener(|this, _: &ShowUnicode, w, cx| this.set_mode(Mode::Unicode, w, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowUtf8, w, cx| this.set_mode(Mode::Utf8, w, cx)))
            .on_action(
                cx.listener(|this, _: &ShowMarkdown, w, cx| this.set_mode(Mode::Markdown, w, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowPdf, w, cx| this.set_mode(Mode::Pdf, w, cx)))
            .on_action(cx.listener(|this, _: &ShowBook, w, cx| this.set_mode(Mode::Book, w, cx)))
            .on_action(
                cx.listener(|this, _: &FontAnsi, w, cx| this.set_font(Font::Ansi, None, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FontAscii, w, cx| this.set_font(Font::Ascii, None, w, cx)),
            )
            .on_action(cx.listener(|this, _: &FontVariable, w, cx| {
                this.set_font(Font::Variable, None, w, cx)
            }))
            .on_action(cx.listener(|this, SetPage(page): &SetPage, w, cx| {
                this.set_font(this.shown.font, *page, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleWrap, w, cx| this.toggle_wrap(w, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-1, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(0, cx)))
            .on_action(cx.listener(|this, _: &FitImage, _, cx| {
                this.fit = !this.fit;
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &FitLarger, _, cx| {
                this.only_larger = !this.only_larger;
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &NextFile, w, cx| this.step(1, w, cx)))
            .on_action(cx.listener(|this, _: &PreviousFile, w, cx| this.step(-1, w, cx)))
            .on_action(cx.listener(|this, _: &Reload, w, cx| this.show(true, w, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, w, cx| this.open_file(w, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, w, cx| this.save_as(w, cx)))
            .on_action(cx.listener(|this, _: &CopyText, w, cx| this.copy(false, w, cx)))
            .on_action(cx.listener(|this, _: &CopyHex, w, cx| this.copy(true, w, cx)))
            .on_action(cx.listener(|this, _: &SearchGoogle, w, cx| this.search(w, cx)))
            .on_action(cx.listener(|this, _: &Find, w, cx| this.open_find(w, cx)))
            .on_action(cx.listener(|this, _: &FindNext, w, cx| this.find_next(false, w, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, w, cx| this.find_next(true, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.selection = (0, this.text.len());
                cx.notify()
            }))
            .on_action(cx.listener(|_, _: &Configure, _, cx| crate::configure_viewer(cx)))
            .on_action(cx.listener(|_, _: &ToggleMinimap, _, cx| crate::toggle_minimap(cx)))
            .on_action(cx.listener(|this, _: &ToggleSyntax, _, cx| this.toggle_syntax(cx)))
            .on_action(cx.listener(|this, _: &PlayPause, _, cx| {
                this.player(cx).inspect(|player| player.toggle_pause());
            }))
            .on_action(cx.listener(|this, _: &Left, _, cx| {
                this.player(cx).inspect(|player| player.seek_by(-5.));
            }))
            .on_action(cx.listener(|this, _: &Right, _, cx| {
                this.player(cx).inspect(|player| player.seek_by(5.));
            }))
            .on_action(cx.listener(|this, _: &LineUp, _, cx| {
                if this.scroll_page(PageScroll::Lines(-1.), cx) {
                    return;
                }
                match (this.player(cx), &this.pdf) {
                    (Some(player), _) => player.change_volume(5.),
                    (None, Some(pdf)) => pdf.update(cx, |pdf, cx| pdf.scroll_by(-PDF_LINE, cx)),
                    (None, None) => this.scroll(|v| v.scroll_rows(-1), cx),
                }
            }))
            .on_action(cx.listener(|this, _: &LineDown, _, cx| {
                if this.scroll_page(PageScroll::Lines(1.), cx) {
                    return;
                }
                match (this.player(cx), &this.pdf) {
                    (Some(player), _) => player.change_volume(-5.),
                    (None, Some(pdf)) => pdf.update(cx, |pdf, cx| pdf.scroll_by(PDF_LINE, cx)),
                    (None, None) => this.scroll(|v| v.scroll_rows(1), cx),
                }
            }))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                if this.scroll_page(PageScroll::Pages(-1.), cx) {
                    return;
                }
                match (this.player(cx), &this.pdf) {
                    (Some(player), _) => player.seek_by(-60.),
                    (None, Some(pdf)) => {
                        pdf.update(cx, |pdf, cx| pdf.scroll_by(-pdf.page_height(), cx))
                    }
                    (None, None) => {
                        this.scroll(|v| v.scroll_rows(-(v.visible().max(2) as isize - 1)), cx)
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                if this.scroll_page(PageScroll::Pages(1.), cx) {
                    return;
                }
                match (this.player(cx), &this.pdf) {
                    (Some(player), _) => player.seek_by(60.),
                    (None, Some(pdf)) => pdf.update(cx, |pdf, cx| pdf.scroll_by(pdf.page_height(), cx)),
                    (None, None) => {
                        this.scroll(|v| v.scroll_rows(v.visible().max(2) as isize - 1), cx)
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| {
                if this.scroll_page(PageScroll::Top, cx) {
                    return;
                }
                match (this.player(cx), &this.pdf) {
                    (Some(player), _) => player.seek_to(0.),
                    (None, Some(pdf)) => pdf.update(cx, |pdf, cx| pdf.scroll_to_end(false, cx)),
                    (None, None) => this.scroll(|v| v.top = 0, cx),
                }
            }))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| {
                if this.scroll_page(PageScroll::Bottom, cx) {
                    return;
                }
                match &this.pdf {
                    Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_to_end(true, cx)),
                    None => this.scroll(|v| v.top = v.max_top(), cx),
                }
            }))
            // macOS shows the menus on the system's menu bar.
            .when(!self.quick && !cfg!(target_os = "macos"), |el| {
                el.child(
                    div()
                        .px_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(self.menu_bar.clone()),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(content)
                            .child(self.wheel_zoom(cx)),
                    )
                    .children(minimap),
            )
            .when(!self.quick, |el| {
                // Video and sound do not zoom.
                let zooms = self.media.is_none();
                let zoom = zooms.then(|| self.render_zoom(cx));
                el.child(status_bar(self.shown.path(), self.size, status, self.shown.describe(), cx).children(zoom))
            })
    }
}

/// The one playing goes on while its file is shown.
fn media_view(
    shown: &Shown,
    playing: Option<Entity<MediaView>>,
    paused: bool,
    cx: &mut App,
) -> Option<Entity<MediaView>> {
    if shown.mode != Mode::Picture || !shown.media {
        return None;
    }
    let playing = playing.filter(|view| view.read(cx).path() == shown.path());
    let path = shown.path().to_owned();
    Some(playing.unwrap_or_else(|| cx.new(|cx| MediaView::new(path, paused, cx))))
}

fn pdf_view(shown: &Shown, cx: &mut App) -> Option<Entity<PdfView>> {
    let path = shown.path().to_owned();
    // An office document's comes once converted.
    let pdf = shown.mode == Mode::Pdf && shown.office.is_none();
    pdf.then(|| cx.new(|cx| PdfView::new(path, cx)))
}

fn markdown(shown: &Shown, text: &TextFile) -> Option<SharedString> {
    let rendered = shown.mode == Mode::Markdown && text.len() <= MARKDOWN_LIMIT;
    let source = rendered.then(|| text.text_of(0..text.len()));
    source?.ok().map(SharedString::from)
}

fn file_size(shown: &Shown, text: &TextFile) -> u64 {
    std::fs::metadata(shown.path()).map_or(text.len(), |meta| meta.len())
}

fn status_bar(path: &Path, len: u64, status: String, shown: String, cx: &App) -> Div {
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
        .child(shown)
}
