//! The mouse as a terminal program asks for it (mc, htop, vim): Zed's
//! crates/terminal/src/mappings/mouse.rs (GPL-3.0-or-later), from Alacritty's.

use alacritty_terminal::term::TermMode;
use gpui_kit::{Modifiers, MouseButton};

pub enum Event {
    /// Pressed, or released.
    Button(MouseButton, bool),
    /// A move, with the button held if any.
    Move(Option<MouseButton>),
    /// A wheel step: up, or down.
    Wheel(bool),
}

/// The bytes for `event` at the screen's `column` and `line`; none when the
/// program did not ask for it.
pub fn report(event: Event, column: usize, line: usize, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let (code, pressed) = match event {
        Event::Button(button, pressed) if mode.intersects(TermMode::MOUSE_MODE) => (button_code(button)?, pressed),
        // Drags only, unless every move was asked for.
        Event::Move(button) if mode.contains(TermMode::MOUSE_MOTION) || mode.contains(TermMode::MOUSE_DRAG) && button.is_some() => {
            (32 + button.map_or(Some(3), button_code)?, true)
        }
        Event::Wheel(up) if mode.intersects(TermMode::MOUSE_MODE) => (if up { 64 } else { 65 }, true),
        _ => return None,
    };
    let mods = 4 * modifiers.shift as u8 + 8 * modifiers.alt as u8 + 16 * modifiers.control as u8;
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{};{};{}{end}", code + mods, column + 1, line + 1).into_bytes());
    }
    // The old encoding has no release of its own.
    let code = if pressed { code } else { 3 } + mods;
    normal(code, column, line, mode.contains(TermMode::UTF8_MOUSE))
}

fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// X10's: a byte each, or two past 95 in UTF-8 mode.
fn normal(code: u8, column: usize, line: usize, utf8: bool) -> Option<Vec<u8>> {
    let max = if utf8 { 2015 } else { 223 };
    if line >= max || column >= max {
        return None;
    }
    let mut message = vec![b'\x1b', b'[', b'M', 32 + code];
    for at in [column, line] {
        match utf8 && at >= 95 {
            true => {
                let at = 33 + at;
                message.extend([(0xC0 + at / 64) as u8, (0x80 + (at & 63)) as u8]);
            }
            false => message.push(33 + at as u8),
        }
    }
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_as_asked() {
        let none = Modifiers::default();
        assert_eq!(report(Event::Wheel(true), 0, 0, none, TermMode::NONE), None, "not asked");
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(report(Event::Button(MouseButton::Left, false), 4, 2, none, sgr).unwrap(), b"\x1b[<0;5;3m");
        assert_eq!(report(Event::Button(MouseButton::Left, true), 0, 0, none, TermMode::MOUSE_REPORT_CLICK).unwrap(), b"\x1b[M !!");
        assert_eq!(report(Event::Move(None), 0, 0, none, TermMode::MOUSE_DRAG), None, "drags only");
    }
}
