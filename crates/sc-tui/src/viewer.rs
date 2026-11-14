//! F3: Lister in the terminal, over the core's viewer. Keys as Lister's:
//! 1 to 7 and 9 the modes (4 a picture, 9 PDF pages, which PgUp and PgDn
//! turn), A and S the code pages, W wraps, N and P the next and previous
//! file, F7 finds, F3 and F5 find again.

use std::io;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sc_core::settings::ViewerSettings;
use sc_core::viewer::{Font, Mode, Query, Shown, TextFile};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::dialog::Form;
use crate::picture::{self, Key, PICTURES, Picture};
use crate::ui::{self, BAR, PANEL};
use crate::{App, Done};

/// Left and Right move this many columns.
const SIDEWAYS: usize = 4;

pub const KEYS: [&str; 10] = ["", "Reload", "Next", "", "", "", "Search", "", "", "Quit"];

pub struct Viewer {
    pub shown: Shown,
    text: Arc<TextFile>,
    /// A server's or an archive's file goes by its own name, not its copy's.
    title: Option<String>,
    top: usize,
    left: usize,
    /// The match Find found.
    selection: Range<u64>,
    /// Brings the row of this offset in sight, this many rows from the top.
    reveal: Option<(u64, usize)>,
    finding: Option<Arc<AtomicBool>>,
    /// Text rows and columns at the last frame.
    pub height: usize,
    pub width: usize,
    pub picture: Option<Picture>,
    pub loading: Option<Key>,
    /// The PDF's page in sight, and how many it has once known.
    pub page: usize,
    pub pages: usize,
}

impl Viewer {
    pub fn open(
        files: Vec<PathBuf>,
        at: usize,
        settings: ViewerSettings,
        title: Option<String>,
    ) -> io::Result<Self> {
        let mut shown = Shown::new(files, at, settings, PICTURES.as_slice(), false, false);
        // Wrapped rows are as wide as the terminal; pictures fit its rows.
        let (columns, lines) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
        shown.settings.text_width = columns as usize;
        let text = shown.start()?;
        Ok(Self {
            shown,
            text: Arc::new(text),
            title,
            top: 0,
            left: 0,
            selection: 0..0,
            reveal: None,
            finding: None,
            height: lines.saturating_sub(2).max(1) as usize,
            width: columns as usize,
            picture: None,
            loading: None,
            page: 0,
            pages: 0,
        })
    }

    /// Indexing or finding goes on in the background.
    pub fn busy(&self) -> bool {
        !self.text.indexed().2 || self.reveal.is_some() || self.finding.is_some()
    }

    fn stop_finding(&mut self) {
        if let Some(stop) = self.finding.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn rows(&self) -> usize {
        self.text.indexed().0
    }

    fn scroll(&mut self, by: isize) {
        let last = self.rows().saturating_sub(self.height);
        self.top = self.top.saturating_add_signed(by).min(last);
    }

    /// Where the first row in sight starts.
    fn screen_top(&self) -> u64 {
        let rows = self.text.rows(self.top..self.top + 1);
        rows.first().map_or(0, |row| row.offset(0))
    }

    /// The same file again, in its new mode, the same text in sight.
    fn reload(&mut self) -> io::Result<()> {
        // A place not reached yet is still the one to keep.
        let keep = match self.reveal {
            Some((offset, _)) => offset,
            None => self.screen_top(),
        };
        self.shown.settings.text_width = self.width.max(1);
        self.text = Arc::new(self.shown.text()?);
        (self.top, self.left, self.selection) = (0, 0, 0..0);
        self.reveal = Some((keep, 0));
        Ok(())
    }

    /// Next or previous of the panel's files.
    fn step(&mut self, by: isize) -> io::Result<()> {
        if !self.shown.step(by) {
            return Ok(());
        }
        self.stop_finding();
        (self.picture, self.loading, self.page, self.pages) = (None, None, 0, 0);
        self.text = Arc::new(self.shown.start()?);
        (self.top, self.left, self.selection, self.reveal) = (0, 0, 0..0, None);
        Ok(())
    }

    fn picture_mode(&self) -> bool {
        matches!(self.shown.mode, Mode::Picture | Mode::Pdf)
    }

    /// The picture on the screen: when it changes, the terminal is cleared
    /// of the old one's pixels.
    pub fn picture_in_sight(&self) -> Option<&Key> {
        let picture = self.picture.as_ref().filter(|_| self.picture_mode())?;
        Some(&picture.key)
    }

    /// PgUp, PgDn, Home and End go through a PDF's pages.
    fn turn(&mut self, by: isize) {
        let last = self.pages.saturating_sub(1);
        self.page = self.page.saturating_add_signed(by).min(last);
    }

    fn name(&self) -> String {
        match &self.title {
            Some(title) => title.clone(),
            None => self.shown.path().display().to_string(),
        }
    }
}

impl App {
    pub fn viewer_key(&mut self, key: KeyEvent) {
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let page = viewer.height.max(1) as isize;
        let rows = !viewer.picture_mode();
        let pdf = viewer.shown.mode == Mode::Pdf;
        let done = match key.code {
            KeyCode::Esc | KeyCode::F(10) | KeyCode::Char('q') => {
                viewer.stop_finding();
                self.viewer = None;
                return;
            }
            KeyCode::Up | KeyCode::PageUp if pdf => return viewer.turn(-1),
            KeyCode::Down | KeyCode::PageDown if pdf => return viewer.turn(1),
            KeyCode::Home if pdf => return viewer.page = 0,
            KeyCode::End if pdf => return viewer.turn(isize::MAX),
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Left
            | KeyCode::Right
                if !rows =>
            {
                return;
            }
            KeyCode::Up => return viewer.scroll(-1),
            KeyCode::Down => return viewer.scroll(1),
            KeyCode::PageUp => return viewer.scroll(-page),
            KeyCode::PageDown => return viewer.scroll(page),
            KeyCode::Home => return (viewer.top, viewer.left) = (0, 0),
            KeyCode::End => return viewer.scroll(isize::MAX),
            KeyCode::Left => return viewer.left = viewer.left.saturating_sub(SIDEWAYS),
            KeyCode::Right if !viewer.shown.wrap => return viewer.left += SIDEWAYS,
            KeyCode::Char(c @ '1'..='9') => {
                let mode = match c {
                    '1' => Mode::Text,
                    '2' => Mode::Binary,
                    '3' => Mode::Hex,
                    '4' => Mode::Picture,
                    '5' => Mode::Html,
                    '6' => Mode::Unicode,
                    '7' => Mode::Utf8,
                    '8' => Mode::Markdown,
                    _ => Mode::Pdf,
                };
                match viewer.shown.set_mode(mode) {
                    true => viewer.reload(),
                    false => Ok(()),
                }
            }
            KeyCode::Char(c @ ('a' | 's' | 'v')) => {
                let font = match c {
                    'a' => Font::Ansi,
                    's' => Font::Ascii,
                    _ => Font::Variable,
                };
                viewer.shown.set_font(font, None);
                viewer.reload()
            }
            KeyCode::Char('w') => {
                viewer.shown.wrap = !viewer.shown.wrap;
                viewer.reload()
            }
            KeyCode::Char('n') => viewer.step(1),
            KeyCode::Char('p') => viewer.step(-1),
            KeyCode::F(2) => viewer.reload(),
            KeyCode::F(7) => return self.open_find(),
            KeyCode::F(3) | KeyCode::F(5) => return self.find_next(shift),
            _ => Ok(()),
        };
        if let Err(err) = done {
            let name = self.viewer.as_ref().map(Viewer::name).unwrap_or_default();
            self.say(format!("{name}: {err}"), true);
        }
    }

    pub fn viewer_scroll(&mut self, by: isize) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.scroll(by);
        }
    }

    fn open_find(&mut self) {
        if !self.viewer.as_ref().is_some_and(|v| v.shown.has_rows()) {
            return;
        }
        let searches = self.searches();
        let query = searches.query.clone();
        let form = Form::new("Find")
            .text("Search for:", &query.text, searches.history.clone())
            .check("Whole words only", query.whole_words)
            .check("Case sensitive", query.case_sensitive)
            .check("Backwards", query.backwards)
            .check("Hex (77 6F 72 64)", query.hex)
            .check("Regular expression", query.regex);
        self.form = Some((form, crate::Purpose::Find));
    }

    pub fn find_answered(&mut self, form: &Form) {
        let query = Query {
            text: form.value(0).to_owned(),
            whole_words: form.checked(1),
            case_sensitive: form.checked(2),
            backwards: form.checked(3),
            hex: form.checked(4),
            regex: form.checked(5),
        };
        if query.text.is_empty() {
            return;
        }
        let searches = self.searches();
        searches.add(query.clone());
        if let Err(err) = searches.save() {
            self.say(format!("Could not save the searches: {err}"), true);
        }
        let from = self.viewer.as_ref().map_or(0, Viewer::screen_top);
        self.run_find(&query, from, query.backwards);
    }

    /// F3 and F5 go on as the dialog said, Shift the other way, from the match.
    fn find_next(&mut self, reverse: bool) {
        let query = self.searches().query.clone();
        let Some(viewer) = self.viewer.as_ref().filter(|v| v.shown.has_rows()) else {
            return;
        };
        if query.text.is_empty() {
            return self.open_find();
        }
        let backwards = query.backwards != reverse;
        let selected = viewer.selection.clone();
        let from = match (selected.is_empty(), backwards) {
            (true, _) => viewer.screen_top(),
            (false, true) => selected.start,
            (false, false) => selected.end,
        };
        self.run_find(&query, from, backwards);
    }

    fn run_find(&mut self, query: &Query, from: u64, backwards: bool) {
        let pattern = match query.pattern() {
            Ok(pattern) => pattern,
            Err(err) => return self.say(err, true),
        };
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        viewer.stop_finding();
        let stop = Arc::new(AtomicBool::new(false));
        viewer.finding = Some(stop.clone());
        let (text, searched) = (viewer.text.clone(), query.text.clone());
        self.background(move || {
            let found = text.find(&pattern, from, backwards, &stop);
            Box::new(move |app: &mut App| {
                let Some(viewer) = app.viewer.as_mut() else {
                    return;
                };
                // Stopped for another search or file.
                if !viewer
                    .finding
                    .as_ref()
                    .is_some_and(|ours| Arc::ptr_eq(ours, &stop))
                {
                    return;
                }
                viewer.finding = None;
                match found {
                    Ok(Some(found)) => {
                        viewer.reveal = Some((found.start, viewer.height / 3));
                        viewer.selection = found;
                    }
                    Ok(None) => app.say(format!("Not found: {searched}"), false),
                    Err(err) => app.say(err.to_string(), true),
                }
            }) as Done
        });
    }
}

/// The whole screen: the file's name and place, its rows, the keys.
pub fn draw(frame: &mut Frame, viewer: &mut Viewer) {
    let area = frame.area();
    if area.height < 3 {
        return;
    }
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    let (height, width) = (body.height as usize, body.width as usize);
    if viewer.shown.wrap && width != viewer.width && width > 0 {
        viewer.width = width;
        let _ = viewer.reload();
    }
    (viewer.height, viewer.width) = (height, width);
    if let Some((offset, above)) = viewer.reveal
        && let Some(row) = viewer.text.row_of(offset)
    {
        viewer.reveal = None;
        let shown = viewer.top..viewer.top + height;
        if above == 0 || !shown.contains(&row) {
            viewer.top = row.saturating_sub(above);
        }
        // The match's column in sight, when rows are not wrapped.
        if !viewer.shown.wrap
            && let Some(found) = viewer.text.rows(row..row + 1).first()
        {
            let column = found.cells.iter().take_while(|c| c.start < offset).count();
            if column < viewer.left || column >= viewer.left + width {
                viewer.left = column.saturating_sub(width / 3);
            }
        }
    }
    let right = match viewer.picture_mode() {
        true => {
            frame.buffer_mut().set_style(body, PANEL);
            picture::draw(frame, body, viewer);
            let about = match &viewer.picture {
                _ if viewer.shown.mode == Mode::Pdf && viewer.pages > 0 => {
                    format!(" · page {} of {}", viewer.page + 1, viewer.pages)
                }
                Some(Picture {
                    shown: Ok((_, about)),
                    ..
                }) => format!(" · {about}"),
                _ => String::new(),
            };
            format!(" {}{about} ", viewer.shown.describe())
        }
        false => draw_text(frame, body, viewer),
    };
    let buf = frame.buffer_mut();
    let top = Rect::new(area.x, area.y, area.width, 1);
    buf.set_style(top, BAR);
    let room = (area.width as usize).saturating_sub(right.width() + 1);
    buf.set_string(area.x + 1, area.y, ui::tail(&viewer.name(), room), BAR);
    let x = area.right().saturating_sub(right.width() as u16);
    buf.set_string(x, area.y, &right, BAR.add_modifier(Modifier::BOLD));
    ui::key_bar(
        buf,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        &KEYS,
    );
}

/// The rows in sight; gives the status line's right end.
fn draw_text(frame: &mut Frame, body: Rect, viewer: &mut Viewer) -> String {
    let (height, width) = (body.height as usize, body.width as usize);
    let (rows, scanned, done) = viewer.text.indexed();
    viewer.top = viewer.top.min(rows.saturating_sub(height));
    let shown = viewer.text.rows(viewer.top..viewer.top + height);

    let buf = frame.buffer_mut();
    buf.set_style(body, PANEL);
    let selected = Style::new().fg(BAR.fg.unwrap()).bg(BAR.bg.unwrap());
    for (n, row) in shown.iter().enumerate() {
        let marks = row.highlights(&viewer.selection);
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0;
        for (column, (at, c)) in row.text.char_indices().enumerate() {
            if column < viewer.left {
                continue;
            }
            used += c.width().unwrap_or(0);
            if used > width {
                break;
            }
            let style = match marks.iter().any(|run| run.contains(&at)) {
                true => selected,
                false => PANEL,
            };
            match spans.last_mut() {
                Some(span) if span.style == style => span.content.to_mut().push(c),
                _ => spans.push(Span::styled(c.to_string(), style)),
            }
        }
        buf.set_line(body.x, body.y + n as u16, &Line::from(spans), body.width);
    }

    let len = viewer.text.len().max(1);
    let percent = match done && viewer.top + height >= rows {
        true => 100,
        false => shown.first().map_or(0, |row| row.offset(0)) * 100 / len,
    };
    let place = match viewer.shown.wrap {
        true => " · Wrap".to_owned(),
        false if viewer.left > 0 => format!(" · Col {}", viewer.left + 1),
        false => String::new(),
    };
    let indexing = match done {
        true => String::new(),
        false => format!(" · reading {}%", scanned * 100 / len),
    };
    let finding = if viewer.finding.is_some() {
        " · finding…"
    } else {
        ""
    };
    format!(
        " {}{place}{indexing}{finding} · {percent:>3}% ",
        viewer.shown.describe()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::menu::Command;
    use sc_core::session::Session;
    use sc_core::settings::Settings;
    use sc_core::viewer::Searches;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        let cells = terminal.backend().buffer().content();
        cells.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn views_and_finds() {
        let root = std::env::temp_dir().join(format!("sc-tui-view-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let lines: Vec<String> = (0..300).map(|n| format!("line {n}")).collect();
        std::fs::write(root.join("a.txt"), lines.join("\n")).unwrap();
        let mut app = App::new(
            Settings::default(),
            &Session::default(),
            [root.clone()].into_iter(),
        );
        app.searches = Some(Searches::default());
        app.panel_mut().set_cursor(1);
        app.command(Command::View);
        while app.viewer.as_ref().is_some_and(|v| !v.text.indexed().2) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let shown = screen(&mut app);
        assert!(
            shown.contains("a.txt") && shown.contains("line 0"),
            "{shown}"
        );

        let query = Query {
            text: "line 250".into(),
            ..Query::default()
        };
        app.run_find(&query, 0, false);
        let done = app.done.1.recv_timeout(std::time::Duration::from_secs(5));
        done.unwrap()(&mut app);
        let shown = screen(&mut app);
        assert!(shown.contains("line 250"), "{shown}");
        let viewer = app.viewer.as_ref().unwrap();
        assert_eq!(viewer.selection.end - viewer.selection.start, 8);

        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.viewer.is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shows_pictures() {
        let root = std::env::temp_dir().join(format!("sc-tui-picture-view-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        image::RgbaImage::new(64, 32)
            .save(root.join("a.png"))
            .unwrap();
        let mut app = App::new(
            Settings::default(),
            &Session::default(),
            [root.clone()].into_iter(),
        );
        app.picker = Some(ratatui_image::picker::Picker::halfblocks());
        app.panel_mut().set_cursor(1);
        app.command(Command::View);
        app.want_picture();
        let done = app.done.1.recv_timeout(std::time::Duration::from_secs(5));
        done.unwrap()(&mut app);
        let shown = screen(&mut app);
        assert!(shown.contains("Image · 64 × 32"), "{shown}");
        app.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert!(
            screen(&mut app).contains("89 50 4E 47"),
            "the PNG's bytes in Hex"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
