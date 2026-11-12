//! Linux and the BSDs: the files' right-click menu as the desktop's file
//! manager has it, its Open With and actions read from the system
//! (pc-core's desktop), with Total Commander's own items.

use std::io;
use std::path::PathBuf;

use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex};
use gpui_kit::*;
use pc_core::desktop::{self, Action as DesktopAction, App};
use pc_core::menu::Command;
use pc_core::{archive, properties};

use crate::menus::Run;
use crate::{Commander, Open, name_dialog};

actions!(desktop_menu, [OtherApplication, NewFile, OpenTerminalHere, FolderProperties, ShowInFileManager, RenameFile]);

/// Opens the chosen files in this application.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = desktop_menu, no_json)]
pub struct OpenWithApp(pub App);

/// One of the desktop's actions on the chosen files.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = desktop_menu, no_json)]
pub struct RunAction(pub DesktopAction);

/// A new file from this template.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = desktop_menu, no_json)]
pub struct NewFromTemplate(pub PathBuf);

/// What the menu is for, as it opens.
struct Target {
    /// Off the entries, or on `..`: the folder itself.
    folder: bool,
    /// On this computer, not on a server nor in an archive.
    local: bool,
    in_archive: bool,
    /// The one entry's name, and whether it is a file.
    single: Option<(String, bool)>,
    /// All chosen are archives.
    archives: bool,
    apps: Vec<App>,
    actions: Vec<DesktopAction>,
    templates: Vec<PathBuf>,
}

impl Commander {
    /// Panel `ix`'s files, with the menu a right click opens.
    pub(crate) fn with_desktop_menu(&self, ix: usize, files: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let (commander, focus) = (cx.entity().downgrade(), self.focus.clone());
        div()
            .id(("files", ix))
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // Before a row hears the press: off the rows, the menu is the folder's.
            .capture_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Right {
                    this.menu_on_row = false;
                    this.active = ix;
                }
            }))
            .child(files)
            .context_menu(move |menu, window, cx| {
                let menu = menu.action_context(focus.clone());
                match commander.upgrade() {
                    Some(commander) => build(menu, commander.read(cx).menu_target(ix), window, cx),
                    None => menu,
                }
            })
            .into_any_element()
    }

    fn menu_target(&self, ix: usize) -> Target {
        let panel = self.panels[ix].tabs.current();
        let chosen = panel.chosen();
        let folder = !self.menu_on_row || chosen.is_empty();
        let local = panel.remote().is_none() && !panel.in_archive();
        let paths: Vec<(PathBuf, bool)> = match folder {
            true => vec![(panel.folder().to_owned(), true)],
            false => chosen.iter().map(|e| (panel.folder().join(&e.name), e.is_dir)).collect(),
        };
        Target {
            folder,
            local,
            in_archive: panel.in_archive(),
            single: match chosen.as_slice() {
                [one] if !folder => Some((one.display_name(), !one.is_dir)),
                _ => None,
            },
            archives: !folder && chosen.iter().all(|e| !e.is_dir && archive::is_archive(&e.name)),
            apps: if local && !folder { desktop::open_with(&paths) } else { Vec::new() },
            actions: if local { desktop::actions(&paths) } else { Vec::new() },
            templates: if folder && !panel.in_archive() { desktop::templates() } else { Vec::new() },
        }
    }

    /// What the menu's commands work on: the chosen files, or the folder
    /// when the menu is the folder's; and the folder.
    fn menu_paths(&self) -> Option<(Vec<PathBuf>, PathBuf)> {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return None;
        }
        let folder = panel.folder().to_owned();
        let chosen = panel.chosen();
        let paths = match self.menu_on_row && !chosen.is_empty() {
            true => chosen.iter().map(|e| folder.join(&e.name)).collect(),
            false => vec![folder.clone()],
        };
        Some((paths, folder))
    }

    /// The menu's own commands, beside the panels' `Run`s.
    pub(crate) fn desktop_menu_actions(&self, panels: Div, cx: &mut Context<Self>) -> Div {
        panels
            .on_action(cx.listener(|this, OpenWithApp(app): &OpenWithApp, window, cx| {
                let app = app.clone();
                this.on_menu_paths("Could not open", move |paths, folder| app.open(&paths, &folder), window, cx)
            }))
            .on_action(cx.listener(|this, RunAction(action): &RunAction, window, cx| {
                let action = action.clone();
                let failed = format!("{} failed", action.name);
                this.on_menu_paths(failed, move |paths, folder| action.run(&paths, &folder), window, cx)
            }))
            .on_action(cx.listener(|this, _: &OtherApplication, window, cx| {
                this.on_menu_paths("Could not open", |paths, _| desktop::choose_app(&paths[0]), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowInFileManager, window, cx| {
                this.on_menu_paths("Could not show it", |paths, _| desktop::reveal(&paths), window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenTerminalHere, window, cx| this.open_terminal(window, cx)))
            .on_action(cx.listener(|this, _: &FolderProperties, window, cx| {
                this.on_menu_paths("Properties", |_, folder| properties::show_in_system(&[folder]), window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(None, window, cx)))
            .on_action(cx.listener(|this, NewFromTemplate(template): &NewFromTemplate, window, cx| {
                this.new_file(Some(template.clone()), window, cx)
            }))
            .on_action(cx.listener(|this, _: &RenameFile, window, cx| this.rename_file(window, cx)))
    }

    /// `run` away from the window, on what the menu was for.
    fn on_menu_paths(
        &mut self,
        failed: impl Into<String>,
        run: impl FnOnce(Vec<PathBuf>, PathBuf) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((paths, folder)) = self.menu_paths() else {
            return;
        };
        let failed = failed.into();
        let task = cx.background_executor().spawn(async move { run(paths, folder) });
        cx.spawn_in(window, async move |_, cx| {
            if let Err(err) = task.await {
                let _ = cx.update(|window, cx| window.push_notification(Notification::error(format!("{failed}: {err}")), cx));
            }
        })
        .detach();
    }

    /// The desktop's terminal in the active panel's folder (Commands › Open Terminal).
    pub(crate) fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() {
            let text = "A terminal opens only in this computer's folders.";
            window.push_notification(Notification::warning(text), cx);
            return;
        }
        if let Err(err) = desktop::open_terminal(panel.folder()) {
            window.push_notification(Notification::error(format!("Could not open a terminal: {err}")), cx);
        }
    }

    /// A new file in the active panel's folder, empty or from a template.
    fn new_file(&mut self, template: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let name = template.as_ref().and_then(|t| t.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let commander = cx.entity().downgrade();
        name_dialog::open("New file", "New file name:", name, window, cx, move |name, window, cx| {
            let template = template.clone();
            let _ = commander.update(cx, |this, cx| {
                let panel = this.panels[side].tabs.current_mut();
                if panel.remote().is_some() {
                    let made = move |panel: &mut pc_core::Panel| panel.new_file(&name, template.as_deref());
                    return this.navigate_on(side, true, made, window, cx);
                }
                let folder = panel.folder().to_owned();
                if let Err(err) = panel.new_file(&name, template.as_deref()) {
                    window.push_notification(Notification::error(format!("Could not make the file: {err}")), cx);
                }
                this.refresh_folder(&folder, cx);
            });
        });
    }
}

/// Dolphin's order: open, the desktop's actions, clipboard, archive,
/// rename and trash, properties.
fn build(menu: PopupMenu, target: Target, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let Target { folder, local, in_archive, single, archives, apps, actions, templates } = target;
    if folder {
        let mut menu = menu;
        if !in_archive {
            menu = menu.submenu("Create New", window, cx, move |menu, _, _| {
                let menu = menu.menu("Folder…", Box::new(Run(Command::NewFolder))).menu("Empty File…", Box::new(NewFile)).separator();
                templates.iter().fold(menu, |menu, template| {
                    let name = template.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    menu.menu(name, Box::new(NewFromTemplate(template.clone())))
                })
            });
        }
        menu = menu.menu("Paste", Box::new(Run(Command::PasteFromClipboard))).separator();
        let mut actions = actions;
        if local {
            menu = menu.menu("Open Terminal Here", Box::new(OpenTerminalHere));
            // Xfce's default actions have one too.
            actions.retain(|action| !action.name.eq_ignore_ascii_case("Open Terminal Here"));
        }
        let properties: Box<dyn Action> = match local {
            true => Box::new(FolderProperties),
            false => Box::new(Run(Command::Properties)),
        };
        return with_actions(menu, actions, window, cx).separator().menu("Properties", properties);
    }
    let mut menu = match &single {
        Some((name, _)) => {
            let name = SharedString::from(name.clone());
            menu.menu_element(Box::new(Open), move |_, _| div().font_weight(FontWeight::BOLD).child(name.clone()))
        }
        None => menu.menu("Open", Box::new(Open)),
    };
    // The desktop's app chooser takes one file.
    let other = matches!(single, Some((_, true)));
    if local && (!apps.is_empty() || other) {
        menu = menu.submenu("Open With", window, cx, move |menu, _, _| {
            let mut menu = menu;
            for (at, app) in apps.iter().enumerate() {
                menu = item(menu, app.name.clone(), app.icon.clone(), Box::new(OpenWithApp(app.clone())));
                // The default first, on its own.
                if at == 0 && apps.len() > 1 {
                    menu = menu.separator();
                }
            }
            match other {
                true => item(menu.separator(), "Other Application…", None, Box::new(OtherApplication)),
                false => menu,
            }
        });
    }
    menu = menu.separator().menu("View", Box::new(Run(Command::View))).menu("Edit", Box::new(Run(Command::Edit))).separator();
    menu = with_actions(menu, actions, window, cx).separator();
    menu = menu.menu("Cut", Box::new(Run(Command::CutToClipboard))).menu("Copy", Box::new(Run(Command::CopyToClipboard))).separator();
    menu = menu.menu("Pack…", Box::new(Run(Command::Pack)));
    if archives {
        menu = menu.menu("Unpack…", Box::new(Run(Command::UnpackSpecificFiles)));
    }
    menu = menu.separator();
    if single.is_some() && !in_archive {
        menu = menu.menu("Rename…", Box::new(RenameFile));
    }
    let delete = if local { "Move to Trash" } else { "Delete" };
    menu = menu.menu(delete, Box::new(Run(Command::Delete))).separator();
    if local {
        menu = menu.menu("Show in File Manager", Box::new(ShowInFileManager));
    }
    menu.menu("Properties", Box::new(Run(Command::Properties)))
}

/// The desktop's actions: the top-level ones here, the rest in Actions;
/// those of a submenu under its name.
fn with_actions(menu: PopupMenu, actions: Vec<DesktopAction>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let (top, nested): (Vec<_>, Vec<_>) = actions.into_iter().partition(|action| action.top_level);
    let menu = group(menu, top, false, window, cx);
    match nested.is_empty() {
        true => menu,
        false => menu.submenu("Actions", window, cx, move |menu, window, cx| group(menu, nested.clone(), true, window, cx)),
    }
}

fn group(mut menu: PopupMenu, actions: Vec<DesktopAction>, icons: bool, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let mut submenus: Vec<(String, Vec<DesktopAction>)> = Vec::new();
    for action in actions {
        match action.submenu.clone() {
            Some(name) => match submenus.iter_mut().find(|(known, _)| *known == name) {
                Some((_, list)) => list.push(action),
                None => submenus.push((name, vec![action])),
            },
            None => menu = entry(menu, action, icons),
        }
    }
    for (name, list) in submenus {
        menu = menu.submenu(name, window, cx, move |menu, _, _| list.iter().cloned().fold(menu, |menu, action| entry(menu, action, true)));
    }
    menu
}

fn entry(menu: PopupMenu, action: DesktopAction, icon: bool) -> PopupMenu {
    let (name, picture) = (action.name.clone(), action.icon.clone());
    match icon {
        true => item(menu, name, picture, Box::new(RunAction(action))),
        false => menu.menu(name, Box::new(RunAction(action))),
    }
}

/// An item with the application's or action's own picture, as the desktop shows it.
fn item(menu: PopupMenu, label: impl Into<SharedString>, picture: Option<PathBuf>, action: Box<dyn Action>) -> PopupMenu {
    let label = label.into();
    menu.menu_element(action, move |_, _| {
        let picture = match &picture {
            Some(path) => img(path.clone()).size(px(16.)).flex_none().into_any_element(),
            None => div().size(px(16.)).flex_none().into_any_element(),
        };
        h_flex().gap_2().child(picture).child(label.clone())
    })
}
