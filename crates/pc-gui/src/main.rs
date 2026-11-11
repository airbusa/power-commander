// No console window beside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appearance;
mod associations_dialog;
mod attributes_dialog;
mod compare_view;
mod connections_dialog;
mod drag_drop;
mod editor;
mod file_tools;
mod find_dialog;
mod keys_dialog;
mod mark_menu;
mod media_view;
mod menus;
mod minimap;
mod name_dialog;
mod occupied;
mod pack_dialog;
mod pdf_view;
mod properties_dialog;
mod quick_view;
mod remote;
mod rename_dialog;
mod settings_dialog;
mod sidebar;
mod transfer;
mod viewer;
mod views;

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::command::{self, Outcome};
use pc_core::menu::{BUTTON_BAR, Command, Item, TAB_MENU};
use pc_core::remote::Address;
use pc_core::session::Session;
use pc_core::settings::{self, Settings};
use pc_core::sidebar::SidebarTab;
use pc_core::theme::{self, Appearance};
use pc_core::{Panel, Sort, SortKey, Tabs, Typed, View, archive, edit, icons, mounts, ops, origin};

use crate::appearance::PanelColors;
use crate::drag_drop::{DragLabel, Dragged};
use crate::menus::{ReopenTab, Run, SelectTab, SetAppearance};

actions!(
    pc,
    [
        SwitchPanel,
        Up,
        Down,
        PageUp,
        PageDown,
        Home,
        End,
        Open,
        Parent,
        FocusPanels,
        CancelPathEdit,
        Left,
        Right
    ]
);

include!(concat!(env!("OUT_DIR"), "/material_icons.rs"));

// Lucide icons beyond GPUI Kit's own set.
gpui_kit::assets::icon_assets!(ExtraIcons, [Server, Smartphone, Tablet, Laptop, Monitor, Tv, Usb]);

/// GPUI Kit's icons, the file icons as `material/<name>.svg`, the player's as `media/`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(name) = path.strip_prefix("media/") {
            let found = media_view::ICONS.iter().find(|(icon, _)| *icon == name);
            return Ok(found.map(|(_, svg)| Cow::Borrowed(*svg)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        let Some(name) = path.strip_prefix("material/") else {
            return gpui_kit::assets::Assets.load(path);
        };
        let found = MATERIAL_ICONS.binary_search_by_key(&name, |(name, _)| name);
        Ok(found.ok().map(|ix| Cow::Borrowed(MATERIAL_ICONS[ix].1)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

const CONTEXT: &str = "Commander";
const ROW: Pixels = px(22.);
/// The path title's height, typing a path or not.
const PATH_HEIGHT: Pixels = px(30.);
/// Name, Ext, Size, Date and Attr; Name 0 takes the rest, until resized.
const COLUMN_WIDTHS: [f32; 5] = [0., 56., 100., 132., 92.];
const COLUMN_MIN: f32 = 24.;
/// Row and header padding plus border, left of Name and right of Attr.
const ROW_INSET: Pixels = px(5.);
/// The rows' scrollbar, right of them and of their titles.
const SCROLLBAR: Pixels = px(16.);
/// Within this share of the middle, the splitter snaps to it.
const SPLIT_MAGNET: f32 = 0.02;

/// Dragging the left edge of a column (Ext 0 … Attr 3) in a panel.
struct ColumnDrag {
    panel: usize,
    column: usize,
}

struct SplitDrag;

fn main() {
    let app = gpui_kit::application().with_assets(AppAssets);
    app.run(|cx| {
        gpui_kit::init(cx);
        let keys = Some(CONTEXT);
        cx.bind_keys([
            KeyBinding::new("tab", SwitchPanel, keys),
            KeyBinding::new("up", Up, keys),
            KeyBinding::new("down", Down, keys),
            KeyBinding::new("left", Left, keys),
            KeyBinding::new("right", Right, keys),
            KeyBinding::new("pageup", PageUp, keys),
            KeyBinding::new("pagedown", PageDown, keys),
            KeyBinding::new("home", Home, keys),
            KeyBinding::new("end", End, keys),
            KeyBinding::new("enter", Open, keys),
            KeyBinding::new("backspace", Parent, keys),
            KeyBinding::new("ctrl-pageup", Parent, keys),
            // Bound after the input's own keys, so it comes first.
            KeyBinding::new("escape", FocusPanels, Some("CommandLine > Input")),
            KeyBinding::new("escape", CancelPathEdit, Some("PathEdit > Input")),
        ]);
        menus::init(cx);
        // Files taken out of archives for the viewer.
        std::thread::spawn(archive::remove_stale_scratch);
        cx.on_app_quit(|_| {
            archive::remove_scratch();
            async {}
        })
        .detach();
        viewer::bind_keys(cx);
        editor::bind_keys(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1200.), px(800.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Power Commander".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(600.), px(400.))),
            app_id: Some("powercommander".into()),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Commander::new(window, cx))
        })
        .expect("open the main window");
    });
}

/// Where Settings opens from other windows.
#[derive(Clone)]
struct MainWindow(AnyWindowHandle, WeakEntity<Commander>);

impl Global for MainWindow {}

/// Settings on its Edit/View page, over the main window.
fn configure_viewer(cx: &mut App) {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    let page = settings::PAGES
        .iter()
        .position(|&(_, name)| name == "Edit/View");
    let _ = main.update(cx, |_, window, cx| {
        window.activate_window();
        let Ok(settings) = commander.read_with(cx, |this, _| this.settings.clone()) else {
            return;
        };
        settings_dialog::open(commander, settings, page.unwrap_or(0), window, cx);
    });
}

/// The setting, for windows that render while the main window updates.
struct ShowMinimap(bool);

impl Global for ShowMinimap {}

/// Whether the Viewer and the Editor show their minimap.
fn minimap_shown(cx: &App) -> bool {
    cx.try_global::<ShowMinimap>().is_none_or(|shown| shown.0)
}

/// Shows or hides the minimap, and saves the setting.
fn toggle_minimap(cx: &mut App) {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    let _ = main.update(cx, |_, window, cx| {
        let _ = commander.update(cx, |this, cx| {
            let mut settings = this.settings.clone();
            settings.viewer.minimap = !settings.viewer.minimap;
            this.apply_settings(settings, window, cx);
        });
    });
}

struct Commander {
    panels: [Side; 2],
    active: usize,
    focus: FocusHandle,
    menu_bar: Entity<AppMenuBar>,
    settings: Settings,
    colors: PanelColors,
    /// Show list of all tabs: the side and its popup.
    tab_list: Option<(usize, Entity<PopupMenu>, Subscription)>,
    /// A side whose path is being typed.
    path_edit: Option<(usize, Entity<InputState>, Subscription)>,
    _appearance: Subscription,
    command: Entity<InputState>,
    /// Widths of Ext, Size, Date and Attr, both panels'.
    columns: [f32; 5],
    /// The left panel's share of the width.
    split: f32,
    _quit: Subscription,
    _command: Subscription,
    transfers: transfer::Transfers,
    /// Save Selection's names, for Restore Selection.
    saved_selection: Vec<std::ffi::OsString>,
    /// Ctrl+C's or Ctrl+X's files.
    file_clip: Option<drag_drop::FileClip>,
    sidebar: sidebar::Sidebar,
    quick: Option<quick_view::QuickView>,
    thumbnails: views::Thumbnails,
    /// The panels one above the other.
    vertical: bool,
    /// Quick View's viewer has the keys: no panel shows a cursor.
    quick_keys: bool,
}

struct Side {
    tabs: Tabs,
    scroll: UniformListScrollHandle,
    /// Brief's columns scroll sideways.
    brief: ScrollHandle,
    /// The entries' area at the last frame: Brief's and Thumbnails' layout.
    area: Rc<Cell<Size<Pixels>>>,
}

impl Commander {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.set_global(MainWindow(window.window_handle(), cx.entity().downgrade()));
        let mut problems = Vec::new();
        let settings = Settings::load().unwrap_or_else(|err| {
            problems.push(format!("Could not read the settings: {err}"));
            Settings::default()
        });
        cx.set_global(ShowMinimap(settings.viewer.minimap));
        let session = Session::load().unwrap_or_else(|err| {
            problems.push(format!("Could not read the open tabs: {err}"));
            Session::default()
        });
        if !problems.is_empty() {
            // Notifications need the window's root first.
            window.defer(cx, move |window, cx| {
                let text = problems.join("\n");
                window.push_notification(Notification::error(text).autohide(false), cx)
            });
        }
        let colors = appearance::apply(&palette(&settings, window), cx);
        menus::bind(&settings.keys, None, cx);
        let sidebar = sidebar::Sidebar::new(session.sidebar, session.sidebar_width, settings.show_hidden);
        menus::set(settings.appearance, &|command| sidebar.shows(command), cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.appearance == Appearance::System {
                this.colors = appearance::apply(&palette(&this.settings, window), cx);
            }
        });
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let filter = settings.filter();
        let fallback = || {
            Panel::new(&home, filter)
                .or_else(|_| Panel::new("/", filter))
                .expect("list the home folder or /")
        };
        let side = |ix: usize| Side {
            tabs: match session.sides.get(ix) {
                Some(state) => Tabs::restore(state, filter, fallback),
                None => Tabs::new(fallback()),
            },
            scroll: UniformListScrollHandle::new(),
            brief: ScrollHandle::new(),
            area: Rc::default(),
        };
        // Closing the window drops this view before the app quits, so both save.
        let quit = cx.on_app_quit(|this, _| {
            this.save_session();
            this.transfers.queue.stop_all();
            async {}
        });
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = this.read_with(cx, |this, _| {
                this.save_session();
                this.transfers.queue.stop_all();
            });
            true
        });
        Self::watch_queue(window, cx);
        Self::watch_devices(cx);
        // Back from another program, which may have changed the folders.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reread_local(cx);
            }
        })
        .detach();
        let command = cx.new(|cx| InputState::new(window, cx));
        let command_entered = cx.subscribe_in(&command, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.run_command(window, cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // The window's root, which paints its background, comes after this.
        cx.defer_in(window, |this, window, cx| this.set_backdrop(window, cx));
        Self {
            panels: [side(0), side(1)],
            active: session.active.min(1),
            columns: match session.columns[..] {
                [name, ext, size, date, attr] => [name, ext, size, date, attr],
                [ext, size, date, attr] => [0., ext, size, date, attr],
                _ => COLUMN_WIDTHS,
            },
            split: match session.split {
                split if (0.15..=0.85).contains(&split) => split,
                _ => 0.5,
            },
            focus,
            menu_bar: AppMenuBar::new(cx),
            settings,
            colors,
            tab_list: None,
            path_edit: None,
            _appearance: appearance,
            command,
            _quit: quit,
            _command: command_entered,
            transfers: transfer::Transfers::default(),
            saved_selection: Vec::new(),
            file_clip: None,
            sidebar,
            quick: None,
            quick_keys: false,
            thumbnails: views::Thumbnails::default(),
            vertical: session.vertical,
        }
    }

    fn apply_settings(&mut self, settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        if settings.filter() != self.settings.filter() {
            for side in &mut self.panels {
                side.tabs.set_filter(settings.filter());
                side.scroll_to_cursor();
            }
            self.sidebar.refresh(settings.show_hidden);
        }
        if settings.viewer.minimap != self.settings.viewer.minimap {
            cx.set_global(ShowMinimap(settings.viewer.minimap));
            cx.refresh_windows();
        }
        if settings.keys != self.settings.keys {
            menus::bind(&settings.keys, Some(&self.settings.keys), cx);
            self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
        }
        self.settings = settings;
        self.save_settings(window, cx);
        cx.notify();
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.settings.save() {
            let text = format!("Could not save the settings: {err}");
            window.push_notification(Notification::error(text), cx);
        }
    }

    fn set_appearance(&mut self, chosen: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.appearance = chosen;
        self.save_settings(window, cx);
        self.colors = appearance::apply(&palette(&self.settings, window), cx);
        self.reload_menus(cx);
        cx.notify();
    }

    fn reload_menus(&self, cx: &mut Context<Self>) {
        let quick = self.quick.is_some();
        let checked = |command| self.sidebar.shows(command) || (command == Command::QuickViewPanel && quick);
        menus::set(self.settings.appearance, &checked, cx);
        self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
    }

    fn move_cursor(&mut self, rows: isize, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        side.tabs.current_mut().move_cursor(rows);
        side.scroll_to_cursor();
        cx.notify();
    }

    // shortcut: a failed save on quit goes unreported; there is no window left to say it.
    fn save_session(&self) {
        let _ = self.session().save();
    }

    /// F3 views, F4 edits the file under the cursor.
    /// F3, F4 and Ctrl+Shift+F4. A server's or an archive's file is copied
    /// out in the background; what the editors save goes back.
    fn open_file(&mut self, how: remote::Open, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(origin) = panel.origin() else {
            return;
        };
        let around = (how == remote::Open::View && origin.is_local()).then(|| panel.files_around());
        let (settings, bounds, name) =
            (self.settings.clone(), window.window_bounds(), origin.name());
        // The file's own viewer or editor, by its association.
        let entry = panel
            .current_entry()
            .map(|e| e.display_name())
            .unwrap_or_default();
        let own = pc_core::associations::find(&settings.associations, &entry).map(|a| match how {
            remote::Open::View => a.view.clone(),
            _ => a.edit.clone(),
        });
        let program = match (how, own.filter(|line| !line.trim().is_empty())) {
            (remote::Open::External, _) => Some(settings.editor.external.clone()),
            (_, own) => own,
        };
        let finish = move |prepared: io::Result<(PathBuf, Option<origin::Upload>)>,
                           window: &mut Window,
                           cx: &mut App| {
            let opened = prepared.and_then(|(file, upload)| match (how, upload) {
                (remote::Open::View, _) if let Some(program) = &program => {
                    edit::edit_externally(program, &file)
                }
                (_, Some(upload)) if let Some(program) = &program => {
                    remote::edit_externally(file, upload, program, window, cx)
                }
                (_, None) if let Some(program) = &program => edit::edit_externally(program, &file),
                (remote::Open::View, _) => {
                    let (files, at) = around.unwrap_or_else(|| (vec![file], 0));
                    viewer::open(files, at, settings.viewer, bounds, cx)
                }
                (remote::Open::Edit | remote::Open::External, upload) => {
                    editor::open(file, settings.editor.limit_mb, upload, bounds, cx)
                }
            });
            if let Err(err) = opened {
                window.push_notification(Notification::error(format!("{name}: {err}")), cx);
            }
        };
        // A server's file comes by a task the queue shows, which can be cancelled.
        if let origin::Origin::Server(fs, path) = origin {
            let upload =
                (how != remote::Open::View).then(|| origin::upload_to(fs.clone(), path.clone()));
            let file = path.file_name().unwrap_or_default().display().to_string();
            let title = match how {
                remote::Open::View => format!("Download {file} to view"),
                _ => format!("Download {file} to edit"),
            };
            let (sent, received) = futures::channel::oneshot::channel();
            let task = origin::fetch_task(fs, path, title, move |fetched| {
                let _ = sent.send(fetched);
            });
            self.transfers.queue.run_now(task);
            cx.spawn_in(window, async move |_, cx| {
                // The queue shows a failure; a cancel opens nothing.
                if let Ok(Ok(file)) = received.await {
                    let _ = cx.update(|window, cx| finish(Ok((file, upload)), window, cx));
                }
            })
            .detach();
            return cx.notify();
        }
        cx.spawn_in(window, async move |_, cx| {
            let prepare = async move {
                match how {
                    remote::Open::View => origin.view().map(|file| (file, None)),
                    _ => origin.edit(),
                }
            };
            let prepared = cx.background_executor().spawn(prepare).await;
            let _ = cx.update(|window, cx| finish(prepared, window, cx));
        })
        .detach();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        if self.panels[side].tabs.current().in_archive() {
            let text = "New folders cannot be made inside archives yet.";
            window.push_notification(Notification::warning(text), cx);
            return;
        }
        let commander = cx.entity().downgrade();
        let label = "New folder (directory):";
        name_dialog::open(
            "New folder",
            label,
            String::new(),
            window,
            cx,
            move |name, window, cx| {
                if name.trim().is_empty() {
                    return;
                }
                let _ = commander.update(cx, |this, cx| {
                    let panel = this.panels[side].tabs.current_mut();
                    if panel.remote().is_some() {
                        let made = move |panel: &mut Panel| panel.new_folder(&name);
                        return this.navigate_on(side, true, made, window, cx);
                    }
                    let folder = panel.folder().to_owned();
                    if let Err(err) = panel.new_folder(&name) {
                        let text = format!("Could not make the folder: {err}");
                        window.push_notification(Notification::error(text), cx);
                    }
                    this.refresh_folder(&folder, cx);
                });
            },
        );
    }

    fn mark(&mut self, change: impl FnOnce(&mut Panel), cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        change(side.tabs.current_mut());
        side.scroll_to_cursor();
        cx.notify();
    }

    fn confirm_delete(&mut self, to_trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        let chosen = panel.chosen();
        if chosen.is_empty() {
            return;
        }
        if panel.in_archive() {
            let text = "Deleting inside archives is not possible yet.";
            window.push_notification(Notification::warning(text), cx);
            return;
        }
        let what = match chosen.as_slice() {
            [one] if one.is_dir => format!("the folder \"{}\"", one.display_name()),
            [one] => format!("the file \"{}\"", one.display_name()),
            many => format!("{} items", many.len()),
        };
        let server = panel.remote().map(|remote| remote.url());
        let (title, text) = match to_trash {
            // A server has no trash.
            _ if server.is_some() => (
                "Delete",
                format!(
                    "Delete {what} on {}? This cannot be undone.",
                    server.unwrap_or_default()
                ),
            ),
            true => ("Delete", format!("Move {what} to the trash?")),
            false => (
                "Delete permanently",
                format!("Delete {what} permanently? This cannot be undone."),
            ),
        };
        let commander = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let commander = commander.clone();
            alert
                .title(title)
                .ok_variant(ButtonVariant::Danger)
                .description(text.clone())
                .ok_text(title)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let _ =
                        commander.update(cx, |this, cx| this.delete(side, to_trash, window, cx));
                    true
                })
        });
    }

    fn delete(&mut self, side: usize, to_trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[side].tabs.current_mut();
        if panel.remote().is_some() {
            return self.navigate_on(side, true, move |panel| panel.delete(false), window, cx);
        }
        let folder = panel.folder().to_owned();
        if let Err(err) = panel.delete(to_trash) {
            window.push_notification(Notification::error(format!("Could not delete: {err}")), cx);
        }
        self.refresh_folder(&folder, cx);
    }

    /// Every tab on `folder`, on both sides, reads it again.
    fn refresh_folder(&mut self, folder: &Path, cx: &mut Context<Self>) {
        for side in &mut self.panels {
            side.tabs.reload_folder(folder);
            side.scroll_to_cursor();
        }
        self.sidebar.refresh(self.settings.show_hidden);
        cx.notify();
    }

    fn set_sort(&mut self, ix: usize, sort: Sort, cx: &mut Context<Self>) {
        let side = &mut self.panels[ix];
        side.tabs.current_mut().set_sort(sort);
        side.scroll_to_cursor();
        cx.notify();
    }

    /// The sorted column turns around, another sorts by its default.
    fn sort_by_column(&mut self, ix: usize, key: SortKey, cx: &mut Context<Self>) {
        self.active = ix;
        let sort = self.panels[ix].tabs.current().sort();
        let sort = match sort.key == key {
            true => Sort {
                descending: !sort.descending,
                ..sort
            },
            false => Sort::by(key),
        };
        self.set_sort(ix, sort, cx);
    }

    fn edit_path(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        let path = self.panels[ix].tabs.current().location();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(path));
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.apply_path_edit(window, cx),
            // Clicking elsewhere drops it, focus going where clicked.
            InputEvent::Blur => {
                this.path_edit = None;
                cx.notify();
            }
            _ => {}
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.path_edit = Some((ix, input, events));
        cx.notify();
    }

    /// Goes to a folder that exists; otherwise says why and stays open.
    fn apply_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ix, typed) = match &self.path_edit {
            Some((ix, input, _)) => (*ix, input.read(cx).value().trim().to_owned()),
            None => return,
        };
        if typed.is_empty() {
            return self.cancel_path_edit(window, cx);
        }
        if let Some(address) = Address::parse(&typed) {
            self.cancel_path_edit(window, cx);
            return self.connect(ix, address, window, cx);
        }
        if mounts::is_url(&typed) {
            self.cancel_path_edit(window, cx);
            return self.mount_url(ix, typed, window, cx);
        }
        let panel = self.panels[ix].tabs.current();
        let target = match panel.typed_location(&typed) {
            // The server's, in the background.
            Typed::Here(target) if panel.remote().is_some() => {
                self.cancel_path_edit(window, cx);
                let go = move |panel: &mut Panel| panel.go_to_folder(target);
                return self.navigate_on(ix, true, go, window, cx);
            }
            Typed::Here(target) => target,
            // A server's path left without its address: this computer's.
            Typed::Local(target) if target.is_dir() => {
                self.cancel_path_edit(window, cx);
                let go = move |panel: &mut Panel| panel.disconnect(target);
                return self.navigate_on(ix, false, go, window, cx);
            }
            Typed::Local(target) => {
                let text = format!("{}: no such folder on this computer", target.display());
                return window.push_notification(Notification::error(text), cx);
            }
        };
        let side = &mut self.panels[ix];
        match side
            .tabs
            .navigate(|panel| panel.go_to_folder(target.clone()))
        {
            Ok(()) => {
                side.scroll_to_cursor();
                self.cancel_path_edit(window, cx);
            }
            Err(err) => {
                let text = format!("{}: {err}", target.display());
                window.push_notification(Notification::error(text), cx);
            }
        }
        cx.notify();
    }

    fn cancel_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path_edit.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn run_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.command.read(cx).value().to_string();
        let target = line.trim().strip_prefix("cd ").unwrap_or(line.trim());
        if let Some(address) = Address::parse(target) {
            self.focus_panels(window, cx);
            return self.connect(self.active, address, window, cx);
        }
        if mounts::is_url(target) {
            let url = target.to_owned();
            self.focus_panels(window, cx);
            return self.mount_url(self.active, url, window, cx);
        }
        let panel = self.panels[self.active].tabs.current();
        if let Some(remote) = panel.remote().filter(|_| !line.trim().is_empty()) {
            // shortcut: commands do not run on servers; cd moves there.
            match line.trim().strip_prefix("cd ").map(str::trim) {
                Some(path) => {
                    let path = match path.strip_prefix('~') {
                        Some(rest) => remote.home().join(rest.trim_start_matches('/')),
                        None => command::resolve(path, panel.folder()),
                    };
                    self.navigate(move |panel| panel.go_to_folder(path), window, cx);
                }
                None => {
                    let text = "Commands run on this computer; the panel is on a server.";
                    window.push_notification(Notification::warning(text), cx);
                }
            }
            return self.focus_panels(window, cx);
        }
        if !line.trim().is_empty() {
            let folder = self.panels[self.active].tabs.current().folder().to_owned();
            match command::run(&line, &folder) {
                Ok(Outcome::ChangeDir(path)) => {
                    self.navigate(move |panel| panel.go_to_folder(path), window, cx)
                }
                Ok(Outcome::Started) => {}
                Err(err) => {
                    window.push_notification(Notification::error(format!("{line}: {err}")), cx)
                }
            }
        }
        self.focus_panels(window, cx);
    }

    fn focus_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus, cx);
    }

    fn render_command_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let folder = self.panels[self.active].tabs.current().folder();
        h_flex()
            .key_context("CommandLine")
            .on_action(
                cx.listener(|this, _: &FocusPanels, window, cx| this.focus_panels(window, cx)),
            )
            .px_1()
            .gap_1()
            .child(
                div()
                    .max_w(relative(0.5))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    .child(format!("{}>", folder.display())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    .child(Input::new(&self.command).small()),
            )
    }

    fn render_button_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = BUTTON_BAR.iter().filter_map(|item| match *item {
            Item::Command { label, command } => {
                let [key, _] = self.settings.keys.keys_of(command);
                Some((label, key, command))
            }
            _ => None,
        });
        h_flex()
            .bg(cx.theme().list_head)
            .border_t_1()
            .border_color(self.colors.header_border)
            .children(buttons.map(|(label, keys, command)| {
                let key = keys_dialog::shown(&keys);
                Button::new(label)
                    .ghost()
                    .small()
                    .flex_1()
                    .label(format!("{key} {label}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        window.dispatch_action(Box::new(Run(command)), cx);
                    }))
            }))
    }

    fn session(&self) -> Session {
        Session {
            sides: self.panels.iter().map(|side| side.tabs.state()).collect(),
            active: self.active,
            columns: self.columns.to_vec(),
            split: self.split,
            sidebar: self.sidebar.tab,
            sidebar_width: self.sidebar.width,
            vertical: self.vertical,
        }
    }

    fn rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (side, tab) = (self.active, self.panels[self.active].tabs.active());
        let name = self.panels[side].tabs.current().name();
        let commander = cx.entity().downgrade();
        name_dialog::open(
            "Rename tab",
            "New name:",
            name,
            window,
            cx,
            move |name, _, cx| {
                let _ = commander.update(cx, |this, cx| {
                    this.panels[side].tabs.rename(tab, &name);
                    cx.notify();
                });
            },
        );
    }

    fn copy_tab_to_other_panel(&mut self, cx: &mut Context<Self>) {
        let copy = self.panels[self.active].tabs.current().clone();
        let other = &mut self.panels[1 - self.active];
        other.tabs.add(copy);
        other.scroll_to_cursor();
        cx.notify();
    }

    fn reopen_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        if let Err(err) = side.tabs.reopen(ix) {
            window.push_notification(
                Notification::error(format!("Could not reopen the tab: {err}")),
                cx,
            );
        }
        side.scroll_to_cursor();
        cx.notify();
    }

    fn show_tab_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = &self.panels[self.active].tabs;
        let titles: Vec<(String, bool)> = tabs
            .panels()
            .iter()
            .enumerate()
            .map(|(ix, panel)| {
                (
                    format!("{}: {}", ix + 1, panel.title()),
                    ix == tabs.active(),
                )
            })
            .collect();
        let focus = self.focus.clone();
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for (ix, (title, current)) in titles.into_iter().enumerate() {
                menu = menu.menu_with_check(title, current, Box::new(SelectTab(ix)));
            }
            menu.action_context(focus)
        });
        let dismissed = cx.subscribe_in(&list, window, |this, _, _: &DismissEvent, window, cx| {
            this.tab_list = None;
            window.focus(&this.focus, cx);
            cx.notify();
        });
        list.read(cx).focus_handle(cx).focus(window, cx);
        self.tab_list = Some((self.active, list, dismissed));
        cx.notify();
    }

    fn change_tabs(&mut self, change: impl FnOnce(&mut Tabs), cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        change(&mut side.tabs);
        side.scroll_to_cursor();
        cx.notify();
    }

    fn select_tab(&mut self, ix: usize, tab: usize, cx: &mut Context<Self>) {
        self.active = ix;
        let side = &mut self.panels[ix];
        side.tabs.select(tab);
        side.scroll_to_cursor();
        cx.notify();
    }

    fn page(&self) -> isize {
        self.panels[self.active].page_step()
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        side.tabs.current_mut().set_view(view);
        side.scroll_to_cursor();
        cx.notify();
    }

    /// On a server, in the background.
    /// Enter: into folders and archives; files open with their program.
    fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let file = panel
            .current_entry()
            .is_some_and(|e| !e.is_dir && (panel.in_archive() || !archive::is_archive(&e.name)));
        match file {
            true => self.launch(window, cx),
            false => self.navigate(|panel| panel.open().map(drop), window, cx),
        }
    }

    /// A server's or an archive's file opens from a copy, and is never run.
    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let (Some(origin), Some(entry)) = (panel.origin(), panel.current_entry()) else {
            return;
        };
        let executable = origin.is_local() && entry.attributes.contains('x');
        let associations = self.settings.associations.clone();
        let name = origin.name();
        cx.spawn_in(window, async move |_, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let file = origin.view()?;
                    pc_core::associations::open(&associations, &file, executable)
                })
                .await;
            if let Err(err) = opened {
                let _ = cx.update(|window, cx| {
                    window.push_notification(Notification::error(format!("{name}: {err}")), cx)
                });
            }
        })
        .detach();
    }

    fn navigate(
        &mut self,
        go: impl FnOnce(&mut Panel) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        self.navigate_on(self.active, remote, go, window, cx);
    }

    /// `background` for a server: the move runs on a copy of the panel,
    /// which then takes the tab's place. Locally it is instant, and keys
    /// pressed meanwhile must not be lost.
    fn navigate_on(
        &mut self,
        ix: usize,
        background: bool,
        go: impl FnOnce(&mut Panel) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = &mut self.panels[ix];
        if !background {
            if let Err(err) = side.tabs.navigate(go) {
                let text = format!("{}: {err}", side.tabs.current().location());
                window.push_notification(Notification::error(text), cx);
            }
            side.scroll_to_cursor();
            return cx.notify();
        }
        let tab = side.tabs.active();
        let mut moved = side.tabs.current().clone();
        cx.spawn_in(window, async move |this, cx| {
            let went = cx
                .background_executor()
                .spawn(async move { go(&mut moved).map(|()| moved) });
            let went = went.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let side = &mut this.panels[ix];
                match went {
                    Ok(moved) => side.tabs.navigated(tab, moved),
                    Err(err) => {
                        let text = format!("{}: {err}", side.tabs.current().location());
                        window.push_notification(Notification::error(text), cx);
                    }
                }
                side.scroll_to_cursor();
                cx.notify();
            });
        })
        .detach();
    }

    /// A panel, or Quick View where it covers it.
    fn render_side(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.quick.as_ref().is_some_and(|quick| quick.side == ix) {
            true => self.render_quick_view(self.quick_keys, cx).expect("Quick View is on"),
            false => self.render_panel(ix, cx).into_any_element(),
        }
    }

    fn render_panel(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let side = &self.panels[ix];
        let panel = side.tabs.current();
        let active = ix == self.active && !self.quick_keys;
        let (title_bg, title_fg, stripe) = if active {
            (self.colors.active_path, theme.foreground, theme.primary)
        } else {
            (self.colors.inactive_path, theme.muted_foreground, self.colors.inactive_path)
        };
        let sort = panel.sort();
        let colors = self.colors;
        let accent = theme.primary;
        let title = |label: &'static str, key: SortKey, right: bool| {
            let sorted = sort.key == key;
            let arrow = match sort.descending {
                true => IconName::ArrowDown,
                false => IconName::ArrowUp,
            };
            h_flex()
                .id((label, ix))
                .h_full()
                .px_1()
                .gap_1()
                .cursor_pointer()
                .border_r_1()
                .border_color(colors.header_border)
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by_column(ix, key, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(right, |el| el.text_right())
                        .child(label),
                )
                .when(sorted, |el| el.child(Icon::new(arrow).small().text_color(accent)))
        };
        let [name, ext, size, date, attr] = self.columns.map(px);
        let filling = self.columns[0] == 0.;
        let (scrolled, wide) = {
            let scroll = &side.scroll.0.borrow().base_handle;
            (scroll.offset().x, scroll.max_offset().x > px(0.))
        };
        // A column's right edge: while Name fills, as far from the right as the
        // columns right of it; otherwise past the columns left of it.
        let handles = (0..5).map(|column| {
            let handle = div()
                .id(("resize", ix * 5 + column))
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(6.))
                .occlude()
                .cursor(CursorStyle::ResizeColumn)
                .on_drag(ColumnDrag { panel: ix, column }, |_, _, _, cx| {
                    cx.new(|_| EmptyView)
                });
            match filling {
                true => {
                    let right: f32 = self.columns[column + 1..].iter().sum();
                    handle.right(px(right - 3.))
                }
                false => {
                    let left: f32 = self.columns[..=column].iter().sum();
                    handle.left(scrolled + px(left - 3.))
                }
            }
        });
        let column_titles = h_flex()
            .h(ROW + px(4.))
            .pl_1()
            .pr(px(4.) + SCROLLBAR)
            .bg(theme.list_head)
            .border_x_1()
            .border_b_1()
            .border_color(colors.header_border)
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let drag = event.drag(cx);
                    if drag.panel != ix {
                        return;
                    }
                    let column = drag.column;
                    let scrolled = this.panels[ix].scroll.0.borrow().base_handle.offset().x;
                    let widths = &mut this.columns;
                    // Name stops filling, so the others keep their widths from now on.
                    if widths[0] == 0. {
                        let inside = event.bounds.size.width - ROW_INSET - ROW_INSET - SCROLLBAR;
                        widths[0] = (f32::from(inside) - widths[1..].iter().sum::<f32>()).max(COLUMN_MIN);
                    }
                    let left = event.bounds.left() + ROW_INSET + scrolled;
                    let left = left + px(widths[..column].iter().sum());
                    let width = f32::from(event.event.position.x - left);
                    widths[column] = width.clamp(COLUMN_MIN, 2000.);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .size_full()
                            .ml(scrolled)
                            .child(title("Name", SortKey::Name, false).map(|el| match filling {
                                true => el.flex_1().min_w_0(),
                                false => el.w(name).flex_none(),
                            }))
                            .child(title("Ext", SortKey::Extension, false).w(ext).flex_none())
                            .child(title("Size", SortKey::Size, true).w(size).flex_none())
                            .child(title("Date", SortKey::Modified, false).w(date).flex_none())
                            .child(title("Attr", SortKey::Attributes, false).w(attr).flex_none()),
                    )
                    .children(handles),
            );
        let editing = self
            .path_edit
            .as_ref()
            .filter(|(side, ..)| *side == ix)
            .map(|(_, input, _)| input.clone());
        let rows = uniform_list(
            ("panel", ix),
            panel.entries().len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| this.render_row(ix, row, cx)).collect()
            }),
        )
        .track_scroll(&side.scroll)
        .size_full()
        .pr(SCROLLBAR)
        // Columns wider than the panel scroll sideways, under a scrollbar of their own.
        .when(!filling, |list| {
            list.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        })
        .when(wide, |list| list.pb(SCROLLBAR));
        // The list paints its rows over its padding too; these keep it under the scrollbars clear.
        let gutter = || div().absolute().right_0().bottom_0().bg(theme.background);
        let rows = match panel.view() {
            View::Full => div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(views::measure(side.area.clone(), cx.entity_id()))
                .child(rows)
                .when(!filling, |el| el.child(gutter().top_0().w(SCROLLBAR)))
                .when(wide, |el| el.child(gutter().left_0().h(SCROLLBAR)))
                .child(div().absolute().inset_0().child(Scrollbar::new(&side.scroll)))
                .into_any_element(),
            View::Brief => self.render_brief(ix, cx),
            View::Thumbnails => self.render_thumbnails(ix, cx),
        };
        let theme = cx.theme();
        let tab_list = self.tab_list.as_ref().filter(|(side, ..)| *side == ix);
        let ring = theme.ring;
        v_flex()
            .size_full()
            .min_w_0()
            .border_1()
            .border_color(theme.border)
            // A click anywhere in it makes it the active one, as Total Commander's.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.active = ix;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .drag_over::<Dragged>(move |style, _, _, _| style.border_color(ring))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(ring))
            .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                this.drop_files(*dragged, ix, None, window, cx)
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_paths(paths, ix, None, window, cx)
            }))
            .when_some(tab_list, |el, (_, list, _)| {
                el.child(deferred(
                    anchored()
                        .snap_to_window_with_margin(px(8.))
                        .child(div().occlude().child(list.clone())),
                ))
            })
            .when(
                side.tabs.panels().len() > 1 || self.settings.tab_header_always,
                |el| {
                    let commander = cx.entity();
                    let focus = self.focus.clone();
                    let tabs = side.tabs.panels().iter().enumerate().map(|(tab, panel)| {
                        // Right-click acts on the clicked tab.
                        Tab::new().label(panel.title()).on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                this.select_tab(ix, tab, cx);
                                window.focus(&this.focus, cx);
                            }),
                        )
                    });
                    el.child(
                        div()
                            .child(
                                TabBar::new(("tabs", ix))
                                    .small()
                                    .selected_index(side.tabs.active())
                                    .children(tabs)
                                    .on_click(cx.listener(move |this, tab: &usize, window, cx| {
                                        this.select_tab(ix, *tab, cx);
                                        window.focus(&this.focus, cx);
                                    })),
                            )
                            .context_menu(move |menu, window, cx| {
                                let tabs = &commander.read(cx).panels[ix].tabs;
                                let locked = tabs.current().is_locked();
                                let closed = tabs
                                    .closed()
                                    .iter()
                                    .map(|panel| panel.path().display().to_string().into())
                                    .collect();
                                menus::context_menu(
                                    menu.action_context(focus.clone()),
                                    TAB_MENU,
                                    |c| c == Command::LockTab && locked,
                                    closed,
                                    window,
                                    cx,
                                )
                            }),
                    )
                },
            )
            .child(match editing {
                Some(input) => div()
                    .key_context("PathEdit")
                    .on_action(cx.listener(|this, _: &CancelPathEdit, window, cx| {
                        this.cancel_path_edit(window, cx)
                    }))
                    // Input::h sizes only multi-line boxes.
                    .child(Styled::h(Input::new(&input).small(), PATH_HEIGHT))
                    .into_any_element(),
                None => div()
                    .id(("path", ix))
                    .h(PATH_HEIGHT)
                    .flex()
                    .items_center()
                    .px_2()
                    .border_l(px(3.))
                    .border_color(stripe)
                    .bg(title_bg)
                    .text_color(title_fg)
                    .cursor_text()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.edit_path(ix, window, cx)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(panel.location()))
                    .when(panel.remote().is_some(), |el| {
                        el.child(
                            Button::new(("disconnect", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip("Disconnect")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.active = ix;
                                    this.disconnect(window, cx);
                                })),
                        )
                    })
                    .into_any_element(),
            })
            .child(column_titles)
            .child(rows)
            .child(
                div()
                    .px_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(match panel.view() {
                        // Names only: the line tells the rest, as Total Commander's.
                        View::Brief => panel.current_entry().map_or_else(String::new, |entry| {
                            let size = panel.size_text(entry);
                            format!("{}   {size}   {}   {}", entry.display_name(), entry.modified_text(), entry.attributes)
                        }),
                        View::Full | View::Thumbnails => panel.status_text(),
                    }),
            )
    }

    fn render_row(&self, ix: usize, row: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let light = !theme.mode.is_dark();
        let icon = icons::material().icon(&entry.display_name(), entry.is_dir, light);
        let (name, extension) = entry.name_and_extension();
        let [name_width, ext, size, date, attr] = self.columns.map(px);
        self.entry_base(ix, row, true, cx)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(ROW)
            .px_1()
            .child(
                h_flex()
                    .map(|el| match name_width > px(0.) {
                        true => el.w(name_width).flex_none(),
                        false => el.flex_1().min_w_0(),
                    })
                    .px_1()
                    .gap_1()
                    .child(img(format!("material/{icon}")).size(px(16.)).flex_none())
                    .child(div().truncate().child(name)),
            )
            .child(div().w(ext).flex_none().px_1().truncate().child(extension))
            .child(
                div()
                    .w(size)
                    .flex_none()
                    .px_1()
                    .text_right()
                    .truncate()
                    .child(panel.size_text(entry)),
            )
            .child(
                div()
                    .w(date)
                    .flex_none()
                    .px_1()
                    .truncate()
                    .child(entry.modified_text()),
            )
            .child(
                div()
                    .w(attr)
                    .flex_none()
                    .px_1()
                    .truncate()
                    .child(entry.attributes.clone()),
            )
    }

    /// An entry as every view has it: clicked, dragged, dropped onto; the
    /// cursor's frame, marked names' colour; `striped` every other one.
    fn entry_base(&self, ix: usize, row: usize, striped: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let cursor = row == panel.cursor() && ix == self.active && !self.quick_keys;
        let marked = panel.is_marked(entry);
        let (labels, payloads) = (cx.entity(), cx.entity());
        let target = cx.theme().accent.opacity(0.35);
        div()
            .id(("row", row))
            .on_drag(Dragged { side: ix, row }, move |dragged, grab, _, cx| {
                let text = labels.read(cx).drag_label(*dragged);
                cx.new(|_| DragLabel { text, grab })
            })
            .external_drag_payload(move |dragged: &Dragged, _, cx| {
                payloads.read(cx).drag_payload(*dragged)
            })
            // A folder takes what is dropped onto it.
            .when(entry.is_dir, |el| {
                el.drag_over::<Dragged>(move |style, _, _, _| style.bg(target))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                    .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                        this.drop_files(*dragged, ix, Some(row), window, cx)
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drop_paths(paths, ix, Some(row), window, cx)
                    }))
            })
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .when(striped && row % 2 == 1, |el| el.bg(self.colors.alternate_row))
            .when(cursor, |el| {
                el.bg(self.colors.cursor).border_dashed().border_color(self.colors.cursor_frame)
            })
            .when(marked, |el| el.text_color(self.colors.marked))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.active = ix;
                this.panels[ix].tabs.current_mut().set_cursor(row);
                cx.notify();
                if event.click_count() == 2 {
                    this.enter(window, cx);
                }
            }))
    }
}

impl Render for Commander {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_quick_view(window, cx);
        self.quick_keys = self.quick_focused(window, cx);
        self.trim_thumbnails(window);
        let vertical = self.vertical;
        // Keeps panel keys out of the menu bar.
        // Typing a path, the panels' keys rest.
        let panels = div()
            .flex()
            .map(|el| if vertical { el.flex_col() } else { el.flex_row() })
            .when(self.path_edit.is_none(), |el| el.key_context(CONTEXT))
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h_0()
            .p_1()
            .on_action(cx.listener(|this, _: &SwitchPanel, window, cx| {
                if !this.tab_quick_view(window, cx) {
                    this.active = 1 - this.active;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Up, _, cx| {
                this.move_cursor(-this.panels[this.active].vertical_step(), cx)
            }))
            .on_action(cx.listener(|this, _: &Down, _, cx| {
                this.move_cursor(this.panels[this.active].vertical_step(), cx)
            }))
            .on_action(cx.listener(|this, _: &Left, _, cx| {
                if let Some(step) = this.panels[this.active].sideways_step() {
                    this.move_cursor(-step, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Right, _, cx| {
                if let Some(step) = this.panels[this.active].sideways_step() {
                    this.move_cursor(step, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.move_cursor(-this.page(), cx)))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.move_cursor(this.page(), cx)))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.move_cursor(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.move_cursor(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.enter(window, cx)))
            .on_action(
                cx.listener(|this, _: &Parent, window, cx| this.navigate(Panel::up, window, cx)),
            )
            .on_action(
                cx.listener(|this, Run(command): &Run, window, cx| match command {
                    Command::DuplicateTab => this.change_tabs(Tabs::duplicate, cx),
                    Command::LockTab => this.change_tabs(Tabs::toggle_lock, cx),
                    Command::CloseTab => this.change_tabs(Tabs::close, cx),
                    Command::NextTab => this.change_tabs(|tabs| tabs.step(1), cx),
                    Command::PreviousTab => this.change_tabs(|tabs| tabs.step(-1), cx),
                    Command::CloseAllTabs => this.change_tabs(Tabs::close_all, cx),
                    Command::CloseDuplicateTabs => this.change_tabs(Tabs::close_duplicates, cx),
                    Command::RenameTab => this.rename_tab(window, cx),
                    Command::CopyTabToOtherPanel => this.copy_tab_to_other_panel(cx),
                    Command::ListTabs => this.show_tab_list(window, cx),
                    Command::NewFolder => this.new_folder(window, cx),
                    Command::CalculateOccupiedSpace => this.calculate_space(window, cx),
                    Command::ChangeAttributes => this.change_attributes(window, cx),
                    Command::Properties => this.show_properties(window, cx),
                    Command::Pack => this.pack(window, cx),
                    Command::UnpackSpecificFiles => this.unpack(window, cx),
                    Command::TestArchives => this.test_archives(window, cx),
                    Command::CreateChecksumFiles => this.create_checksums(window, cx),
                    Command::VerifyChecksums => this.verify_checksums(window, cx),
                    Command::SplitFile => this.split_file(window, cx),
                    Command::MultiRenameTool => this.multi_rename(window, cx),
                    Command::CompareByContent => this.compare_files(window, cx),
                    Command::AssociateWith => this.associate_with(window, cx),
                    Command::InternalAssociations => this.internal_associations(window, cx),
                    Command::CombineFiles => this.combine_files(window, cx),
                    Command::View => this.open_file(remote::Open::View, window, cx),
                    Command::Edit => this.open_file(remote::Open::Edit, window, cx),
                    Command::EditExternally => this.open_file(remote::Open::External, window, cx),
                    Command::Delete => this.confirm_delete(true, window, cx),
                    Command::DeletePermanently => this.confirm_delete(false, window, cx),
                    Command::ToggleMark => this.mark(Panel::toggle_mark, cx),
                    Command::Copy => this.transfer(ops::Kind::Copy, window, cx),
                    Command::CopyToClipboard => this.copy_files(false, window, cx),
                    Command::CutToClipboard => this.copy_files(true, window, cx),
                    Command::PasteFromClipboard => this.paste_files(window, cx),
                    Command::Move => this.transfer(ops::Kind::Move, window, cx),
                    Command::ConnectToServer => {
                        let side = this.active;
                        connections_dialog::open(cx.entity().downgrade(), side, window, cx)
                    }
                    Command::Disconnect => this.disconnect(window, cx),
                    Command::QuickViewPanel => this.toggle_quick_view(window, cx),
                    Command::Brief => this.set_view(View::Brief, cx),
                    Command::Full => this.set_view(View::Full, cx),
                    Command::ThumbnailView => this.set_view(View::Thumbnails, cx),
                    Command::VerticalArrangement => {
                        this.vertical = !this.vertical;
                        cx.notify()
                    }
                    Command::SidebarTree => this.toggle_sidebar(SidebarTab::Tree, window, cx),
                    Command::SidebarFavourites => {
                        this.toggle_sidebar(SidebarTab::Favourites, window, cx)
                    }
                    Command::OperationQueue => {
                        transfer::open_queue(this.transfers.queue.clone(), window, cx)
                    }
                    Command::SelectAll => this.mark(|panel| panel.mark_all(true), cx),
                    Command::UnselectAll => this.mark(|panel| panel.mark_all(false), cx),
                    Command::InvertSelection => this.mark(Panel::invert_marks, cx),
                    Command::SortByName => this.set_sort(this.active, Sort::by(SortKey::Name), cx),
                    Command::SortByExtension => {
                        this.set_sort(this.active, Sort::by(SortKey::Extension), cx)
                    }
                    Command::SortByTime => {
                        this.set_sort(this.active, Sort::by(SortKey::Modified), cx)
                    }
                    Command::SortBySize => this.set_sort(this.active, Sort::by(SortKey::Size), cx),
                    Command::Unsorted => {
                        this.set_sort(this.active, Sort::by(SortKey::Unsorted), cx)
                    }
                    Command::ReversedOrder => {
                        let sort = this.panels[this.active].tabs.current().sort();
                        let reversed = Sort {
                            descending: !sort.descending,
                            ..sort
                        };
                        this.set_sort(this.active, reversed, cx)
                    }
                    Command::Settings => {
                        let commander = cx.entity().downgrade();
                        settings_dialog::open(commander, this.settings.clone(), 0, window, cx)
                    }
                    Command::Shortcuts => {
                        let commander = cx.entity().downgrade();
                        keys_dialog::open(commander, this.settings.keys.clone(), window, cx)
                    }
                    other => {
                        if !this.mark_command(*other, window, cx) {
                            cx.propagate()
                        }
                    }
                }),
            )
            .on_action(cx.listener(|this, SelectTab(tab): &SelectTab, _, cx| {
                this.select_tab(this.active, *tab, cx)
            }))
            .on_action(cx.listener(|this, ReopenTab(ix): &ReopenTab, window, cx| {
                this.reopen_tab(*ix, window, cx)
            }))
            .on_action(cx.listener(|this, SetAppearance(chosen): &SetAppearance, window, cx| {
                this.set_appearance(*chosen, window, cx)
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SplitDrag>, _, cx| {
                    let (bounds, at) = (event.bounds, event.event.position);
                    let share = match this.vertical {
                        true => (at.y - bounds.top()) / bounds.size.height,
                        false => (at.x - bounds.left()) / bounds.size.width,
                    };
                    this.split = match (share - 0.5).abs() < SPLIT_MAGNET {
                        true => 0.5,
                        false => share.clamp(0.15, 0.85),
                    };
                    cx.notify();
                }),
            )
            .child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex_none()
                    .map(|el| match vertical {
                        true => el.w_full().h(relative(self.split)).pb(px(2.)),
                        false => el.h_full().w(relative(self.split)).pr(px(2.)),
                    })
                    .child(self.render_side(0, cx)),
            )
            .child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex_1()
                    .map(|el| match vertical {
                        true => el.w_full().pt(px(2.)),
                        false => el.h_full().pl(px(2.)),
                    })
                    .child(self.render_side(1, cx)),
            )
            .child(
                div()
                    .id("splitter")
                    .absolute()
                    .occlude()
                    .map(|el| match vertical {
                        true => el.left_0().right_0().top(relative(self.split)).mt(px(-3.)).h(px(6.)),
                        false => el.top_0().bottom_0().left(relative(self.split)).ml(px(-3.)).w(px(6.)),
                    })
                    .cursor(if vertical { CursorStyle::ResizeUpDown } else { CursorStyle::ResizeLeftRight })
                    .hover(|style| style.bg(cx.theme().list_active))
                    .on_drag(SplitDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
            );
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background)
            .child(panels)
            .children(self.render_transfers(cx))
            .child(self.render_command_line(cx))
            .child(self.render_button_bar(cx));
        // The sidebar may let the desktop through: everything else paints its own background.
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(
                div()
                    .bg(self.colors.bar)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.menu_bar.clone()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .on_drag_move(cx.listener(
                        |this, event: &DragMoveEvent<sidebar::SidebarDrag>, _, cx| {
                            this.drag_sidebar(event.bounds, event.event.position.x, cx)
                        },
                    ))
                    .children(self.render_sidebar(cx))
                    .child(main),
            )
    }
}

/// The viewer's and editor's windows: where the main one is. Wayland lets
/// no app place its windows; as the main window's child, the compositor
/// centres it over the main window.
pub(crate) fn over_main_window(title: String, bounds: WindowBounds, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(bounds),
        kind: match cx.compositor_name() {
            "Wayland" => WindowKind::Floating,
            _ => WindowKind::Normal,
        },
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    }
}

/// The system's look, light or dark as chosen.
fn palette(settings: &Settings, window: &Window) -> theme::Palette {
    let dark = match settings.appearance {
        Appearance::System => matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
        Appearance::Light => false,
        Appearance::Dark => true,
    };
    theme::system(dark)
}
