//! Dialogs that ask: lines to type and boxes to tick, framed in the middle.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::{Block, Clear, Padding};
use unicode_width::UnicodeWidthStr;

use crate::input::Input;
use crate::ui::{BAR, DIALOG, fit};

pub enum Field {
    Text {
        label: String,
        input: Input,
        /// Up and Down bring these back, the latest first.
        history: Vec<String>,
        recalled: Option<usize>,
    },
    Check {
        label: String,
        on: bool,
    },
}

pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    focus: usize,
}

#[derive(Debug, PartialEq)]
pub enum Answer {
    Open,
    Ok,
    Cancel,
}

impl Form {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            fields: Vec::new(),
            focus: 0,
        }
    }

    pub fn text(mut self, label: &str, value: &str, history: Vec<String>) -> Self {
        let mut input = Input::default();
        input.insert(value);
        self.fields.push(Field::Text {
            label: label.into(),
            input,
            history,
            recalled: None,
        });
        self
    }

    pub fn check(mut self, label: &str, on: bool) -> Self {
        self.fields.push(Field::Check {
            label: label.into(),
            on,
        });
        self
    }

    pub fn value(&self, ix: usize) -> &str {
        match &self.fields[ix] {
            Field::Text { input, .. } => input.text(),
            Field::Check { .. } => "",
        }
    }

    pub fn checked(&self, ix: usize) -> bool {
        matches!(self.fields[ix], Field::Check { on: true, .. })
    }

    /// Tab and Shift+Tab go round the fields; Up and Down too, but in a
    /// line with history, which they bring back.
    pub fn key(&mut self, key: &KeyEvent) -> Answer {
        let count = self.fields.len();
        let step = |focus: usize, by: usize| (focus + by) % count;
        match (&mut self.fields[self.focus], key.code) {
            (_, KeyCode::Esc) => return Answer::Cancel,
            (_, KeyCode::Enter) => return Answer::Ok,
            (_, KeyCode::Tab) => self.focus = step(self.focus, 1),
            (_, KeyCode::BackTab) => self.focus = step(self.focus, count - 1),
            (
                Field::Text {
                    input,
                    history,
                    recalled,
                    ..
                },
                code @ (KeyCode::Up | KeyCode::Down),
            ) if !history.is_empty() => {
                let at = match (code, *recalled) {
                    (KeyCode::Up, None) => 0,
                    (KeyCode::Up, Some(at)) => (at + 1).min(history.len() - 1),
                    (_, None | Some(0)) => return Answer::Open,
                    (_, Some(at)) => at - 1,
                };
                *recalled = Some(at);
                input.clear();
                input.insert(&history[at]);
            }
            (_, KeyCode::Down) => self.focus = step(self.focus, 1),
            (_, KeyCode::Up) => self.focus = step(self.focus, count - 1),
            (Field::Check { on, .. }, KeyCode::Char(' ')) => *on = !*on,
            (Field::Text { input, .. }, _) => {
                input.handle(key);
            }
            _ => {}
        }
        Answer::Open
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let rows: u16 = self
            .fields
            .iter()
            .map(|field| match field {
                Field::Text { .. } => 2,
                Field::Check { .. } => 1,
            })
            .sum();
        let width = 60.min(area.width.saturating_sub(4)).max(20);
        let height = (rows + 4).min(area.height);
        let at = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width.min(area.width),
            height,
        );
        frame.render_widget(Clear, at);
        let block = Block::bordered()
            .title(format!(" {} ", self.title))
            .style(DIALOG)
            .padding(Padding::horizontal(1));
        let inner = block.inner(at);
        frame.render_widget(block, at);
        let room = inner.width as usize;
        let mut y = inner.y;
        let mut cursor = None;
        for (ix, field) in self.fields.iter().enumerate() {
            if y >= inner.bottom() {
                break;
            }
            let focused = ix == self.focus;
            let buf = frame.buffer_mut();
            match field {
                Field::Text { label, input, .. } => {
                    buf.set_string(inner.x, y, fit(label, room), DIALOG);
                    y += 1;
                    let chars: Vec<char> = input.text().chars().collect();
                    let start = input.cursor().saturating_sub(room.saturating_sub(1));
                    let shown: String = chars[start..].iter().collect();
                    buf.set_string(inner.x, y, fit(&shown, room), BAR);
                    if focused {
                        let before: String = chars[start..input.cursor()].iter().collect();
                        cursor = Some(Position::new(inner.x + before.width() as u16, y));
                    }
                }
                Field::Check { label, on } => {
                    let mark = if *on { "[x]" } else { "[ ]" };
                    let style = if focused { BAR } else { DIALOG };
                    buf.set_string(inner.x, y, fit(&format!("{mark} {label}"), room), style);
                }
            }
            y += 1;
        }
        let buttons = "Enter OK   Esc Cancel";
        let x = inner.x + inner.width.saturating_sub(buttons.width() as u16) / 2;
        frame
            .buffer_mut()
            .set_string(x, inner.bottom().saturating_sub(1), buttons, DIALOG);
        if let Some(cursor) = cursor {
            frame.set_cursor_position(cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    #[test]
    fn types_ticks_and_recalls() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let mut form = Form::new("Find")
            .text("Search for:", "", vec!["last".into(), "older".into()])
            .check("Case sensitive", false);
        form.key(&key(KeyCode::Up));
        form.key(&key(KeyCode::Up));
        assert_eq!(form.value(0), "older");
        form.key(&key(KeyCode::Down));
        assert_eq!(form.value(0), "last");
        form.key(&key(KeyCode::Char('!')));
        form.key(&key(KeyCode::Tab));
        form.key(&key(KeyCode::Char(' ')));
        assert!(form.checked(1));
        assert_eq!(form.key(&key(KeyCode::Enter)), Answer::Ok);
        assert_eq!(form.value(0), "last!");
    }
}
