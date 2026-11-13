//! Total Commander's keys for the panels: swapping them, one's folder in
//! the other (Ctrl+Left and Right), the root, a folder in a new tab, names
//! onto the command line, a new text file in the editor.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use pc_core::Panel;

use crate::{Commander, name_dialog, remote};

/// A folder at the cursor to open, not `..` nor a macOS application.
fn at_folder(panel: &Panel) -> bool {
    panel.current_entry().is_some_and(|entry| entry.is_dir && !entry.is_parent() && !entry.is_app())
}

impl Commander {
    /// Ctrl+R and F2: the folder read again.
    pub(crate) fn reread_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        self.navigate_on(self.active, remote, Panel::reload, window, cx);
    }

    /// Ctrl+U: the panels' folders change places; Ctrl+Shift+U, with all their tabs.
    pub(crate) fn swap_panels(&mut self, with_tabs: bool, cx: &mut Context<Self>) {
        let [left, right] = &mut self.panels;
        match with_tabs {
            true => std::mem::swap(&mut left.tabs, &mut right.tabs),
            false => std::mem::swap(left.tabs.current_mut(), right.tabs.current_mut()),
        }
        for side in &mut self.panels {
            side.scroll_to_cursor();
        }
        cx.notify();
    }

    /// Target = Source: the other panel shows this one's folder.
    pub(crate) fn target_equals_source(&mut self, cx: &mut Context<Self>) {
        let copy = self.panels[self.active].tabs.current().copy_elsewhere();
        let other = &mut self.panels[1 - self.active];
        let tab = other.tabs.active();
        other.tabs.navigated(tab, copy);
        other.scroll_to_cursor();
        cx.notify();
    }

    /// Ctrl+Left and Ctrl+Right, as Total Commander's: panel `target` shows
    /// the folder at the other panel's cursor, else that panel's own folder.
    /// The active panel stays so.
    pub(crate) fn show_in(&mut self, target: usize, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.panels[1 - target].tabs.current();
        let folder = at_folder(source);
        let copy = source.copy_elsewhere();
        let remote = copy.remote().is_some();
        let side = &mut self.panels[target];
        let tab = side.tabs.active();
        side.tabs.navigated(tab, copy);
        side.scroll_to_cursor();
        if folder {
            return self.navigate_on(target, remote, |panel| panel.open().map(drop), window, cx);
        }
        cx.notify();
    }

    /// Ctrl+\ and Ctrl+Backspace.
    pub(crate) fn go_to_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        self.navigate_on(self.active, remote, Panel::go_to_root, window, cx);
    }

    /// Ctrl+Up: the folder at the cursor in a new tab; Ctrl+Shift+Up, in one
    /// of the other panel. On a file, this folder.
    pub(crate) fn open_in_new_tab(&mut self, other: bool, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.panels[self.active].tabs.current();
        let folder = at_folder(source);
        let copy = source.copy_elsewhere();
        let remote = copy.remote().is_some();
        let target = if other { 1 - self.active } else { self.active };
        self.panels[target].tabs.add(copy);
        self.panels[target].scroll_to_cursor();
        if folder {
            return self.navigate_on(target, remote, |panel| panel.open().map(drop), window, cx);
        }
        cx.notify();
    }

    /// Ctrl+P, Ctrl+Enter and Ctrl+Shift+Enter: the folder, the name at the
    /// cursor or both onto the command line, quoted with a space in them.
    pub(crate) fn to_command_line(&mut self, folder: bool, name: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let entry = panel.current_entry().filter(|entry| !entry.is_parent()).map(|entry| entry.display_name());
        let text = match (folder, entry) {
            (true, Some(entry)) if name => panel.folder().join(entry).display().to_string(),
            (false, Some(entry)) => entry,
            (false, None) => return,
            _ => {
                let path = panel.folder().display().to_string();
                match path.ends_with(std::path::MAIN_SEPARATOR) {
                    true => path,
                    false => format!("{path}{}", std::path::MAIN_SEPARATOR),
                }
            }
        };
        let text = match text.contains(' ') {
            true => format!("\"{text}\""),
            false => text,
        };
        self.command.update(cx, |input, cx| {
            let line = input.value();
            let line = match line.is_empty() || line.ends_with(' ') {
                true => format!("{line}{text}"),
                false => format!("{line} {text}"),
            };
            input.set_value(line, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Shift+F4: a text file, new or there already, in the editor.
    pub(crate) fn new_text_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let commander = cx.entity().downgrade();
        name_dialog::open("New text file", "File name, opened in the editor:", String::new(), window, cx, move |name, window, cx| {
            let name = name.trim().to_owned();
            if name.is_empty() {
                return;
            }
            let _ = commander.update(cx, |this, cx| {
                let panel = this.panels[side].tabs.current_mut();
                // shortcut: on a server the file is made, not opened; F4 opens it then.
                if panel.remote().is_some() {
                    let made = move |panel: &mut Panel| panel.new_file(&name, None);
                    return this.navigate_on(side, true, made, window, cx);
                }
                let folder = panel.folder().to_owned();
                let path = folder.join(&name);
                let made = match path.exists() {
                    true => panel.go_to_file(&path, None),
                    false => panel.new_file(&name, None),
                };
                if let Err(err) = made {
                    window.push_notification(Notification::error(format!("Could not make the file: {err}")), cx);
                    return;
                }
                this.refresh_folder(&folder, cx);
                this.active = side;
                this.open_file(remote::Open::Edit, window, cx);
            });
        });
    }
}
