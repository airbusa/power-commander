//! Find Files in a window of its own, as Total Commander's: the General,
//! Advanced and Load/Save pages, the results below as they come, F3 and F4
//! on them, Go to file and Feed to listbox.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerState, DateTime};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::origin::Origin;
use pc_core::search::{self, Age, Attribute, Duplicates, Event, Found, Query, Saved, SizeOp, Text, Unit};
use pc_core::{grouped, local_time};

use crate::{Commander, MainWindow, editor, viewer};

const CONTEXT: &str = "FindFiles";
const ROW_HEIGHT: f32 = 22.;
const PAGES: [&str; 3] = ["General", "Advanced", "Load/Save"];

actions!(find_files, [ViewFound, EditFound, InFound, StopOrClose]);

pub(crate) fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("f3", ViewFound, keys),
        KeyBinding::new("f4", EditFound, keys),
        KeyBinding::new("f2", InFound, keys),
        KeyBinding::new("escape", StopOrClose, keys),
    ]);
}

/// Alt+F7's window, while it is open: Alt+F7 brings it back.
struct OpenSearch(AnyWindowHandle);

impl Global for OpenSearch {}

impl Commander {
    /// Find Files from the active panel's folder; `separate`: one more
    /// window, which stays when going to a file.
    pub(crate) fn find_files(&mut self, separate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !separate
            && let Some(OpenSearch(open)) = cx.try_global::<OpenSearch>()
            && open.update(cx, |_, window, _| window.activate_window()).is_ok()
        {
            return;
        }
        let panel = self.panels[self.active].tabs.current();
        let local = panel.remote().is_none();
        let folder = match local {
            true => panel.folder().to_owned(),
            false => std::env::home_dir().unwrap_or_default(),
        };
        // "Only search in selected": the marked ones, not the cursor's.
        let chosen: Vec<PathBuf> = match local && !panel.in_archive() {
            true => panel.marked_names().iter().map(|name| folder.join(name)).collect(),
            false => Vec::new(),
        };
        let main = window.bounds();
        let size = size(px(820.), px(700.)).min(&main.size);
        let origin = main.origin + point((main.size.width - size.width) / 2., (main.size.height - size.height) / 2.);
        let bounds = WindowBounds::Windowed(Bounds { origin, size });
        let options = crate::over_main_window("Find Files".into(), bounds, cx);
        let commander = cx.entity().downgrade();
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| FindFiles::new(commander, separate, folder, chosen, window, cx))
        });
        match opened {
            Ok((handle, _)) if !separate => cx.set_global(OpenSearch(handle)),
            Ok(_) => {}
            Err(err) => window.push_notification(Notification::error(err.to_string()), cx),
        }
    }
}

/// What the search's thread found since the window last looked.
#[derive(Default)]
struct Progress {
    found: Vec<Found>,
    folder: Option<PathBuf>,
    done: Option<Result<(), String>>,
}

struct Number {
    on: bool,
    input: Entity<InputState>,
}

struct FindFiles {
    commander: WeakEntity<Commander>,
    separate: bool,
    page: usize,
    names: Entity<InputState>,
    places: Entity<InputState>,
    /// The choices of General and Advanced without their typed texts.
    query: Query,
    find_text: bool,
    text: Entity<InputState>,
    text_options: Text,
    between: bool,
    from: Entity<DatePickerState>,
    to: Entity<DatePickerState>,
    newer: Number,
    newer_age: Age,
    older: Number,
    older_age: Age,
    size: Number,
    size_op: SizeOp,
    size_unit: Unit,
    attributes_on: bool,
    /// Wanted, not wanted, or either.
    attributes: [Option<bool>; 5],
    duplicates_on: bool,
    duplicates: Duplicates,
    /// The panel's marked files and folders, for "Only search in selected".
    chosen: Vec<PathBuf>,
    only_chosen: bool,
    has_locate: bool,
    saved: Saved,
    saved_at: Option<usize>,
    save_name: Entity<InputState>,
    results: Vec<Found>,
    cursor: Option<usize>,
    /// Earlier searches' results, for < and >; `at` is the shown one's.
    earlier: Vec<Vec<Found>>,
    at: usize,
    /// F2: the next search looks only at these results.
    in_found: bool,
    stop: Option<Arc<AtomicBool>>,
    status: String,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _inputs: Vec<Subscription>,
}

impl FindFiles {
    fn new(
        commander: WeakEntity<Commander>,
        separate: bool,
        folder: PathBuf,
        chosen: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let names = input("*.txt *.md | backup*  (empty: all)", window, cx);
        let places = input("Folders, ; between them", window, cx);
        places.update(cx, |places, cx| places.set_value(folder.display().to_string(), window, cx));
        let text = input("Text to find", window, cx);
        let number = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_owned();
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let day = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"))
        };
        // Enter in a field starts the search.
        let start = |this: &mut Self, _: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
            if let InputEvent::PressEnter { .. } = event {
                this.start(window, cx);
            }
        };
        let _inputs = vec![
            cx.subscribe_in(&names, window, start),
            cx.subscribe_in(&places, window, start),
            cx.subscribe_in(&text, window, start),
        ];
        names.update(cx, |names, cx| names.focus(window, cx));
        let saved = Saved::load().unwrap_or_default();
        Self {
            commander,
            separate,
            page: 0,
            names,
            places,
            query: Query::default(),
            find_text: false,
            text,
            text_options: Text::default(),
            between: false,
            from: day(window, cx),
            to: day(window, cx),
            newer: Number { on: false, input: number("1", window, cx) },
            newer_age: Age::Days,
            older: Number { on: false, input: number("1", window, cx) },
            older_age: Age::Days,
            size: Number { on: false, input: number("1", window, cx) },
            size_op: SizeOp::More,
            size_unit: Unit::Kilo,
            attributes_on: false,
            attributes: [None; 5],
            duplicates_on: false,
            duplicates: Duplicates { name: true, size: true, content: false },
            only_chosen: false,
            chosen,
            has_locate: search::has_locate(),
            saved,
            saved_at: None,
            save_name: input("Name of the search", window, cx),
            results: Vec::new(),
            cursor: None,
            earlier: Vec::new(),
            at: 0,
            in_found: false,
            stop: None,
            status: String::new(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _inputs,
        }
    }

    /// What the pages say, as a search to run or save.
    fn read_query(&self, cx: &App) -> Result<Query, String> {
        let number = |number: &Number, what: &str| -> Result<Option<u64>, String> {
            let text = number.input.read(cx).value();
            match number.on {
                true => text.trim().parse().map(Some).map_err(|_| format!("{what} needs a number, not {text}")),
                false => Ok(None),
            }
        };
        let day = |state: &Entity<DatePickerState>| match state.read(cx).date_time() {
            DateTime::Single(Some(time)) => Ok(time.date().format("%Y-%m-%d").to_string()),
            _ => Err("Date between needs both days".to_owned()),
        };
        let between = match self.between {
            true => Some((day(&self.from)?, day(&self.to)?)),
            false => None,
        };
        let attributes = Attribute::ALL.into_iter().zip(self.attributes);
        let attributes = attributes.filter_map(|(attribute, wanted)| Some((attribute, wanted?)));
        Ok(Query {
            names: self.names.read(cx).value().to_string(),
            places: self.places.read(cx).value().to_string(),
            text: self.find_text.then(|| Text {
                text: self.text.read(cx).value().to_string(),
                ..self.text_options.clone()
            }),
            between,
            newer: number(&self.newer, "Not older than")?.map(|n| (n as u32, self.newer_age)),
            older: number(&self.older, "Older than")?.map(|n| (n as u32, self.older_age)),
            size: number(&self.size, "File size")?.map(|n| (self.size_op, n, self.size_unit)),
            attributes: if self.attributes_on { attributes.collect() } else { Vec::new() },
            duplicates: self.duplicates_on.then_some(self.duplicates),
            ..self.query.clone()
        })
    }

    /// Fills the pages with a saved search.
    fn load(&mut self, query: Query, window: &mut Window, cx: &mut Context<Self>) {
        let set = |input: &Entity<InputState>, text: String, window: &mut Window, cx: &mut Context<Self>| {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        };
        set(&self.names, query.names.clone(), window, cx);
        set(&self.places, query.places.clone(), window, cx);
        self.find_text = query.text.is_some();
        if let Some(text) = &query.text {
            set(&self.text, text.text.clone(), window, cx);
            self.text_options = text.clone();
        }
        self.between = false;
        if let Some((from, to)) = &query.between
            && let (Some(from), Some(to)) = (search::day(from), search::day(to))
        {
            self.between = true;
            self.from.update(cx, |state, cx| state.set_date(from, window, cx));
            self.to.update(cx, |state, cx| state.set_date(to, window, cx));
        }
        let number = |number: &mut Number, value: Option<u64>, window: &mut Window, cx: &mut Context<Self>| {
            number.on = value.is_some();
            if let Some(value) = value {
                number.input.update(cx, |input, cx| input.set_value(value.to_string(), window, cx));
            }
        };
        number(&mut self.newer, query.newer.map(|(n, _)| n as u64), window, cx);
        number(&mut self.older, query.older.map(|(n, _)| n as u64), window, cx);
        number(&mut self.size, query.size.map(|(_, n, _)| n), window, cx);
        self.newer_age = query.newer.map_or(Age::Days, |(_, age)| age);
        self.older_age = query.older.map_or(Age::Days, |(_, age)| age);
        (self.size_op, self.size_unit) = query.size.map_or((SizeOp::More, Unit::Kilo), |(op, _, unit)| (op, unit));
        self.attributes_on = !query.attributes.is_empty();
        self.attributes = Attribute::ALL.map(|attribute| {
            query.attributes.iter().find(|(wanted, _)| *wanted == attribute).map(|(_, on)| *on)
        });
        self.duplicates_on = query.duplicates.is_some();
        self.duplicates = query.duplicates.unwrap_or(self.duplicates);
        self.query = query;
        cx.notify();
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_search();
        let query = match self.read_query(cx) {
            Ok(query) => query,
            Err(err) => return window.push_notification(Notification::error(err), cx),
        };
        let places = match (self.in_found, self.only_chosen && !self.chosen.is_empty()) {
            (true, _) => self.results.iter().filter(|hit| hit.member.is_none()).map(|hit| hit.path.clone()).collect(),
            (false, true) => self.chosen.clone(),
            (false, false) => search::places(&query.places),
        };
        let saved = &mut self.saved;
        Saved::remember(&mut saved.names, &query.names);
        Saved::remember(&mut saved.places, &query.places);
        if let Some(text) = &query.text {
            Saved::remember(&mut saved.texts, &text.text);
        }
        if let Err(err) = saved.save() {
            window.push_notification(Notification::error(format!("Could not keep the search: {err}")), cx);
        }
        // The results shown go to the history.
        self.earlier.truncate(self.at);
        if !self.results.is_empty() {
            self.earlier.push(std::mem::take(&mut self.results));
        }
        self.at = self.earlier.len();
        self.cursor = None;
        self.in_found = false;
        let stop = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress::default()));
        let (stopped, reported) = (stop.clone(), progress.clone());
        std::thread::spawn(move || {
            let result = search::run(&query, &places, &stopped, &mut |event| {
                let mut progress = reported.lock().unwrap();
                match event {
                    Event::Folder(folder) => progress.folder = Some(folder),
                    Event::Found(found) => progress.found.push(found),
                }
            });
            reported.lock().unwrap().done = Some(result);
        });
        self.stop = Some(stop);
        self.status = "Searching…".into();
        // The results show as they come, ten times a second.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = this.update(cx, |this, cx| this.take(&progress, cx));
                if done.unwrap_or(true) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// True once the search is over.
    fn take(&mut self, progress: &Mutex<Progress>, cx: &mut Context<Self>) -> bool {
        let mut progress = progress.lock().unwrap();
        self.results.append(&mut progress.found);
        if self.cursor.is_none() && !self.results.is_empty() {
            self.cursor = Some(0);
        }
        let (folder, done) = (progress.folder.take(), progress.done.take());
        drop(progress);
        if let Some(folder) = folder {
            self.status = format!("Searching: {}", folder.display());
        }
        cx.notify();
        let Some(done) = done else {
            return false;
        };
        let stopped = self.stop.take().is_some_and(|stop| stop.load(Ordering::Relaxed));
        self.status = match done {
            Err(err) => err,
            Ok(()) if stopped => format!("Stopped: {}", self.counts()),
            Ok(()) => self.counts(),
        };
        true
    }

    fn counts(&self) -> String {
        let dirs = self.results.iter().filter(|hit| hit.is_dir).count();
        let files = self.results.len() - dirs;
        format!("{files} files and {dirs} directories found")
    }

    fn stop_search(&mut self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn stop_or_close(&mut self, window: &mut Window) {
        match &self.stop {
            Some(stop) => stop.store(true, Ordering::Relaxed),
            None => window.remove_window(),
        }
    }

    /// < and >: an earlier search's results, or a later one's.
    fn step_history(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.stop.is_some() {
            return;
        }
        let at = self.at.saturating_add_signed(by);
        if at > self.earlier.len() || at == self.at {
            return;
        }
        // The shown results take their place in the list.
        let shown = std::mem::take(&mut self.results);
        match self.at == self.earlier.len() {
            true => self.earlier.push(shown),
            false => self.earlier[self.at] = shown,
        }
        self.results = std::mem::take(&mut self.earlier[at]);
        if at == self.earlier.len() - 1 {
            self.earlier.pop();
        }
        self.at = at;
        self.cursor = (!self.results.is_empty()).then_some(0);
        self.status = self.counts();
        cx.notify();
    }

    fn current(&self) -> Option<&Found> {
        self.results.get(self.cursor?)
    }

    fn move_cursor(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let at = self.cursor.map_or(0, |at| at.saturating_add_signed(by)).min(self.results.len() - 1);
        self.cursor = Some(at);
        self.scroll.scroll_to_item(at, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// F3 and F4. F3 goes on through the files found with Next and Previous.
    fn open_found(&mut self, edit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(found) = self.current().filter(|found| !found.is_dir).cloned() else {
            return;
        };
        let Some(commander) = self.commander.upgrade() else {
            return;
        };
        let settings = commander.read(cx).settings.clone();
        let bounds = window.window_bounds();
        // As Lister from Total Commander's search: at the text found.
        let text = self.query_text(cx);
        if let Some(text) = &text {
            viewer::remember_find(text, cx);
        }
        let view = match text.is_some() {
            true => viewer::open_finding,
            false => viewer::open,
        };
        let files: Vec<PathBuf> = self.results.iter().filter(|hit| !hit.is_dir && hit.member.is_none()).map(|hit| hit.path.clone()).collect();
        if !edit && found.member.is_none() {
            let at = files.iter().position(|path| *path == found.path).unwrap_or(0);
            if let Err(err) = view(files, at, settings.viewer, bounds, cx) {
                window.push_notification(Notification::error(format!("{}: {err}", found.shown())), cx);
            }
            return;
        }
        let origin = match found.member.clone() {
            Some(member) => Origin::Archive(member, None),
            None => Origin::Local(found.path.clone()),
        };
        cx.spawn_in(window, async move |_, cx| {
            let taken = cx.background_spawn(async move {
                match edit {
                    true => origin.edit(),
                    false => origin.view().map(|file| (file, None)),
                }
            });
            let taken = taken.await;
            let _ = cx.update(|window, cx| {
                let opened = taken.and_then(|(file, upload)| match edit {
                    true => editor::open(file, settings.editor.limit_mb, upload, bounds, cx),
                    false => view(vec![file], 0, settings.viewer, bounds, cx),
                });
                if let Err(err) = opened {
                    window.push_notification(Notification::error(format!("{}: {err}", found.shown())), cx);
                }
            });
        })
        .detach();
    }

    /// The text searched for, for the viewer's Find.
    fn query_text(&self, cx: &App) -> Option<Text> {
        let text = self.text.read(cx).value().to_string();
        (self.find_text && !text.is_empty() && !self.text_options.not_containing).then(|| Text {
            text,
            ..self.text_options.clone()
        })
    }

    /// Go to file: the active panel shows it, the cursor on it.
    fn go_to_found(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(found) = self.current().cloned() else {
            return;
        };
        let member = found.member.map(|member| member.inside);
        self.in_main(window, cx, move |panel| panel.go_to_file(&found.path, member.as_deref()));
    }

    /// Feed to listbox: the active panel lists the files found.
    fn feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self.results.iter().filter(|hit| hit.member.is_none()).map(|hit| hit.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        self.in_main(window, cx, move |panel| {
            panel.show_found(paths);
            Ok(())
        });
    }

    /// Moves the main window's active panel; Alt+F7's window closes, as Total Commander's dialog.
    fn in_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        go: impl FnOnce(&mut pc_core::Panel) -> std::io::Result<()> + Send + 'static,
    ) {
        let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
            return;
        };
        let _ = main.update(cx, |_, main, cx| {
            main.activate_window();
            commander.update(cx, |this, cx| this.navigate(go, main, cx))
        });
        if !self.separate {
            window.remove_window();
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.save_name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return window.push_notification(Notification::warning("Name the search first."), cx);
        }
        let query = match self.read_query(cx) {
            Ok(query) => query,
            Err(err) => return window.push_notification(Notification::error(err), cx),
        };
        let searches = &mut self.saved.searches;
        match searches.iter().position(|(saved, _)| *saved == name) {
            Some(at) => searches[at].1 = query,
            None => searches.push((name, query)),
        }
        self.saved_at = searches.len().checked_sub(1);
        self.write_saved(window, cx);
    }

    fn delete_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.saved_at.filter(|at| *at < self.saved.searches.len()) else {
            return;
        };
        self.saved.searches.remove(at);
        self.saved_at = None;
        self.write_saved(window, cx);
    }

    fn write_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.saved.save() {
            window.push_notification(Notification::error(format!("Could not save the searches: {err}")), cx);
        }
        cx.notify();
    }

    fn load_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, query)) = self.saved_at.and_then(|at| self.saved.searches.get(at)).cloned() else {
            return;
        };
        self.save_name.update(cx, |input, cx| input.set_value(name, window, cx));
        self.load(query, window, cx);
        self.page = 0;
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Search in".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let text: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
            let _ = this.update_in(cx, |this, window, cx| {
                this.places.update(cx, |places, cx| places.set_value(text.join("; "), window, cx));
            });
        })
        .detach();
    }

    // Widgets.

    fn check(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        on: bool,
        set: fn(&mut Self, bool),
        cx: &mut Context<Self>,
    ) -> Checkbox {
        Checkbox::new(id).label(label.into()).checked(on).on_click(cx.listener(move |this, checked: &bool, _, cx| {
            set(this, *checked);
            cx.notify();
        }))
    }

    /// A button showing `current`, its menu the choices.
    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        choices: Vec<(T, String)>,
        current: T,
        set: fn(&mut Self, T),
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<T> {
        let label = choices.iter().find(|(value, _)| *value == current).map_or(String::new(), |(_, label)| label.clone());
        let this = cx.weak_entity();
        Button::new(id).small().outline().label(label).dropdown_caret(true).disabled(disabled).dropdown_menu(move |mut menu: PopupMenu, _, _| {
            for (value, label) in &choices {
                let (this, value) = (this.clone(), *value);
                let item = PopupMenuItem::new(label.clone()).checked(value == current).on_click(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        set(this, value);
                        cx.notify();
                    });
                });
                menu = menu.item(item);
            }
            menu
        })
    }

    /// The ▾ beside a field: what was typed there before.
    fn history(&self, id: &'static str, input: &Entity<InputState>, earlier: &[String]) -> impl IntoElement + use<> {
        let (input, earlier) = (input.clone(), earlier.to_vec());
        Button::new(id).small().ghost().icon(IconName::ChevronDown).disabled(earlier.is_empty()).dropdown_menu(
            move |mut menu: PopupMenu, _, _| {
                for text in &earlier {
                    let (input, text) = (input.clone(), text.clone());
                    let item = PopupMenuItem::new(text.clone()).on_click(move |_, window, cx| {
                        input.update(cx, |input, cx| input.set_value(text.clone(), window, cx))
                    });
                    menu = menu.item(item);
                }
                menu
            },
        )
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str| div().w(px(90.)).flex_none().child(text);
        let drives = {
            let places = self.places.clone();
            Button::new("drives").small().outline().label("Drives").dropdown_caret(true).dropdown_menu(
                move |mut menu: PopupMenu, _, _| {
                    for drive in search::drives() {
                        let (places, text) = (places.clone(), drive.display().to_string());
                        let item = PopupMenuItem::new(text.clone()).on_click(move |_, window, cx| {
                            places.update(cx, |places, cx| places.set_value(text.clone(), window, cx))
                        });
                        menu = menu.item(item);
                    }
                    menu
                },
            )
        };
        let mut depths = vec![(None, "all (unlimited depth)".to_owned()), (Some(0), "current dir only".to_owned())];
        depths.extend((1..=9).map(|n| (Some(n), format!("{n} level{}", if n == 1 { "" } else { "s" }))));
        let text_off = !self.find_text;
        let options = &self.text_options;
        let ansi = pc_core::codepage::system().0;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(label("Search for:"))
                    .child(div().flex_1().child(Input::new(&self.names).small()))
                    .child(self.history("names-history", &self.names, &self.saved.names)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(label("Search in:"))
                    .child(div().flex_1().child(Input::new(&self.places).small().disabled(self.only_chosen)))
                    .child(self.history("places-history", &self.places, &self.saved.places))
                    .child(Button::new("browse").small().outline().label(">>").on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))))
                    .child(drives),
            )
            .child(
                h_flex()
                    .gap_6()
                    .child(self.check("names-regex", "RegEx", self.query.regex, |this, on| this.query.regex = on, cx))
                    .child(
                        self.check(
                            "only-chosen",
                            match self.chosen.len() {
                                0 => "Only search in selected files/folders".to_owned(),
                                n => format!("Only search in the {n} selected"),
                            },
                            self.only_chosen,
                            |this, on| this.only_chosen = on,
                            cx,
                        )
                        .disabled(self.chosen.is_empty()),
                    )
                    .child(self.check("archives", "Search archives", self.query.archives, |this, on| this.query.archives = on, cx))
                    .when(self.has_locate, |row| {
                        row.child(self.check("locate", "From the locate index", self.query.locate, |this, on| this.query.locate = on, cx))
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child("Search in subdirectories:")
                    .child(self.choice("depth", depths, self.query.depth, |this, depth| this.query.depth = depth, false, cx)),
            )
            .child(div().h_px().bg(cx.theme().border))
            .child(
                h_flex()
                    .gap_1()
                    .child(div().w(px(90.)).flex_none().child(self.check("find-text", "Find text:", self.find_text, |this, on| this.find_text = on, cx)))
                    .child(div().flex_1().child(Input::new(&self.text).small().disabled(text_off)))
                    .child(self.history("texts-history", &self.text, &self.saved.texts)),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_8()
                    .pl(px(94.))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(self.check("whole-words", "Whole words only", options.whole_words, |this, on| this.text_options.whole_words = on, cx).disabled(text_off))
                            .child(self.check("case", "Case sensitive", options.case_sensitive, |this, on| this.text_options.case_sensitive = on, cx).disabled(text_off))
                            .child(self.check("text-regex", "RegEx", options.regex, |this, on| this.text_options.regex = on, cx).disabled(text_off))
                            .child(self.check("hex", "Hex", options.hex, |this, on| this.text_options.hex = on, cx).disabled(text_off))
                            .child(
                                self.check("not-containing", "Find files NOT containing the text", options.not_containing, |this, on| this.text_options.not_containing = on, cx)
                                    .disabled(text_off),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(self.check("utf8", "UTF-8", options.utf8, |this, on| this.text_options.utf8 = on, cx).disabled(text_off))
                            .child(self.check("utf16", "UTF-16", options.utf16, |this, on| this.text_options.utf16 = on, cx).disabled(text_off))
                            .child(self.check("ansi", format!("ANSI (code page {ansi})"), options.ansi, |this, on| this.text_options.ansi = on, cx).disabled(text_off))
                            .child(
                                self.check("office", "Office files (docx, xlsx, pptx, odt…)", options.office, |this, on| this.text_options.office = on, cx)
                                    .disabled(text_off),
                            ),
                    ),
            )
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ages = || Age::ALL.map(|age| (age, age.name().to_owned())).to_vec();
        let number = |number: &Number| div().w(px(70.)).child(Input::new(&number.input).small().disabled(!number.on));
        let label = |check: Checkbox| div().w(px(150.)).flex_none().child(check);
        let attributes = Attribute::ALL.into_iter().enumerate().map(|(at, attribute)| {
            // Click: wanted, not wanted, either.
            let (mark, tip) = match self.attributes[at] {
                Some(true) => ("✓", "Must be"),
                Some(false) => ("✗", "Must not be"),
                None => ("–", "Either"),
            };
            Button::new(("attribute", at))
                .small()
                .ghost()
                .label(format!("{mark} {}", attribute.name()))
                .tooltip(tip)
                .disabled(!self.attributes_on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.attributes[at] = match this.attributes[at] {
                        None => Some(true),
                        Some(true) => Some(false),
                        Some(false) => None,
                    };
                    cx.notify();
                }))
        });
        let attributes: Vec<Button> = attributes.collect();
        let duplicates = &self.duplicates;
        let off = !self.duplicates_on;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(label(self.check("between", "Date between:", self.between, |this, on| this.between = on, cx)))
                    .child(div().w(px(150.)).child(DatePicker::new(&self.from).small().disabled(!self.between)))
                    .child("and")
                    .child(div().w(px(150.)).child(DatePicker::new(&self.to).small().disabled(!self.between))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(label(self.check("newer", "Not older than:", self.newer.on, |this, on| this.newer.on = on, cx)))
                    .child(number(&self.newer))
                    .child(self.choice("newer-age", ages(), self.newer_age, |this, age| this.newer_age = age, !self.newer.on, cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(label(self.check("older", "Older than:", self.older.on, |this, on| this.older.on = on, cx)))
                    .child(number(&self.older))
                    .child(self.choice("older-age", ages(), self.older_age, |this, age| this.older_age = age, !self.older.on, cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(label(self.check("size", "File size:", self.size.on, |this, on| this.size.on = on, cx)))
                    .child(
                        self.choice("size-op", SizeOp::ALL.map(|op| (op, op.name().to_owned())).to_vec(), self.size_op, |this, op| this.size_op = op, !self.size.on, cx),
                    )
                    .child(number(&self.size))
                    .child(
                        self.choice("size-unit", Unit::ALL.map(|unit| (unit, unit.name().to_owned())).to_vec(), self.size_unit, |this, unit| this.size_unit = unit, !self.size.on, cx),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(label(self.check("attributes", "Attributes:", self.attributes_on, |this, on| this.attributes_on = on, cx)))
                    .children(attributes),
            )
            .child(div().h_px().bg(cx.theme().border))
            .child(self.check("duplicates", "Find duplicate files:", self.duplicates_on, |this, on| this.duplicates_on = on, cx))
            .child(
                h_flex()
                    .gap_6()
                    .pl(px(24.))
                    .child(self.check("same-name", "same name", duplicates.name, |this, on| this.duplicates.name = on, cx).disabled(off))
                    .child(self.check("same-size", "same size", duplicates.size, |this, on| this.duplicates.size = on, cx).disabled(off))
                    .child(self.check("same-content", "same contents", duplicates.content, |this, on| this.duplicates.content = on, cx).disabled(off)),
            )
    }

    fn render_saved(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.saved.searches.iter().enumerate().map(|(at, (name, _))| {
            div()
                .id(("saved", at))
                .px_2()
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .when(self.saved_at == Some(at), |row| row.bg(theme.list_active).text_color(theme.foreground))
                .child(name.clone())
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.saved_at = Some(at);
                    if event.click_count() == 2 {
                        this.load_saved(window, cx);
                    }
                    cx.notify();
                }))
        });
        let chosen = self.saved_at.is_some();
        v_flex()
            .gap_2()
            .size_full()
            .child("Previous searches:")
            .child(
                v_flex()
                    .id("saved-searches")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .children(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(220.)).child(Input::new(&self.save_name).small()))
                    .child(Button::new("save").small().outline().label("Save").on_click(cx.listener(|this, _, window, cx| this.save(window, cx))))
                    .child(
                        Button::new("load")
                            .small()
                            .outline()
                            .label("Load")
                            .disabled(!chosen)
                            .on_click(cx.listener(|this, _, window, cx| this.load_saved(window, cx))),
                    )
                    .child(
                        Button::new("delete")
                            .small()
                            .outline()
                            .label("Delete")
                            .disabled(!chosen)
                            .on_click(cx.listener(|this, _, window, cx| this.delete_saved(window, cx))),
                    ),
            )
    }

    fn render_row(&self, at: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let found = &self.results[at];
        // Duplicates: a line between the sets.
        let new_set = at > 0 && found.group.is_some() && self.results[at - 1].group != found.group;
        let size = match found.is_dir {
            true => "<DIR>".to_owned(),
            false => grouped(found.size),
        };
        let time = found.modified.map(|time| local_time(time, "%Y-%m-%d %H:%M")).unwrap_or_default();
        h_flex()
            .id(("found", at))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_4()
            .when(new_set, |row| row.border_t_1().border_color(theme.border))
            .when(self.cursor == Some(at), |row| row.bg(theme.list_active))
            // The end of a long path: the name.
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_start().child(found.shown()))
            .child(div().w(px(110.)).flex_none().text_right().child(size))
            .child(div().w(px(130.)).flex_none().text_color(theme.muted_foreground).child(time))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.cursor = Some(at);
                window.focus(&this.focus, cx);
                if event.click_count() == 2 {
                    this.go_to_found(window, cx);
                }
                cx.notify();
            }))
    }
}

impl Render for FindFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = crate::header::title_bar("Find Files", window, cx);
        let page = match self.page {
            0 => self.render_general(cx).into_any_element(),
            1 => self.render_advanced(cx).into_any_element(),
            _ => self.render_saved(cx).into_any_element(),
        };
        let searching = self.stop.is_some();
        let some = self.current().is_some();
        let file = self.current().is_some_and(|found| !found.is_dir);
        let theme = cx.theme().clone();
        let list = uniform_list(
            "found",
            self.results.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| range.map(|at| this.render_row(at, cx)).collect()),
        )
        .track_scroll(&self.scroll)
        .flex_1();
        let button = |id: &'static str, label: &'static str| Button::new(id).small().outline().label(label);
        v_flex()
            .key_context(CONTEXT)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .children(header)
            .on_action(cx.listener(|this, _: &ViewFound, window, cx| this.open_found(false, window, cx)))
            .on_action(cx.listener(|this, _: &EditFound, window, cx| this.open_found(true, window, cx)))
            .on_action(cx.listener(|this, _: &InFound, _, cx| {
                this.in_found = !this.in_found && !this.results.is_empty();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &StopOrClose, window, _| this.stop_or_close(window)))
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .p_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                TabBar::new("pages")
                                    .small()
                                    .underline()
                                    .selected_index(self.page)
                                    .children(PAGES.map(|name| Tab::new().label(name)))
                                    .on_click(cx.listener(|this, page: &usize, _, cx| {
                                        this.page = *page;
                                        cx.notify();
                                    })),
                            )
                            .child(div().h(px(300.)).child(page)),
                    )
                    .child(
                        v_flex()
                            .w(px(110.))
                            .gap_2()
                            .pt_8()
                            .child(match searching {
                                true => Button::new("stop")
                                    .small()
                                    .label("Stop")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.stop_search();
                                        cx.notify();
                                    })),
                                false => Button::new("start")
                                    .small()
                                    .primary()
                                    .label("Start search")
                                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                            })
                            .child(button("close", "Close").on_click(|_, window, _| window.remove_window())),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .gap_2()
                    .child("Search results:")
                    .child(div().flex_1())
                    .child(
                        Button::new("earlier")
                            .xsmall()
                            .ghost()
                            .icon(IconName::ChevronLeft)
                            .disabled(searching || self.at == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.step_history(-1, cx))),
                    )
                    .child(
                        Button::new("later")
                            .xsmall()
                            .ghost()
                            .icon(IconName::ChevronRight)
                            .disabled(searching || self.at >= self.earlier.len())
                            .on_click(cx.listener(|this, _, _, cx| this.step_history(1, cx))),
                    )
                    .child(
                        Checkbox::new("in-found")
                            .label("F2 Search in found files")
                            .checked(self.in_found)
                            .disabled(self.results.is_empty() || searching)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.in_found = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("results")
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .mx_3()
                    .mt_1()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| match event.keystroke.key.as_str() {
                        "up" => this.move_cursor(-1, cx),
                        "down" => this.move_cursor(1, cx),
                        "pageup" => this.move_cursor(-20, cx),
                        "pagedown" => this.move_cursor(20, cx),
                        "home" => this.move_cursor(isize::MIN, cx),
                        "end" => this.move_cursor(isize::MAX, cx),
                        "enter" => this.go_to_found(window, cx),
                        _ => cx.propagate(),
                    }))
                    .child(list),
            )
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .child(button("view", "F3 View").disabled(!file).on_click(cx.listener(|this, _, window, cx| this.open_found(false, window, cx))))
                    .child(button("edit", "F4 Edit").disabled(!file).on_click(cx.listener(|this, _, window, cx| this.open_found(true, window, cx))))
                    .child(button("new", "New search").disabled(searching).on_click(cx.listener(|this, _, window, cx| {
                        this.results.clear();
                        this.cursor = None;
                        this.in_found = false;
                        this.status.clear();
                        this.page = 0;
                        this.names.update(cx, |names, cx| names.focus(window, cx));
                        cx.notify();
                    })))
                    .child(button("go", "Go to file").disabled(!some).on_click(cx.listener(|this, _, window, cx| this.go_to_found(window, cx))))
                    .child(
                        button("feed", "Feed to listbox")
                            .disabled(searching || !self.results.iter().any(|hit| hit.member.is_none()))
                            .on_click(cx.listener(|this, _, window, cx| this.feed(window, cx))),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pb_2()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(self.status.clone()),
            )
    }
}

impl Drop for FindFiles {
    fn drop(&mut self) {
        self.stop_search();
    }
}
