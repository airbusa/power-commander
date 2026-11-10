//! The Mark menu: groups by mask, the same extension, selections saved
//! and restored, names to the clipboard, and folders compared.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use pc_core::menu::Command;
use pc_core::{Panel, command};

use crate::{Commander, name_dialog};

impl Commander {
    /// The Mark menu's commands; false for others.
    pub(crate) fn mark_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match command {
            Command::SelectGroup => self.mark_group(true, window, cx),
            Command::UnselectGroup => self.mark_group(false, window, cx),
            Command::SelectAllWithSameExtension => self.mark(Panel::mark_same_extension, cx),
            Command::SaveSelection => {
                self.saved_selection = self.panels[self.active].tabs.current().marked_names();
            }
            Command::RestoreSelection => {
                let saved = self.saved_selection.clone();
                self.mark(|panel| panel.mark_names(&saved), cx);
            }
            Command::SaveSelectionToFile => self.selection_file(true, window, cx),
            Command::LoadSelectionFromFile => self.selection_file(false, window, cx),
            Command::CopyNamesToClipboard => self.copy_names(false, false, cx),
            Command::CopyNamesWithPathToClipboard => self.copy_names(true, false, cx),
            Command::CopyWithAllDetails => self.copy_names(false, true, cx),
            Command::CopyWithPathAndDetails => self.copy_names(true, true, cx),
            Command::CompareDirectories => self.compare_folders(false, window, cx),
            Command::MarkNewerHideSame => self.compare_folders(true, window, cx),
            _ => return false,
        }
        true
    }

    fn mark_group(&mut self, mark: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (title, label) = match mark {
            true => (
                "Select group",
                "Select the files (*.txt *.md; a folder mask ends in /):",
            ),
            false => (
                "Unselect group",
                "Unselect the files (*.txt *.md; a folder mask ends in /):",
            ),
        };
        let commander = cx.entity().downgrade();
        name_dialog::open(title, label, "*".into(), window, cx, move |mask, _, cx| {
            let _ = commander.update(cx, |this, cx| {
                this.mark(|panel| panel.mark_group(&mask, mark), cx)
            });
        });
    }

    fn selection_file(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            let text = "Selection files are kept on this computer, outside archives.";
            return window.push_notification(Notification::warning(text), cx);
        }
        let folder = panel.folder().to_owned();
        let file = folder.join("selection.txt").display().to_string();
        let (title, label) = match save {
            true => ("Save selection", "Save the selected names to the file:"),
            false => ("Load selection", "Select the names listed in the file:"),
        };
        let commander = cx.entity().downgrade();
        name_dialog::open(title, label, file, window, cx, move |text, window, cx| {
            let path = command::resolve(text.trim(), &folder);
            let _ = commander.update(cx, |this, cx| {
                let done = match save {
                    true => {
                        let names = this.panels[this.active].tabs.current().marked_names();
                        pc_core::save_selection(&path, &names)
                            .map(|()| this.reread(this.active, cx))
                    }
                    false => pc_core::load_selection(&path)
                        .map(|names| this.mark(|panel| panel.mark_names(&names), cx)),
                };
                if let Err(err) = done {
                    let text = format!("{}: {err}", path.display());
                    window.push_notification(Notification::error(text), cx);
                }
            });
        });
    }

    fn copy_names(&mut self, path: bool, details: bool, cx: &mut Context<Self>) {
        let text = self.panels[self.active]
            .tabs
            .current()
            .names_text(path, details);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Marks on both sides what differs; with `hide_same` the same files leave the lists.
    fn compare_folders(&mut self, hide_same: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (left, right) = self.panels.split_at_mut(1);
        let (left, right) = (left[0].tabs.current_mut(), right[0].tabs.current_mut());
        let count = left.compare_with(right, hide_same);
        let text = match count {
            0 => "The folders have the same files.".to_string(),
            n => format!("{n} file(s) marked: newer than, or missing on, the other side."),
        };
        window.push_notification(Notification::info(text), cx);
        for side in &self.panels {
            side.scroll_to_cursor();
        }
        cx.notify();
    }
}
