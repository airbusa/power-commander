//! A line of text being typed: the command line, and dialogs' fields.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
pub struct Input {
    text: String,
    /// In characters.
    cursor: usize,
}

impl Input {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, text: &str) {
        let at = self.byte(self.cursor);
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    /// The editing keys; false for any other.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Char(c) if plain => self.insert(c.encode_utf8(&mut [0; 4])),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.text.remove(self.byte(self.cursor));
            }
            KeyCode::Delete if self.cursor < len => {
                self.text.remove(self.byte(self.cursor));
            }
            _ => return false,
        }
        true
    }

    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(at, _)| at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_by_characters() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let mut input = Input::default();
        for c in "ls ő".chars() {
            input.handle(&key(KeyCode::Char(c)));
        }
        input.handle(&key(KeyCode::Left));
        input.handle(&key(KeyCode::Backspace));
        input.insert("-l ");
        assert_eq!((input.text(), input.cursor()), ("ls-l ő", 5));
        input.handle(&key(KeyCode::Delete));
        assert_eq!(input.text(), "ls-l ");
        assert!(!input.handle(&key(KeyCode::Delete)), "nothing after it");
        assert!(!input.handle(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
    }
}
