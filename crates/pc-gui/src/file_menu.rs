//! macOS: the files' right-click menu, as Finder's: what is right-clicked,
//! or what is marked if that is, opened, shown, renamed, copied.

use std::path::{Path, PathBuf};

use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use pc_core::finder;
use pc_core::menu::Command;

use crate::menus::Run;
use crate::{Commander, Open, name_dialog};

actions!(file_menu, [ShowPackageContents, RenameFile, Duplicate, MakeAlias, QuickLook, RevealInFinder]);

/// Opens them with this application.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = file_menu, no_json)]
pub struct OpenWith(pub PathBuf);

/// What the menu is for, as it opens.
struct Chosen {
    /// The cursor on `..`: the folder itself.
    folder: bool,
    /// On this Mac, not on a server nor in an archive.
    local: bool,
    in_archive: bool,
    /// The one entry's name.
    single: Option<String>,
    count: usize,
    /// The one entry is an application.
    app: bool,
    /// Whose applications Open With lists.
    first: Option<PathBuf>,
}

impl Commander {
    /// The right-clicked row of panel `ix` is what the menu is for: the
    /// marked ones if it is one of them, else it alone, as Finder selects.
    pub(crate) fn right_click(&mut self, ix: usize, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        window.focus(&self.focus, cx);
        let panel = self.panels[ix].tabs.current_mut();
        if panel.entries().get(row).is_some_and(|entry| !panel.is_marked(entry)) {
            panel.mark_all(false);
        }
        panel.set_cursor(row);
        cx.notify();
    }

    /// Panel `ix`'s files, with the menu a right click opens.
    pub(crate) fn with_file_menu(&self, ix: usize, files: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let (commander, focus) = (cx.entity().downgrade(), self.focus.clone());
        div()
            .id(("files", ix))
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(files)
            .context_menu(move |menu, window, cx| {
                let menu = menu.action_context(focus.clone());
                match commander.upgrade() {
                    Some(commander) => build(menu, commander.read(cx).chosen_for_menu(ix), window, cx),
                    None => menu,
                }
            })
            .into_any_element()
    }

    fn chosen_for_menu(&self, ix: usize) -> Chosen {
        let panel = self.panels[ix].tabs.current();
        let chosen = panel.chosen();
        let local = panel.remote().is_none() && !panel.in_archive();
        let single = match chosen.as_slice() {
            [entry] => Some(entry),
            _ => None,
        };
        Chosen {
            folder: chosen.is_empty(),
            local,
            in_archive: panel.in_archive(),
            // An application as Finder names it, without `.app`.
            single: single.map(|entry| match entry.is_app() {
                true => entry.name_and_extension().0,
                false => entry.display_name(),
            }),
            count: chosen.len(),
            app: single.is_some_and(|entry| entry.is_app()),
            first: chosen.first().map(|entry| panel.folder().join(&entry.name)),
        }
    }

    /// The active panel's chosen files, or its folder; on this Mac only.
    fn chosen_paths(&self) -> Option<Vec<PathBuf>> {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return None;
        }
        let paths: Vec<PathBuf> = panel.chosen().iter().map(|entry| panel.folder().join(&entry.name)).collect();
        Some(match paths.is_empty() {
            true => vec![panel.folder().to_owned()],
            false => paths,
        })
    }

    /// The menu's own commands, beside the panels' `Run`s.
    pub(crate) fn file_menu_actions(&self, panels: Div, cx: &mut Context<Self>) -> Div {
        panels
            .on_action(cx.listener(|this, _: &ShowPackageContents, window, cx| {
                let panel = this.panels[this.active].tabs.current();
                if let Some(entry) = panel.current_entry().filter(|entry| entry.is_app()) {
                    let path = panel.folder().join(&entry.name);
                    this.navigate(move |panel| panel.go_to_folder(path), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameFile, window, cx| this.rename_file(window, cx)))
            .on_action(cx.listener(|this, _: &Duplicate, window, cx| {
                this.in_background("Could not duplicate", |paths| finder::duplicate(&paths).map(drop), true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MakeAlias, window, cx| {
                this.in_background("Could not make the alias", |paths| finder::make_alias(&paths), true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickLook, window, cx| {
                this.in_background("Quick Look", |paths| finder::quick_look(&paths), false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, window, cx| {
                this.in_background("Could not show it in Finder", |paths| finder::reveal(&paths), false, window, cx)
            }))
            .on_action(cx.listener(|this, OpenWith(app): &OpenWith, window, cx| {
                let app = app.clone();
                this.in_background("Could not open", move |paths| finder::open_with(&app, &paths), false, window, cx)
            }))
    }

    /// `run` on the chosen files away from the window; `changes` the folder, which is read again then.
    fn in_background(
        &mut self,
        failed: &'static str,
        run: impl FnOnce(Vec<PathBuf>) -> std::io::Result<()> + Send + 'static,
        changes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(paths) = self.chosen_paths() else {
            return;
        };
        let folder = self.panels[self.active].tabs.current().folder().to_owned();
        let task = cx.background_executor().spawn(async move { run(paths) });
        cx.spawn_in(window, async move |this, cx| {
            let done = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(err) = done {
                    window.push_notification(Notification::error(format!("{failed}: {err}")), cx);
                }
                if changes {
                    this.refresh_folder(&folder, cx);
                }
            });
        })
        .detach();
    }

    /// The one chosen entry renamed, here or on a server.
    fn rename_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let chosen = self.panels[side].tabs.current().chosen();
        let [entry] = chosen.as_slice() else {
            return;
        };
        let name = entry.name.clone();
        let commander = cx.entity().downgrade();
        name_dialog::open("Rename", "New name:", entry.display_name(), window, cx, move |to, window, cx| {
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
}

/// Finder's menu, but its Share, Tags, Services and View Options.
fn build(menu: PopupMenu, chosen: Chosen, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    if chosen.folder {
        let menu = menu
            .menu("New Folder", Box::new(Run(Command::NewFolder)))
            .menu("Paste", Box::new(Run(Command::PasteFromClipboard)))
            .separator()
            .menu("Get Info", Box::new(Run(Command::Properties)));
        return match chosen.local {
            true => menu.menu("Reveal in Finder", Box::new(RevealInFinder)),
            false => menu,
        };
    }
    let what = match &chosen.single {
        Some(name) => format!("“{name}”"),
        None => format!("{} Items", chosen.count),
    };
    let mut menu = menu.menu("Open", Box::new(Open));
    if chosen.local {
        if let Some(first) = chosen.first.clone() {
            menu = menu.submenu("Open With", window, cx, move |menu, _, _| open_with(menu, &first));
        }
        if chosen.app {
            menu = menu.menu("Show Package Contents", Box::new(ShowPackageContents));
        }
    }
    let delete = if chosen.local { "Move to Trash" } else { "Delete" };
    menu = menu
        .separator()
        .menu(delete, Box::new(Run(Command::Delete)))
        .separator()
        .menu("Get Info", Box::new(Run(Command::Properties)));
    if chosen.single.is_some() && !chosen.in_archive {
        menu = menu.menu("Rename…", Box::new(RenameFile));
    }
    menu = menu.menu(format!("Compress {what}…"), Box::new(Run(Command::Pack)));
    if chosen.local {
        menu = menu
            .menu("Duplicate", Box::new(Duplicate))
            .menu("Make Alias", Box::new(MakeAlias))
            .menu(format!("Quick Look {what}"), Box::new(QuickLook))
            .menu("Reveal in Finder", Box::new(RevealInFinder));
    }
    menu.separator()
        .menu(format!("Copy {what}"), Box::new(Run(Command::CopyToClipboard)))
        .menu("Copy Path to Clipboard", Box::new(Run(Command::CopyNamesWithPathToClipboard)))
}

/// The applications macOS opens `file` with: its default first.
fn open_with(mut menu: PopupMenu, file: &Path) -> PopupMenu {
    let (default, mut others) = applications_for(file);
    others.retain(|app| Some(app) != default.as_ref());
    others.sort_by_key(|app| app_name(app).to_lowercase());
    others.dedup();
    if let Some(app) = default {
        menu = menu.menu(format!("{} (default)", app_name(&app)), Box::new(OpenWith(app)));
        if !others.is_empty() {
            menu = menu.separator();
        }
    }
    for app in others {
        menu = menu.menu(app_name(&app), Box::new(OpenWith(app)));
    }
    menu
}

fn app_name(app: &Path) -> String {
    app.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

/// Launch Services' default application for `file`, and all that open it.
fn applications_for(file: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let workspace = NSWorkspace::sharedWorkspace();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    let path = |url: &NSURL| url.path().map(|path| PathBuf::from(path.to_string()));
    let default = workspace.URLForApplicationToOpenURL(&url).and_then(|app| path(&app));
    let all = workspace.URLsForApplicationsToOpenURL(&url).iter().filter_map(|app| path(&app)).collect();
    (default, all)
}
