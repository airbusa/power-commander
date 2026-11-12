//! Right-click on files, which every system's menu shares: what it is for,
//! and Rename. Windows: Explorer's own menu, as Total Commander shows it.

use std::path::Path;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::*;

use crate::{Commander, name_dialog};

impl Commander {
    /// The right-clicked row of panel `ix` is what the menu is for: the
    /// marked ones if it is one of them, else it alone, as Finder and
    /// Explorer select.
    pub(crate) fn right_click(&mut self, ix: usize, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        self.menu_on_row = true;
        window.focus(&self.focus, cx);
        let panel = self.panels[ix].tabs.current_mut();
        if panel.entries().get(row).is_some_and(|entry| !panel.is_marked(entry)) {
            panel.mark_all(false);
        }
        panel.set_cursor(row);
        cx.notify();
    }

    /// The cursor's entry gets a new name, here or on a server.
    pub(crate) fn rename_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        let Some(entry) = panel.current_entry().filter(|entry| !entry.is_parent()) else {
            return;
        };
        let name = entry.name.clone();
        // A search result's path keeps its folder.
        let shown = Path::new(&name).file_name().unwrap_or(&name).to_string_lossy().into_owned();
        let commander = cx.entity().downgrade();
        name_dialog::open("Rename", "New name:", shown, window, cx, move |to, window, cx| {
            let name = name.clone();
            let _ = commander.update(cx, |this, cx| {
                let panel = this.panels[side].tabs.current_mut();
                if panel.remote().is_some() {
                    let renamed = move |panel: &mut pc_core::Panel| panel.rename_entry(&name, &to);
                    return this.navigate_on(side, true, renamed, window, cx);
                }
                let folder = panel.folder().to_owned();
                if let Err(err) = panel.rename_entry(&name, &to) {
                    window.push_notification(Notification::error(format!("Could not rename: {err}")), cx);
                }
                this.refresh_folder(&folder, cx);
            });
        });
    }

    /// Panel `ix`'s files; off the entries, a right click shows the folder's menu.
    #[cfg(windows)]
    pub(crate) fn with_shell_menu(&self, ix: usize, files: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.active = ix;
                    this.shell_menu(ix, None, window, cx)
                }),
            )
            .child(files)
            .into_any_element()
    }

    /// Explorer's menu of the chosen entries at `row`, or of the folder off them.
    #[cfg(windows)]
    pub(crate) fn shell_menu(&mut self, side: usize, row: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        use pc_core::shell_menu::{self, Choice};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let panel = self.panels[side].tabs.current();
        // shortcut: no menu on servers and in archives yet; give them the app's own items when asked.
        if panel.remote().is_some() || panel.in_archive() {
            return;
        }
        let Ok(RawWindowHandle::Win32(handle)) = window.window_handle().map(|handle| handle.as_raw()) else {
            return;
        };
        let owner = handle.hwnd.get();
        let folder = panel.folder().to_owned();
        let on_entry = row.and_then(|row| panel.entries().get(row)).is_some_and(|e| !e.is_parent());
        let chosen = panel.chosen();
        let (paths, header, background) = match on_entry {
            false => (vec![folder.clone()], None, true),
            true => {
                let header = match chosen.as_slice() {
                    [one] => Some((one.display_name(), one.is_dir)),
                    _ => None,
                };
                (chosen.iter().map(|e| folder.join(&e.name)).collect(), header, false)
            }
        };
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let header = header.as_ref().map(|(name, is_dir)| (name.as_str(), *is_dir));
            sender.send(shell_menu::show(owner, &paths, background, header)).ok();
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(chosen) = receiver.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| match chosen {
                Ok(None) => {}
                Ok(Some(Choice::Open)) => this.enter(window, cx),
                Ok(Some(Choice::View)) => this.open_file(crate::remote::Open::View, window, cx),
                Ok(Some(Choice::Pack)) => this.pack(window, cx),
                Ok(Some(Choice::Rename)) => this.rename_file(window, cx),
                Ok(Some(Choice::Delete)) => this.confirm_delete(true, window, cx),
                Ok(Some(Choice::DeletePermanently)) => this.confirm_delete(false, window, cx),
                Ok(Some(Choice::Done)) => this.refresh_folder(&folder, cx),
                Err(err) => window.push_notification(Notification::error(format!("The menu: {err}")), cx),
            });
        })
        .detach();
    }
}
