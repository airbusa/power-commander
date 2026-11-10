// No console window beside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appearance;
mod attributes_dialog;
mod connections_dialog;
mod editor;
mod find_dialog;
mod menus;
mod minimap;
mod name_dialog;
mod occupied;
mod pdf_view;
mod properties_dialog;
mod remote;
mod settings_dialog;
mod transfer;
mod viewer;

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::command::{self, Outcome};
use pc_core::menu::{BUTTON_BAR, Command, Item, TAB_MENU};
use pc_core::remote::Address;
use pc_core::session::Session;
use pc_core::settings::{self, Settings};
use pc_core::theme::{self, Theme};
use pc_core::{Panel, Sort, SortKey, Tabs, archive, edit, icons, ops};

use crate::appearance::PanelColors;
use crate::menus::{ReopenTab, Run, SelectTab, SetTheme};

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
        CancelPathEdit
    ]
);

include!(concat!(env!("OUT_DIR"), "/material_icons.rs"));

/// GPUI Kit's icons, and the file icons as `material/<name>.svg`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
/// Ext, Size, Date and Attr; Name takes the rest.
const COLUMN_WIDTHS: [f32; 4] = [56., 100., 132., 92.];
const COLUMN_MIN: f32 = 24.;
/// Row and header padding plus border, left of Name and right of Attr.
const ROW_INSET: Pixels = px(5.);
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
    themes: Vec<Theme>,
    settings: Settings,
    colors: PanelColors,
    /// Show list of all tabs: the side and its popup.
    tab_list: Option<(usize, Entity<PopupMenu>, Subscription)>,
    /// A side whose path is being typed.
    path_edit: Option<(usize, Entity<InputState>, Subscription)>,
    _appearance: Subscription,
    command: Entity<InputState>,
    /// Widths of Ext, Size, Date and Attr, both panels'.
    columns: [f32; 4],
    /// The left panel's share of the width.
    split: f32,
    _quit: Subscription,
    _command: Subscription,
    transfers: transfer::Transfers,
}

struct Side {
    tabs: Tabs,
    scroll: UniformListScrollHandle,
}

impl Side {
    fn scroll_to_cursor(&self) {
        let cursor = self.tabs.current().cursor();
        self.scroll.scroll_to_item(cursor, ScrollStrategy::Nearest);
    }
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
        let mut themes = theme::builtin();
        if let Some(dir) = settings::themes_dir() {
            let (own, unreadable) = theme::load_dir(&dir);
            problems.extend(unreadable);
            for own in own {
                match themes.iter().any(|t| t.name == own.name) {
                    true => problems.push(format!(
                        "{}: there is a theme named {} already",
                        dir.display(),
                        own.name
                    )),
                    false => themes.push(own),
                }
            }
        }
        if !problems.is_empty() {
            // Notifications need the window's root first.
            window.defer(cx, move |window, cx| {
                let text = problems.join("\n");
                window.push_notification(Notification::error(text).autohide(false), cx)
            });
        }
        let colors = appearance::apply(pick(&themes, &settings, window), cx);
        menus::set(&themes, settings.theme.as_deref(), cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.theme.is_none() {
                this.colors = appearance::apply(pick(&this.themes, &this.settings, window), cx);
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
        let command = cx.new(|cx| InputState::new(window, cx));
        let command_entered = cx.subscribe_in(&command, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.run_command(window, cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            panels: [side(0), side(1)],
            active: session.active.min(1),
            columns: <[f32; 4]>::try_from(session.columns.as_slice()).unwrap_or(COLUMN_WIDTHS),
            split: match session.split {
                split if (0.15..=0.85).contains(&split) => split,
                _ => 0.5,
            },
            focus,
            menu_bar: AppMenuBar::new(cx),
            themes,
            settings,
            colors,
            tab_list: None,
            path_edit: None,
            _appearance: appearance,
            command,
            _quit: quit,
            _command: command_entered,
            transfers: transfer::Transfers::default(),
        }
    }

    fn apply_settings(&mut self, settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        if settings.filter() != self.settings.filter() {
            for side in &mut self.panels {
                side.tabs.set_filter(settings.filter());
                side.scroll_to_cursor();
            }
        }
        if settings.viewer.minimap != self.settings.viewer.minimap {
            cx.set_global(ShowMinimap(settings.viewer.minimap));
            cx.refresh_windows();
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

    fn set_theme(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = name;
        self.save_settings(window, cx);
        self.colors = appearance::apply(pick(&self.themes, &self.settings, window), cx);
        menus::set(&self.themes, self.settings.theme.as_deref(), cx);
        self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
        cx.notify();
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
        cx.spawn_in(window, async move |_, cx| {
            let prepare = async move {
                match how {
                    remote::Open::View => origin.view().map(|file| (file, None)),
                    _ => origin.edit(),
                }
            };
            let prepared = cx.background_executor().spawn(prepare).await;
            let _ = cx.update(|window, cx| {
                let opened = prepared.and_then(|(file, upload)| match (how, upload) {
                    (remote::Open::View, _) => {
                        let (files, at) = around.unwrap_or_else(|| (vec![file], 0));
                        viewer::open(files, at, settings.viewer, bounds, cx)
                    }
                    (remote::Open::Edit, upload) => {
                        editor::open(file, settings.editor.limit_mb, upload, cx)
                    }
                    (remote::Open::External, Some(upload)) => {
                        remote::edit_externally(file, upload, &settings.editor.external, window, cx)
                    }
                    (remote::Open::External, None) => {
                        edit::edit_externally(&settings.editor.external, &file)
                    }
                });
                if let Err(err) = opened {
                    window.push_notification(Notification::error(format!("{name}: {err}")), cx);
                }
            });
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
        // On a server, a path typed is the server's.
        let panel = self.panels[ix].tabs.current();
        if let Some(remote) = panel.remote() {
            let target = match typed.strip_prefix(&remote.url()) {
                Some(path) => PathBuf::from(path),
                None => panel.folder().join(&typed),
            };
            self.cancel_path_edit(window, cx);
            return self.navigate_on(
                ix,
                true,
                move |panel| panel.go_to_folder(target),
                window,
                cx,
            );
        }
        let side = &mut self.panels[ix];
        let target = command::resolve(&typed, side.tabs.current().folder());
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
            Item::Command {
                label,
                keys,
                command,
            } => Some((label, keys, command)),
            _ => None,
        });
        h_flex()
            .border_t_1()
            .border_color(cx.theme().border)
            .children(buttons.map(|(label, keys, command)| {
                let key = Keystroke::parse(keys)
                    .map(|key| Kbd::format(&key))
                    .unwrap_or_default();
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
        let height = self.panels[self.active]
            .scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .height;
        ((height / ROW) as isize - 1).max(1)
    }

    /// On a server, in the background.
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

    fn render_panel(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let side = &self.panels[ix];
        let panel = side.tabs.current();
        let active = ix == self.active;
        let (title_bg, title_fg) = if active {
            (theme.primary, theme.primary_foreground)
        } else {
            (theme.muted, theme.muted_foreground)
        };
        let sort = panel.sort();
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
                .when(sorted, |el| {
                    el.bg(theme.accent).text_color(theme.accent_foreground)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by_column(ix, key, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(right, |el| el.text_right())
                        .child(label),
                )
                .when(sorted, |el| el.child(Icon::new(arrow).small()))
        };
        let [ext, size, date, attr] = self.columns.map(px);
        // A column's left edge, as far from the right as the columns right of it.
        let handles = (0..4).map(|column| {
            let right: f32 = self.columns[column..].iter().sum();
            div()
                .id(("resize", ix * 4 + column))
                .absolute()
                .top_0()
                .bottom_0()
                .right(ROW_INSET + px(right - 3.))
                .w(px(6.))
                .occlude()
                .cursor(CursorStyle::ResizeColumn)
                .on_drag(ColumnDrag { panel: ix, column }, |_, _, _, cx| {
                    cx.new(|_| EmptyView)
                })
        });
        let column_titles = h_flex()
            .relative()
            .h(ROW + px(4.))
            .px_1()
            .bg(theme.list_head)
            .border_x_1()
            .border_b_1()
            .border_color(theme.border)
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let drag = event.drag(cx);
                    if drag.panel != ix {
                        return;
                    }
                    let column = drag.column;
                    let right: f32 = this.columns[column + 1..].iter().sum();
                    let edge = event.bounds.right() - ROW_INSET - px(right);
                    let width = f32::from(edge - event.event.position.x);
                    this.columns[column] = width.clamp(COLUMN_MIN, 600.);
                    cx.notify();
                }),
            )
            .child(title("Name", SortKey::Name, false).flex_1().min_w_0())
            .child(title("Ext", SortKey::Extension, false).w(ext).flex_none())
            .child(title("Size", SortKey::Size, true).w(size).flex_none())
            .child(title("Date", SortKey::Modified, false).w(date).flex_none())
            .child(
                title("Attr", SortKey::Attributes, false)
                    .w(attr)
                    .flex_none(),
            )
            .children(handles);
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
        .flex_1();
        let tab_list = self.tab_list.as_ref().filter(|(side, ..)| *side == ix);
        v_flex()
            .size_full()
            .min_w_0()
            .border_1()
            .border_color(theme.border)
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
                    .bg(title_bg)
                    .text_color(title_fg)
                    .border_1()
                    .border_color(gpui_kit::transparent_black())
                    .when_some(
                        self.colors.active_title_border.filter(|_| active),
                        |el, border| el.border_color(border),
                    )
                    .cursor_text()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.edit_path(ix, window, cx)),
                    )
                    .truncate()
                    .child(panel.location())
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
                    .child(panel.status_text()),
            )
    }

    fn render_row(&self, ix: usize, row: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let light = !theme.mode.is_dark();
        let icon = icons::material().icon(&entry.display_name(), entry.is_dir, light);
        let cursor = (row == panel.cursor()).then_some(match ix == self.active {
            true => self.colors.active,
            false => self.colors.inactive,
        });
        let (name, extension) = entry.name_and_extension();
        let [ext, size, date, attr] = self.columns.map(px);
        let marked = panel.is_marked(entry);
        h_flex()
            .id(("row", row))
            .w_full()
            .h(ROW)
            .px_1()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .when_some(cursor, |el, cursor| {
                el.text_color(cursor.foreground)
                    .when_some(cursor.background, |el, bg| el.bg(bg))
                    .when_some(cursor.border, |el, border| el.border_color(border))
            })
            .when(marked, |el| el.text_color(self.colors.marked))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.active = ix;
                this.panels[ix].tabs.current_mut().set_cursor(row);
                cx.notify();
                if event.click_count() == 2 {
                    this.navigate(|panel| panel.open().map(drop), window, cx);
                }
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
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
}

impl Render for Commander {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keeps panel keys out of the menu bar.
        // Typing a path, the panels' keys rest.
        let panels = h_flex()
            .when(self.path_edit.is_none(), |el| el.key_context(CONTEXT))
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h_0()
            .p_1()
            .on_action(cx.listener(|this, _: &SwitchPanel, _, cx| {
                this.active = 1 - this.active;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.move_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.move_cursor(-this.page(), cx)))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.move_cursor(this.page(), cx)))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.move_cursor(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.move_cursor(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| {
                this.navigate(|panel| panel.open().map(drop), window, cx)
            }))
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
                    Command::View => this.open_file(remote::Open::View, window, cx),
                    Command::Edit => this.open_file(remote::Open::Edit, window, cx),
                    Command::EditExternally => this.open_file(remote::Open::External, window, cx),
                    Command::Delete => this.confirm_delete(true, window, cx),
                    Command::DeletePermanently => this.confirm_delete(false, window, cx),
                    Command::ToggleMark => this.mark(Panel::toggle_mark, cx),
                    Command::Copy => this.transfer(ops::Kind::Copy, window, cx),
                    Command::Move => this.transfer(ops::Kind::Move, window, cx),
                    Command::ConnectToServer => {
                        let side = this.active;
                        connections_dialog::open(cx.entity().downgrade(), side, window, cx)
                    }
                    Command::Disconnect => this.disconnect(window, cx),
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
                    _ => cx.propagate(),
                }),
            )
            .on_action(cx.listener(|this, SelectTab(tab): &SelectTab, _, cx| {
                this.select_tab(this.active, *tab, cx)
            }))
            .on_action(cx.listener(|this, ReopenTab(ix): &ReopenTab, window, cx| {
                this.reopen_tab(*ix, window, cx)
            }))
            .on_action(cx.listener(|this, SetTheme(name): &SetTheme, window, cx| {
                this.set_theme(name.as_ref().map(|name| name.to_string()), window, cx)
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SplitDrag>, _, cx| {
                    let bounds = event.bounds;
                    let share = (event.event.position.x - bounds.left()) / bounds.size.width;
                    this.split = match (share - 0.5).abs() < SPLIT_MAGNET {
                        true => 0.5,
                        false => share.clamp(0.15, 0.85),
                    };
                    cx.notify();
                }),
            )
            .child(
                div()
                    .h_full()
                    .min_w_0()
                    .flex_none()
                    .w(relative(self.split))
                    .pr(px(2.))
                    .child(self.render_panel(0, cx)),
            )
            .child(
                div()
                    .h_full()
                    .min_w_0()
                    .flex_1()
                    .pl(px(2.))
                    .child(self.render_panel(1, cx)),
            )
            .child(
                div()
                    .id("splitter")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(self.split))
                    .ml(px(-3.))
                    .w(px(6.))
                    .occlude()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|style| style.bg(cx.theme().list_active))
                    .on_drag(SplitDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
            );
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(
                div()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.menu_bar.clone()),
            )
            .child(panels)
            .children(self.render_transfers(cx))
            .child(self.render_command_line(cx))
            .child(self.render_button_bar(cx))
    }
}

fn pick<'a>(themes: &'a [Theme], settings: &Settings, window: &Window) -> &'a Theme {
    let dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    let named = |name: &str| themes.iter().find(|t| t.name == name);
    settings
        .theme
        .as_deref()
        .and_then(named)
        .or_else(|| named(theme::default_name(dark)))
        .expect("the default themes are built in")
}
