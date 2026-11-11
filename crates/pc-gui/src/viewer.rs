use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, MenuBuilder, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis, ScrollbarHandle};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use pc_core::codepage;
use pc_core::settings::ViewerSettings;
use pc_core::viewer::{
    COPY_LIMIT, Font, MARKDOWN_LIMIT, Mode, Query, Row, Searches, Shown, TextFile, search_url,
};

use crate::find_dialog;
use crate::minimap::{self, Minimap};
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
        CopyText,
        CopyHex,
        SearchGoogle,
        SelectAll,
        Find,
        FindNext,
        FindPrevious
    ]
);

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
        KeyBinding::new("a", FontAnsi, keys),
        KeyBinding::new("s", FontAscii, keys),
        KeyBinding::new("v", FontVariable, keys),
        KeyBinding::new("w", ToggleWrap, keys),
        KeyBinding::new("f", FitImage, keys),
        KeyBinding::new("l", FitLarger, keys),
        KeyBinding::new("m", ToggleMinimap, keys),
        KeyBinding::new("n", NextFile, keys),
        KeyBinding::new("p", PreviousFile, keys),
        KeyBinding::new("f2", Reload, keys),
        KeyBinding::new("secondary-c", CopyText, keys),
        KeyBinding::new("secondary-shift-c", CopyHex, keys),
        KeyBinding::new("secondary-a", SelectAll, keys),
        KeyBinding::new("f7", Find, keys),
        KeyBinding::new("f5", FindNext, keys),
        KeyBinding::new("shift-f5", FindPrevious, keys),
    ]);
}

/// Shows `files[at]`; Next and Previous go through `files`. `bounds` are
/// the main window's, which the viewer covers.
pub fn open(
    files: Vec<PathBuf>,
    at: usize,
    settings: ViewerSettings,
    bounds: WindowBounds,
    cx: &mut App,
) -> io::Result<()> {
    let mut shown = Shown::new(files, at, settings, Img::extensions(), true);
    let text = shown.start()?;
    let options = WindowOptions {
        window_bounds: Some(bounds),
        // Wayland lets no app place its windows; as the main window's child,
        // the compositor centres it over the main window.
        kind: match cx.compositor_name() {
            "Wayland" => WindowKind::Floating,
            _ => WindowKind::Normal,
        },
        titlebar: Some(TitlebarOptions {
            title: Some(title(shown.path()).into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Viewer::new(shown, text, window, cx))
    })
    .map(drop)
    .map_err(|err| io::Error::other(err.to_string()))
}

fn title(path: &Path) -> String {
    format!("{} — Power Commander", path.display())
}

/// The searches, shared by the viewers.
struct LastSearches(Searches);

impl Global for LastSearches {}

/// Read from disk at the first Find.
fn searches(cx: &mut App) -> &mut Searches {
    if !cx.has_global::<LastSearches>() {
        cx.set_global(LastSearches(Searches::load().unwrap_or_default()));
    }
    &mut cx.global_mut::<LastSearches>().0
}

struct Viewer {
    shown: Shown,
    menu_bar: Entity<AppMenuBar>,
    /// Shared with a running Find.
    text: Arc<TextFile>,
    /// The file's, HTML's text is shorter.
    size: u64,
    /// The source the Markdown mode renders.
    markdown: Option<SharedString>,
    /// The PDF mode's pages.
    pdf: Option<Entity<PdfView>>,
    fit: bool,
    only_larger: bool,
    /// Anchor and head, as file offsets: what Copy copies.
    selection: (u64, u64),
    /// The left button is down for a selection.
    selecting: bool,
    /// A running Find; setting it stops the search.
    finding: Option<Arc<AtomicBool>>,
    minimap: Entity<Minimap>,
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
    }
}

/// The minimap's view of the rows.
/// File, Edit, Options and Encoding, as Lister's; each built as it opens.
fn menu_bar(viewer: WeakEntity<Viewer>, cx: &mut App) -> Entity<AppMenuBar> {
    let menus = ["File", "Edit", "Options", "Encoding"].map(|name| {
        let viewer = viewer.clone();
        let build: MenuBuilder = Rc::new(move |menu, _, cx| match viewer.upgrade() {
            Some(viewer) => viewer.read(cx).menu(name, menu, cx),
            None => menu,
        });
        (name.into(), build)
    });
    AppMenuBar::built(menus.into(), cx)
}

struct ViewerMap(WeakEntity<Viewer>);

/// The minimap's view of a rendered Markdown page: its source lines,
/// scrolled as far through as the page.
struct MarkdownMap(WeakEntity<Viewer>);

impl MarkdownMap {
    /// Source lines, and the page's scroll, visible and scrollable heights.
    fn page(&self, cx: &App) -> Option<(usize, f32, f32, f32)> {
        let viewer = self.0.upgrade()?.read(cx);
        let lines = viewer.markdown.as_ref()?.lines().count().max(1);
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
            .and_then(|viewer| viewer.read(cx).markdown.clone())
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
    fn new(shown: Shown, text: TextFile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            menu_bar: menu_bar(cx.entity().downgrade(), cx),
            size: file_size(&shown, &text),
            markdown: markdown(&shown, &text),
            pdf: pdf_view(&shown, cx),
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
        }
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
        self.pdf = pdf_view(&self.shown, cx);
        self.text = Arc::new(text);
        self.selection = (0, 0);
        self.stop_finding();
        if !keep_top {
            let mut view = self.view.0.borrow_mut();
            (view.top, view.x) = (0, px(0.));
        }
        self._poll = Self::poll(cx);
        window.set_window_title(&title(self.shown.path()));
        cx.notify();
    }

    fn failed(&mut self, err: io::Error, window: &mut Window, cx: &mut Context<Self>) {
        let text = format!("{}: {err}", self.shown.path().display());
        window.push_notification(Notification::error(text), cx);
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

    /// One of the menus, as it is now.
    fn menu(&self, name: &str, menu: PopupMenu, cx: &App) -> PopupMenu {
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
        let file = |menu: PopupMenu| {
            menu.menu("Open...", Box::new(OpenFile))
                .menu("Save as...", Box::new(SaveAs))
                .separator()
                .menu("Next file", Box::new(NextFile))
                .menu("Previous file", Box::new(PreviousFile))
                .separator()
                .menu("Reload", Box::new(Reload))
                .separator()
                .menu("Close", Box::new(Close))
        };
        let picture_shown = matches!(mode, Mode::Picture | Mode::Markdown | Mode::Pdf);
        let edit = move |menu: PopupMenu| {
            menu.menu_with_disabled("Copy as text", Box::new(CopyText), none)
                .menu("Select all", Box::new(SelectAll))
                .separator()
                .menu_with_disabled("Find...", Box::new(Find), picture_shown)
                .menu_with_disabled("Find next", Box::new(FindNext), picture_shown)
        };
        let options = move |mut menu: PopupMenu| {
            let modes: [(&str, Mode, Box<dyn Action>); 9] = [
                ("Text only", Mode::Text, Box::new(ShowText)),
                (
                    "Binary (fixed line length)",
                    Mode::Binary,
                    Box::new(ShowBinary),
                ),
                ("Hex", Mode::Hex, Box::new(ShowHex)),
                ("Image", Mode::Picture, Box::new(ShowPicture)),
                ("HTML text (strip tags)", Mode::Html, Box::new(ShowHtml)),
                ("Unicode (UTF-16)", Mode::Unicode, Box::new(ShowUnicode)),
                ("UTF-8", Mode::Utf8, Box::new(ShowUtf8)),
                ("Markdown", Mode::Markdown, Box::new(ShowMarkdown)),
                ("PDF", Mode::Pdf, Box::new(ShowPdf)),
            ];
            for (label, this, action) in modes {
                let off = this == Mode::Picture && !picture || this == Mode::Pdf && !pdf;
                menu = menu.menu_with_check_and_disabled(label, mode == this, action, off);
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
                menu = menu.menu_with_check(label, font == this, action);
            }
            let image = mode == Mode::Picture;
            let larger = "Fit only larger images";
            menu.separator()
                .menu_with_check(
                    format!("Wrap text ({columns} columns)"),
                    wrap,
                    Box::new(ToggleWrap),
                )
                .separator()
                .menu_with_check_and_disabled(
                    "Fit image to window",
                    fit,
                    Box::new(FitImage),
                    !image,
                )
                .menu_with_check_and_disabled(larger, only_larger, Box::new(FitLarger), !image)
                .separator()
                .menu_with_check("Minimap", minimap, Box::new(ToggleMinimap))
                .separator()
                .menu("Configure...", Box::new(Configure))
        };
        let (ansi, dos) = codepage::system();
        let encoding = move |mut menu: PopupMenu, off: bool| {
            menu = menu
                .menu_with_check_and_disabled(
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
                menu = menu.menu_with_check_and_disabled(
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
        let menu = menu.action_context(self.focus.clone());
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
            Font::Variable => (theme.font_family.clone(), theme.font_size),
            Font::Ansi | Font::Ascii => (theme.mono_font_family.clone(), theme.mono_font_size),
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
            false => ROW,
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
        let texts: Vec<_> = lines
            .iter()
            .map(|row| {
                let runs = row.highlights(&selection).into_iter();
                let text = StyledText::new(row.text.clone());
                text.with_highlights(runs.map(|run| (run, highlight)))
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

    fn render_markdown(&self) -> AnyElement {
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
                    .child(v_flex().px_4().py_2().child(view)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&self.markdown_scroll)),
            )
            .into_any_element()
    }

    fn render_picture(&self) -> AnyElement {
        let fit = match (self.fit, self.only_larger) {
            (true, false) => ObjectFit::Contain,
            (true, true) => ObjectFit::ScaleDown,
            (false, _) => ObjectFit::None,
        };
        let picture = img(self.shown.path().to_owned())
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
        v_flex()
            .absolute()
            .inset_0()
            .p_2()
            .child(picture)
            .into_any_element()
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (content, status) = match self.shown.mode {
            Mode::Picture => (self.render_picture(), String::new()),
            Mode::Markdown => (self.render_markdown(), String::new()),
            Mode::Pdf => match &self.pdf {
                Some(pdf) => (pdf.clone().into_any_element(), pdf.read(cx).status()),
                None => (div().into_any_element(), String::new()),
            },
            _ => self.render_rows(window, cx),
        };
        // The PDF view has its own, of page thumbnails.
        let minimap = match self.shown.mode {
            Mode::Picture | Mode::Pdf => None,
            _ if !crate::minimap_shown(cx) => None,
            Mode::Markdown => Some(self.markdown_minimap.clone()),
            _ => Some(self.minimap.clone()),
        };
        let theme = cx.theme();
        v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|_, _: &Close, window, _| window.remove_window()))
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
            .on_action(cx.listener(|this, _: &LineUp, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_by(-PDF_LINE, cx)),
                None => this.scroll(|v| v.scroll_rows(-1), cx),
            }))
            .on_action(cx.listener(|this, _: &LineDown, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_by(PDF_LINE, cx)),
                None => this.scroll(|v| v.scroll_rows(1), cx),
            }))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_by(-pdf.page_height(), cx)),
                None => this.scroll(|v| v.scroll_rows(-(v.visible().max(2) as isize - 1)), cx),
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_by(pdf.page_height(), cx)),
                None => this.scroll(|v| v.scroll_rows(v.visible().max(2) as isize - 1), cx),
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_to_end(false, cx)),
                None => this.scroll(|v| v.top = 0, cx),
            }))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| match &this.pdf {
                Some(pdf) => pdf.update(cx, |pdf, cx| pdf.scroll_to_end(true, cx)),
                None => this.scroll(|v| v.top = v.max_top(), cx),
            }))
            .child(
                div()
                    .px_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.menu_bar.clone()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().relative().child(content))
                    .children(minimap),
            )
            .child(status_bar(
                self.shown.path(),
                self.size,
                status,
                self.shown.describe(),
                cx,
            ))
    }
}

fn pdf_view(shown: &Shown, cx: &mut App) -> Option<Entity<PdfView>> {
    let path = shown.path().to_owned();
    (shown.mode == Mode::Pdf).then(|| cx.new(|cx| PdfView::new(path, cx)))
}

fn markdown(shown: &Shown, text: &TextFile) -> Option<SharedString> {
    let rendered = shown.mode == Mode::Markdown && text.len() <= MARKDOWN_LIMIT;
    let source = rendered.then(|| text.text_of(0..text.len()));
    source?.ok().map(SharedString::from)
}

fn file_size(shown: &Shown, text: &TextFile) -> u64 {
    std::fs::metadata(shown.path()).map_or(text.len(), |meta| meta.len())
}

fn status_bar(path: &Path, len: u64, status: String, shown: String, cx: &App) -> impl IntoElement {
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
