//! Drawing, in Midnight Commander's colors: the terminal's own palette, so
//! its theme decides the shades.

use pc_core::menu::MENUS;
use pc_core::{Entry, Panel, archive};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::App;

pub const PANEL: Style = Style::new().fg(Color::Gray).bg(Color::Blue);
// Fixed 256-colour shades: themes repaint only the first 16, and Breeze's
// magenta and yellow are unreadable on its blue.
const YELLOW: Color = Color::Indexed(228);
const PINK: Color = Color::Indexed(219);
const GREEN: Color = Color::Indexed(120);

const HEADER: Style = Style::new()
    .fg(YELLOW)
    .bg(Color::Blue)
    .add_modifier(Modifier::BOLD);
pub const BAR: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
const KEY_NUMBER: Style = Style::new().fg(Color::Gray).bg(Color::Black);
pub const DIALOG: Style = Style::new().fg(Color::Black).bg(Color::Gray);
const ERROR: Style = Style::new().fg(Color::White).bg(Color::Red);

/// The panels' function keys; F1 and F2 have none yet.
const KEYS: [&str; 10] = [
    "", "", "View", "Edit", "Copy", "Move", "Mkdir", "Delete", "Menu", "Quit",
];

/// Where the last frame put what the mouse can hit.
#[derive(Default)]
pub struct Places {
    /// Each side's file rows.
    pub rows: [Rect; 2],
    /// Each side's tab labels.
    pub tabs: [Vec<Rect>; 2],
    pub keys: Rect,
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.places.keys = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    match app.viewer.as_mut() {
        Some(viewer) => crate::viewer::draw(frame, viewer),
        None => draw_panels(frame, app),
    }
    if let Some((form, _)) = &app.form {
        form.draw(frame);
    }
    if let Some(message) = &app.message {
        dialog(frame, &message.text, message.error);
    }
}

fn draw_panels(frame: &mut Frame, app: &mut App) {
    let [menu, panels, command, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let halves: [Rect; 2] = Layout::horizontal([Constraint::Fill(1); 2]).areas(panels);
    let buf = frame.buffer_mut();
    menu_bar(buf, menu);
    for (side, area) in halves.into_iter().enumerate() {
        app.places.rows[side] = panel(buf, area, app, side);
    }
    key_bar(buf, keys, &KEYS);
    command_line(frame, command, app);
}

fn menu_bar(buf: &mut Buffer, area: Rect) {
    buf.set_style(area, BAR);
    let labels: Vec<&str> = MENUS.iter().map(|menu| menu.label).collect();
    buf.set_stringn(
        area.x + 1,
        area.y,
        labels.join("  "),
        area.width.into(),
        BAR,
    );
}

/// One side; gives the area of its file rows.
fn panel(buf: &mut Buffer, area: Rect, app: &mut App, ix: usize) -> Rect {
    let active = ix == app.active;
    let tabs_shown = app.sides[ix].tabs.panels().len() > 1 || app.settings.tab_header_always;
    Block::bordered().style(PANEL).render(area, buf);
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    if inner.height < 4 || inner.width < 10 {
        return Rect::default();
    }
    let side = &app.sides[ix];
    let panel = side.tabs.current();
    // The location on the top border, its end kept when long.
    let title = format!(" {} ", tilde(&panel.location()));
    let title = tail(&title, (area.width as usize).saturating_sub(4));
    let title_style = if active { BAR } else { PANEL };
    buf.set_string(area.x + 2, area.y, title, title_style);

    let mut y = inner.y;
    app.places.tabs[ix] = match tabs_shown {
        true => tab_strip(buf, Rect::new(inner.x, y, inner.width, 1), app, ix),
        false => Vec::new(),
    };
    y += tabs_shown as u16;
    let columns = Columns::new(inner.width);
    columns.header(buf, inner.x, y);
    y += 1;
    let status_y = inner.bottom() - 1;
    let rows = Rect::new(inner.x, y, inner.width, status_y - 1 - y);
    // The separator above the status line, joined to the column lines.
    let line = status_y - 1;
    buf.set_string(area.x, line, "├", PANEL);
    buf.set_string(area.right() - 1, line, "┤", PANEL);
    for x in inner.x..inner.right() {
        let joint = columns.lines.iter().any(|&at| inner.x + at == x);
        buf.set_string(x, line, if joint { "┴" } else { "─" }, PANEL);
    }

    let side = &mut app.sides[ix];
    let panel = side.tabs.current();
    let visible = rows.height as usize;
    let cursor = panel.cursor();
    if cursor < side.top {
        side.top = cursor;
    } else if visible > 0 && cursor >= side.top + visible {
        side.top = cursor + 1 - visible;
    }
    side.top = side.top.min(panel.entries().len().saturating_sub(visible));
    for row in 0..visible {
        let at = side.top + row;
        let y = rows.y + row as u16;
        let entry = panel.entries().get(at);
        let style = match entry {
            Some(entry) => entry_style(panel, entry, active && at == cursor),
            None => PANEL,
        };
        buf.set_style(Rect::new(rows.x, y, rows.width, 1), style);
        columns.row(buf, rows.x, y, entry.map(|entry| (panel, entry)), style);
    }
    let status = take_width(panel.status_text().chars(), inner.width as usize);
    buf.set_string(inner.x, status_y, status, PANEL);
    rows
}

/// Gives where each label is.
fn tab_strip(buf: &mut Buffer, area: Rect, app: &App, ix: usize) -> Vec<Rect> {
    let tabs = &app.sides[ix].tabs;
    let mut x = area.x;
    let mut labels = Vec::new();
    for (n, panel) in tabs.panels().iter().enumerate() {
        let current = n == tabs.active();
        let style = match current {
            true if ix == app.active => BAR,
            true => PANEL.add_modifier(Modifier::REVERSED),
            false => PANEL,
        };
        let label = format!(" {}:{} ", n + 1, panel.title());
        let room = area.right().saturating_sub(x) as usize;
        if room == 0 {
            break;
        }
        let label = fit(&label, room.min(label.width()));
        buf.set_string(x, area.y, &label, style);
        labels.push(Rect::new(x, area.y, label.width() as u16, 1));
        x += label.width() as u16;
    }
    labels
}

fn entry_style(panel: &Panel, entry: &Entry, cursor: bool) -> Style {
    let marked = panel.is_marked(entry);
    let base = if cursor { BAR } else { PANEL };
    let fg = match () {
        _ if marked => Some(YELLOW),
        _ if cursor => None,
        _ if entry.is_dir => Some(Color::White),
        _ if archive::is_archive(&entry.name) => Some(PINK),
        _ if entry.attributes.contains('x') => Some(GREEN),
        _ => None,
    };
    let style = match fg {
        Some(fg) => base.fg(fg),
        None => base,
    };
    match marked || (entry.is_dir && !cursor) {
        true => style.add_modifier(Modifier::BOLD),
        false => style,
    }
}

/// Name, Size and Modified, as wide as the panel lets them be.
struct Columns {
    name: usize,
    size: usize,
    date: usize,
    /// Where the column lines are, from the left.
    lines: Vec<u16>,
}

impl Columns {
    fn new(width: u16) -> Self {
        let width = width as usize;
        let size = 10;
        let date = if width >= 52 { 16 } else { 0 };
        let name = width.saturating_sub(size + date + 1 + usize::from(date > 0));
        let mut lines = vec![name as u16];
        if date > 0 {
            lines.push((name + 1 + size) as u16);
        }
        Columns {
            name,
            size,
            date,
            lines,
        }
    }

    fn header(&self, buf: &mut Buffer, x: u16, y: u16) {
        let titles = [
            ("Name", self.name),
            ("Size", self.size),
            ("Modified", self.date),
        ];
        let mut at = x;
        for (n, (title, width)) in titles.into_iter().enumerate() {
            if width == 0 {
                continue;
            }
            if n > 0 {
                buf.set_string(at, y, "│", PANEL);
                at += 1;
            }
            buf.set_string(at, y, format!("{title:^width$}"), HEADER);
            at += width as u16;
        }
    }

    fn row(&self, buf: &mut Buffer, x: u16, y: u16, row: Option<(&Panel, &Entry)>, style: Style) {
        let (name, size, date) = match row {
            Some((panel, entry)) => {
                let name = match entry.is_dir {
                    true => format!("/{}", entry.display_name()),
                    false => entry.display_name(),
                };
                (
                    name,
                    size_text(panel, entry, self.size),
                    entry.modified_text(),
                )
            }
            None => Default::default(),
        };
        buf.set_string(x, y, fit(&name, self.name), style);
        let mut at = x + self.name as u16;
        buf.set_string(at, y, "│", style);
        at += 1;
        let size = format!("{:>width$}", size, width = self.size);
        buf.set_string(at, y, size, style);
        if self.date > 0 {
            at += self.size as u16;
            buf.set_string(at, y, "│", style);
            buf.set_string(at + 1, y, fit(&date, self.date), style);
        }
    }
}

/// Digits grouped, or shorter units when they do not fit.
fn size_text(panel: &Panel, entry: &Entry, width: usize) -> String {
    if entry.is_parent() {
        return "UP--DIR".into();
    }
    let text = panel.size_text(entry);
    match text.width() <= width {
        true => text,
        false => pc_core::human_size(panel.measured_size(entry).unwrap_or(entry.size)),
    }
}

/// `text` in exactly `width` columns: padded, or cut in the middle with
/// `~` so both ends (an extension) stay, as Midnight Commander does.
pub fn fit(text: &str, width: usize) -> String {
    let len = text.width();
    if len <= width {
        return format!("{text}{}", " ".repeat(width - len));
    }
    if width == 0 {
        return String::new();
    }
    let keep_end = (width - 1) / 3;
    let head = take_width(text.chars(), width - 1 - keep_end);
    let tail: String = take_width(text.chars().rev(), keep_end)
        .chars()
        .rev()
        .collect();
    let out = format!("{head}~{tail}");
    let pad = width.saturating_sub(out.width());
    format!("{out}{}", " ".repeat(pad))
}

/// The home folder as `~`, as shells show it.
fn tilde(path: &str) -> String {
    let home = std::env::home_dir().unwrap_or_default();
    match std::path::Path::new(path).strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() && rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) if !home.as_os_str().is_empty() => {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        }
        _ => path.into(),
    }
}

/// The end of `text` in `width` columns, `…` first when cut.
pub fn tail(text: &str, width: usize) -> String {
    if text.width() <= width || width == 0 {
        return text.into();
    }
    let end: String = take_width(text.chars().rev(), width - 1)
        .chars()
        .rev()
        .collect();
    format!("…{end}")
}

fn take_width(chars: impl Iterator<Item = char>, width: usize) -> String {
    let mut used = 0;
    chars
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

pub fn key_bar(buf: &mut Buffer, area: Rect, labels: &[&str; 10]) {
    let slot = area.width / 10;
    for (n, label) in labels.iter().enumerate() {
        let x = area.x + slot * n as u16;
        let width = match n {
            9 => area.right() - x,
            _ => slot,
        };
        buf.set_string(x, area.y, format!("{:>2}", n + 1), KEY_NUMBER);
        let room = width.saturating_sub(2) as usize;
        buf.set_string(x + 2, area.y, fit(label, room), BAR);
    }
}

/// The folder as the prompt, then what is typed; the end stays in sight.
fn command_line(frame: &mut Frame, area: Rect, app: &App) {
    let folder = tilde(
        &app.sides[app.active]
            .tabs
            .current()
            .folder()
            .display()
            .to_string(),
    );
    let prompt = tail(&format!("{folder}>"), (area.width / 2) as usize);
    let buf = frame.buffer_mut();
    buf.set_string(area.x, area.y, &prompt, Style::new());
    let x = area.x + prompt.width() as u16;
    let room = area.right().saturating_sub(x) as usize;
    if room == 0 {
        return;
    }
    let chars: Vec<char> = app.command.text().chars().collect();
    let start = app.command.cursor().saturating_sub(room - 1);
    let shown: String = chars[start..].iter().collect();
    let shown = take_width(shown.chars(), room);
    buf.set_string(x, area.y, &shown, Style::new());
    let before: String = chars[start..app.command.cursor()].iter().collect();
    if app.message.is_none() && app.form.is_none() {
        frame.set_cursor_position(Position::new(x + before.width() as u16, area.y));
    }
}

/// A message in a box in the middle; any key closes it.
fn dialog(frame: &mut Frame, text: &str, error: bool) {
    let area = frame.area();
    let width = (text.width() as u16 + 6).clamp(30, area.width.saturating_sub(4).max(1));
    let lines =
        (text.width() as u16).div_ceil(width - 4).max(1) + text.matches('\n').count() as u16;
    let height = (lines + 4).min(area.height);
    let at = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let style = if error { ERROR } else { DIALOG };
    let title = if error {
        " Error "
    } else {
        " Power Commander "
    };
    frame.render_widget(Clear, at);
    let block = Block::bordered().title(title).style(style);
    let body = Paragraph::new(format!("{text}\n\n[ OK ]"))
        .wrap(Wrap { trim: false })
        .centered()
        .block(block.padding(ratatui::widgets::Padding::horizontal(1)));
    frame.render_widget(body, at);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_names_into_columns() {
        assert_eq!(fit("a.txt", 7), "a.txt  ");
        assert_eq!(fit("a_very_long_name.txt", 10), "a_very~txt");
        assert_eq!(fit("ő東京.txt", 6).width(), 6);
        assert_eq!(tail("/home/user/projects", 9), "…projects");
        assert_eq!(fit("x", 0), "");
    }
}
