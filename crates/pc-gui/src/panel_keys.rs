//! Total Commander's keys for the panels: swapping them, one's folder in
//! the other (Ctrl+Left and Right), the root, a folder in a new tab, names
//! onto the command line, a new text file in the editor.

use std::ffi::OsString;
use std::path::Path;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{IconName, Sizable, WindowExt, h_flex};
use gpui_kit::*;
use pc_core::menu::Command;
use pc_core::{FileFilter, Panel, View};

use crate::{CancelRename, Commander, name_dialog, remote};

/// Shift+F6: a name being changed in its row, as Total Commander's.
pub(crate) struct Renaming {
    side: usize,
    /// The entry's, which finds its row however the list changes meanwhile.
    name: OsString,
    input: Entity<InputState>,
    _events: Subscription,
}

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

    /// Shift+F6: the name at the cursor edited in its row, the file's name
    /// without its extension chosen; the Brief and Thumbnails views ask in a dialog.
    pub(crate) fn rename_in_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        if panel.view() != View::Full {
            return self.rename_file(window, cx);
        }
        let Some(entry) = panel.current_entry().filter(|entry| !entry.is_parent()) else {
            return;
        };
        let name = entry.name.clone();
        // A search result's path keeps its folder.
        let shown = Path::new(&name).file_name().unwrap_or(&name).to_string_lossy().into_owned();
        let stem = match entry.is_dir {
            true => shown.len(),
            false => shown.rfind('.').filter(|&dot| dot > 0).unwrap_or(shown.len()),
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(shown));
        // Enter, or clicking elsewhere, renames; Escape (`CancelRename`) does not.
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(true, window, cx);
            }
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_selected_range(0..stem, cx);
        });
        self.renaming = Some(Renaming { side, name, input, _events: events });
        cx.notify();
    }

    pub(crate) fn finish_rename(&mut self, apply: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        window.focus(&self.focus, cx);
        cx.notify();
        let to = renaming.input.read(cx).value().trim().to_owned();
        let shown = Path::new(&renaming.name).file_name().map(|name| name.to_string_lossy().into_owned());
        if !apply || to.is_empty() || shown.as_deref() == Some(to.as_str()) {
            return;
        }
        let (side, name) = (renaming.side, renaming.name);
        let panel = self.panels[side].tabs.current_mut();
        if panel.remote().is_some() {
            let renamed = move |panel: &mut Panel| panel.rename_entry(&name, &to);
            return self.navigate_on(side, true, renamed, window, cx);
        }
        let folder = panel.folder().to_owned();
        if let Err(err) = panel.rename_entry(&name, &to) {
            window.push_notification(Notification::error(format!("Could not rename: {err}")), cx);
        }
        self.refresh_folder(&folder, cx);
    }

    /// The row's name and extension cells while it is renamed: the name in a
    /// field `width` wide, and a check mark that renames.
    pub(crate) fn rename_cell(&self, ix: usize, row: usize, width: Pixels, icon: AnyElement, cx: &mut Context<Self>) -> Result<AnyElement, AnyElement> {
        let renaming = self.renaming.as_ref().filter(|renaming| {
            let entry = self.panels[ix].tabs.current().entries().get(row);
            renaming.side == ix && entry.is_some_and(|entry| entry.name == renaming.name)
        });
        let Some(renaming) = renaming else {
            return Err(icon);
        };
        let cell = h_flex()
            .w(width)
            .flex_none()
            .px_1()
            .gap_1()
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .key_context("RenameInPlace")
                    .on_action(cx.listener(|this, _: &CancelRename, window, cx| this.finish_rename(false, window, cx)))
                    // Not the row's: a click here neither moves the cursor nor ends the edit.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Input::new(&renaming.input).xsmall()),
            )
            .child(
                Button::new("rename-ok")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Check)
                    .on_click(cx.listener(|this, _, window, cx| this.finish_rename(true, window, cx))),
            );
        Ok(cell.into_any_element())
    }

    /// The Show menu's filters: the active panel lists only these files.
    pub(crate) fn show_files(&mut self, filter: FileFilter, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        self.navigate_on(self.active, remote, move |panel| panel.set_file_filter(filter), window, cx);
        self.reload_menus(cx);
    }

    /// Custom...: file types typed, kept as the menu's next filter item.
    pub(crate) fn custom_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        let label = "File types, e.g. *.txt *.md (after |, those left out):";
        name_dialog::open("Show files", label, crate::menus::last_filter(), window, cx, move |masks, window, cx| {
            let masks = masks.trim().to_owned();
            if masks.is_empty() {
                return;
            }
            let _ = commander.update(cx, |this, cx| {
                crate::menus::set_last_filter(&masks);
                this.settings.last_filter = masks.clone();
                this.save_settings(window, cx);
                this.show_files(FileFilter::Masks(masks), window, cx);
            });
        });
    }

    /// The Show menu's check mark: the active panel's filter.
    pub(crate) fn filter_checked(&self, command: Command) -> bool {
        let filter = self.panels[self.active].tabs.current().file_filter();
        match (command, filter) {
            (Command::AllFiles, FileFilter::All) => true,
            (Command::Programs, FileFilter::Programs) => true,
            (Command::AllWithExtension, FileFilter::Masks(masks)) => *masks == crate::menus::last_filter(),
            (Command::CustomFilter, FileFilter::Masks(masks)) => *masks != crate::menus::last_filter(),
            (Command::OnlySelectedFiles, FileFilter::Selected(_)) => true,
            _ => false,
        }
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
