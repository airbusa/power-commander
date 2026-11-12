//! macOS: the files' right-click menu, as Finder's: what is right-clicked,
//! or what is marked if that is, opened, shown, renamed, copied.

use std::path::{Path, PathBuf};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use pc_core::finder;
use pc_core::menu::Command;

use crate::menus::Run;
use crate::{Commander, Open};

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
    /// Panel `ix`'s files; a right click on no row opens the folder's menu.
    pub(crate) fn with_file_menu(&self, ix: usize, files: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("files", ix))
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(files)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    let panel = this.panels[ix].tabs.current_mut();
                    panel.mark_all(false);
                    if panel.entries().first().is_some_and(|entry| entry.is_parent()) {
                        panel.set_cursor(0);
                    }
                    this.active = ix;
                    window.focus(&this.focus, cx);
                    this.show_file_menu(ix, window, cx);
                }),
            )
            .into_any_element()
    }

    /// AppKit's menu at the mouse; what is picked goes to the panels.
    pub(crate) fn show_file_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let entries = build(self.chosen_for_menu(ix), self.chosen_paths().unwrap_or_default());
        let focus = self.focus.clone();
        cx.notify();
        // Its own event loop runs while it is open: outside GPUI's update.
        cx.spawn_in(window, async move |_, cx| {
            if let Some(action) = native::pop_up(&entries) {
                let _ = cx.update(|window, cx| focus.dispatch_action(&*action, window, cx));
            }
        })
        .detach();
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
}

enum Entry {
    Item { title: String, icon: Option<Icon>, action: Box<dyn Action> },
    Submenu { title: String, entries: Vec<Entry> },
    /// The system's Share submenu for these files.
    Share(Vec<PathBuf>),
    Separator,
}

enum Icon {
    Symbol(&'static str),
    File(PathBuf),
}

fn item(title: impl Into<String>, action: impl Action) -> Entry {
    Entry::Item { title: title.into(), icon: None, action: Box::new(action) }
}

fn with_symbol(entry: Entry, symbol: &'static str) -> Entry {
    match entry {
        Entry::Item { title, action, .. } => Entry::Item { title, icon: Some(Icon::Symbol(symbol)), action },
        other => other,
    }
}

/// Finder's menu, but its Tags, Services and View Options.
fn build(chosen: Chosen, paths: Vec<PathBuf>) -> Vec<Entry> {
    use Entry::Separator;
    if chosen.folder {
        let mut menu = vec![
            item("New Folder", Run(Command::NewFolder)),
            item("Paste", Run(Command::PasteFromClipboard)),
            Separator,
            item("Get Info", Run(Command::Properties)),
        ];
        if chosen.local {
            menu.push(item("Reveal in Finder", RevealInFinder));
        }
        return menu;
    }
    let what = match &chosen.single {
        Some(name) => format!("“{name}”"),
        None => format!("{} Items", chosen.count),
    };
    let mut menu = vec![item("Open", Open)];
    if chosen.local {
        if let Some(first) = &chosen.first {
            menu.push(Entry::Submenu { title: "Open With".into(), entries: open_with(first) });
        }
        if chosen.app {
            menu.push(item("Show Package Contents", ShowPackageContents));
        }
    }
    let delete = if chosen.local { "Move to Trash" } else { "Delete" };
    menu.extend([Separator, item(delete, Run(Command::Delete)), Separator, item("Get Info", Run(Command::Properties))]);
    if chosen.single.is_some() && !chosen.in_archive {
        menu.push(item("Rename…", RenameFile));
    }
    menu.push(item(format!("Compress {what}…"), Run(Command::Pack)));
    if chosen.local {
        menu.extend([
            with_symbol(item("Duplicate", Duplicate), "plus.square.on.square"),
            item("Make Alias", MakeAlias),
            item(format!("Quick Look {what}"), QuickLook),
            item("Reveal in Finder", RevealInFinder),
        ]);
        if !paths.is_empty() {
            menu.push(Entry::Share(paths));
        }
    }
    menu.extend([
        Separator,
        with_symbol(item(format!("Copy {what}"), Run(Command::CopyToClipboard)), "doc.on.doc"),
        item("Copy Path to Clipboard", Run(Command::CopyNamesWithPathToClipboard)),
    ]);
    menu
}

/// The applications macOS opens `file` with: its default first.
fn open_with(file: &Path) -> Vec<Entry> {
    let (default, mut others) = applications_for(file);
    // One per app: updaters keep old copies; the one in /Applications first.
    others.sort_by_key(|app| !app.starts_with("/Applications"));
    let mut seen: Vec<String> = default.iter().filter_map(|app| bundle_id(app)).collect();
    others.retain(|app| match bundle_id(app) {
        Some(id) if seen.contains(&id) => false,
        Some(id) => {
            seen.push(id);
            true
        }
        None => Some(app) != default.as_ref(),
    });
    others.sort_by_key(|app| app_name(app).to_lowercase());
    let entry = |title: String, app: PathBuf| Entry::Item {
        title,
        icon: Some(Icon::File(app.clone())),
        action: Box::new(OpenWith(app)),
    };
    let mut menu = Vec::new();
    if let Some(app) = default {
        menu.push(entry(format!("{} (default)", app_name(&app)), app));
        if !others.is_empty() {
            menu.push(Entry::Separator);
        }
    }
    menu.extend(others.into_iter().map(|app| entry(app_name(&app), app)));
    menu
}

fn app_name(app: &Path) -> String {
    app.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

fn bundle_id(app: &Path) -> Option<String> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&app.to_string_lossy()))?;
    Some(bundle.bundleIdentifier()?.to_string())
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

/// AppKit's side: the menu, its pictures, sharing.
mod native {
    use std::cell::Cell;
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use gpui_kit::Action;
    use objc2_app_kit::{NSEvent, NSImage, NSMenu, NSMenuItem, NSSharingServicePicker, NSWorkspace};
    use objc2_foundation::{NSArray, NSSize, NSString, NSURL};

    use super::{Entry, Icon};

    thread_local! {
        static PICKED: Cell<Option<isize>> = const { Cell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCFileMenuTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(pick:))]
            fn pick(&self, item: &NSMenuItem) {
                PICKED.set(Some(item.tag()));
            }
        }
    );

    /// The menu at the mouse until closed; what was picked.
    pub(super) fn pop_up(entries: &[Entry]) -> Option<Box<dyn Action>> {
        let mtm = MainThreadMarker::new()?;
        let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
        let mut picks = Vec::new();
        let menu = menu(entries, &target, &mut picks, mtm);
        PICKED.set(None);
        menu.popUpMenuPositioningItem_atLocation_inView(None, NSEvent::mouseLocation(), None);
        let picked = PICKED.take()?;
        picks.into_iter().nth(picked as usize)
    }

    fn menu(entries: &[Entry], target: &Target, picks: &mut Vec<Box<dyn Action>>, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        for entry in entries {
            let item = match entry {
                Entry::Separator => NSMenuItem::separatorItem(mtm),
                Entry::Submenu { title, entries } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(title));
                    item.setSubmenu(Some(&self::menu(entries, target, picks, mtm)));
                    item
                }
                Entry::Share(paths) => {
                    let picker = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &urls(paths)) };
                    picker.standardShareMenuItem(mtm)
                }
                Entry::Item { title, icon, action } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(title));
                    unsafe {
                        item.setTarget(Some(target as &AnyObject));
                        item.setAction(Some(sel!(pick:)));
                    }
                    item.setTag(picks.len() as isize);
                    picks.push(action.boxed_clone());
                    if let Some(image) = icon.as_ref().and_then(image) {
                        item.setImage(Some(&image));
                    }
                    item
                }
            };
            menu.addItem(&item);
        }
        menu
    }

    fn image(icon: &Icon) -> Option<Retained<NSImage>> {
        let image = match icon {
            Icon::Symbol(name) => NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?,
            Icon::File(path) => {
                let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
                image.setSize(NSSize::new(16., 16.));
                image
            }
        };
        Some(image)
    }

    fn urls(paths: &[PathBuf]) -> Retained<NSArray> {
        let urls: Vec<Retained<AnyObject>> =
            paths.iter().map(|path| NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())).into()).collect();
        NSArray::from_retained_slice(&urls)
    }
}
