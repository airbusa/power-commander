//! F4's editor: GPUI Kit's code editor in the file's language colours, its
//! View options kept in the settings, VS Code's line and case commands, and
//! VS Code's Find and Replace (`find_bar`).

use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::base::input::{AddCursorAbove, AddCursorBelow};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{self, Editor, EditorState, Escape, InputEvent, Position, RopeExt, TabSize};
use gpui_kit::component::menu::{AppMenuBar, MenuBuilder};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Icon, IconName, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::find_bar::{FindBar, SelectAllMatches};
use crate::menus::Entries;
use crate::minimap::{self, Minimap};
use pc_core::edit;
use pc_core::settings::EditorSettings;
use pc_core::text_edit::{self, TextCase, TextEdit};

pub use pc_core::origin::Upload;

actions!(
    editor,
    [
        Save,
        SaveAs,
        Reload,
        Close,
        FindNext,
        FindPrevious,
        FindWordAtCaret,
        GoToLine,
        UpperCase,
        LowerCase,
        TitleCase,
        DuplicateLines,
        DeleteLines,
        MoveLinesUp,
        MoveLinesDown,
        SortLines,
        SortLinesDescending,
        TrimTrailingWhitespace,
        ToggleComment,
        ToggleWrap,
        ToggleLineNumbers,
        ToggleWhitespace,
        ToggleIndentGuides,
        ToggleFolding,
        UnfoldAll,
        ToggleMinimap,
        ToggleHardTabs
    ]
);

/// The View menu's tab widths.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = editor, no_json)]
struct SetTabSize(usize);

/// The Language menu's: a highlighter's name; none goes by the file's name.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = editor, no_json)]
struct SetLanguage(Option<&'static str>);

const CONTEXT: &str = "TextEditor";
const MENUS: [&str; 5] = ["File", "Edit", "Selection", "View", "Language"];

/// VS Code's keys, where it has them.
pub fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-s", Save, keys),
        KeyBinding::new("secondary-shift-s", SaveAs, keys),
        KeyBinding::new("secondary-w", Close, keys),
        KeyBinding::new("f3", FindNext, keys),
        KeyBinding::new("shift-f3", FindPrevious, keys),
        KeyBinding::new("secondary-f3", FindWordAtCaret, keys),
        KeyBinding::new("secondary-g", GoToLine, keys),
        KeyBinding::new("secondary-shift-u", UpperCase, keys),
        KeyBinding::new("secondary-u", LowerCase, keys),
        KeyBinding::new("secondary-shift-d", DuplicateLines, keys),
        KeyBinding::new("secondary-shift-k", DeleteLines, keys),
        KeyBinding::new("alt-up", MoveLinesUp, keys),
        KeyBinding::new("alt-down", MoveLinesDown, keys),
        KeyBinding::new("secondary-/", ToggleComment, keys),
        KeyBinding::new("secondary-shift-l", SelectAllMatches, keys),
        KeyBinding::new("alt-z", ToggleWrap, keys),
    ]);
    crate::find_bar::bind_keys(cx);
}

/// Refuses files over the settings' limit; ones editing could change by
/// accident open to a warning, as VS Code's, and as text only on request.
pub fn open(
    path: PathBuf,
    options: EditorSettings,
    upload: Option<Upload>,
    over: crate::Over,
    cx: &mut App,
) -> io::Result<()> {
    let (text, unfit) = match edit::load_text(&path, options.limit_mb)? {
        edit::Loaded::Text(text) => (text, None),
        edit::Loaded::Unfit { why, anyway } => (String::new(), Some((why, anyway))),
    };
    let window = crate::over_main_window(title(&path, false), over, cx);
    gpui_kit::open_window(window, cx, |window, cx| {
        cx.new(|cx| TextEditor::new(path, text, unfit, upload, options, window, cx))
    })
    .map(drop)
    .map_err(|err| io::Error::other(err.to_string()))
}

fn title(path: &std::path::Path, edited: bool) -> String {
    let mark = if edited { "*" } else { "" };
    format!("{mark}{} — Editor", path.display())
}

struct TextEditor {
    path: PathBuf,
    upload: Option<Upload>,
    editor: Entity<EditorState>,
    edited: bool,
    /// Not to edit as text: why, and the text shown on request.
    unfit: Option<(&'static str, String)>,
    /// Shown anyway: the first save asks, as it changes what is not text.
    forced: bool,
    /// The View menu's options; the tab width and tabs are this file's.
    options: EditorSettings,
    /// Highlighted as; chosen in the Language menu, else by the file's name.
    language: &'static str,
    chosen_language: bool,
    crlf: bool,
    find_bar: Entity<FindBar>,
    menu_bar: Entity<AppMenuBar>,
    minimap: Entity<Minimap>,
    _subscriptions: [Subscription; 3],
}

/// The minimap's view of the text.
struct EditorMap(Entity<EditorState>);

impl EditorMap {
    fn line_height(&self, cx: &App) -> Option<f32> {
        self.0.read(cx).line_height().map(f32::from)
    }
}

impl minimap::Source for EditorMap {
    fn position(&self, height: f32, cx: &App) -> Option<minimap::Position> {
        let line_height = self.line_height(cx)?;
        let editor = self.0.read(cx);
        let rows = editor.text().lines_len().max(1) as f32;
        let visible = height / line_height;
        // The editor scrolls past its last line by half its height, at least three lines.
        let beyond = (visible / 2.).max(3.);
        Some(minimap::Position {
            first: -f32::from(editor.scroll_offset().y) / line_height,
            visible,
            max_first: (rows + beyond - visible).max(0.),
        })
    }

    fn lines(&self, rows: std::ops::Range<usize>, cx: &App) -> Vec<String> {
        let text = self.0.read(cx).text();
        let rows = rows.start..rows.end.min(text.lines_len());
        let lines = rows.map(|row| text.slice_line(row).chars().take(minimap::COLUMNS).collect());
        lines.collect()
    }

    fn scroll_to(&self, first: f32, cx: &mut App) {
        let line_height = self.line_height(cx).unwrap_or(16.);
        self.0.update(cx, |editor, cx| {
            let x = editor.scroll_offset().x;
            editor.set_scroll_offset(point(x, px(-first * line_height)), cx);
        });
    }

    fn wheel(&self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut App) {
        self.0.update(cx, |editor, cx| {
            let line_height = editor.line_height().unwrap_or(px(16.));
            let offset = editor.scroll_offset();
            let delta = event.delta.pixel_delta(line_height);
            editor.set_scroll_offset(point(offset.x, offset.y + delta.y), cx);
        });
    }
}

/// The editor's menus, each built as it opens.
fn menu_bar(this: WeakEntity<TextEditor>, cx: &mut App) -> Entity<AppMenuBar> {
    let menus = MENUS.map(|name| {
        let this = this.clone();
        let build: MenuBuilder = Rc::new(move |menu, _, cx| match this.upgrade() {
            Some(this) => {
                let this = this.read(cx);
                let focus = this.editor.read(cx).focus_handle(cx);
                this.menu(name, menu.action_context(focus), cx)
            }
            None => menu,
        });
        (name.into(), build)
    });
    AppMenuBar::built(menus.into(), cx)
}

impl TextEditor {
    fn new(
        path: PathBuf,
        text: String,
        unfit: Option<(&'static str, String)>,
        upload: Option<Upload>,
        mut options: EditorSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = match unfit {
            Some(_) => "text",
            None => text_edit::language_of(&path),
        };
        (options.tab_size, options.hard_tabs) = text_edit::detect_indent(&text).unwrap_or((options.tab_size, options.hard_tabs));
        let crlf = text.contains("\r\n");
        let editor = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language(language)
                .line_number(options.line_numbers)
                .soft_wrap(options.wrap)
                .folding(options.folding)
                .indent_guides(options.indent_guides)
                .tab_size(TabSize { tab_size: options.tab_size, hard_tabs: options.hard_tabs })
                // Find is the find bar's.
                .searchable(false);
            editor.set_show_whitespaces(options.whitespace, window, cx);
            editor.set_value(text, window, cx);
            editor.focus(window, cx);
            editor
        });
        let changes = cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
            if let InputEvent::Change = event
                && !this.edited
            {
                this.edited = true;
                window.set_window_title(&title(&this.path, true));
                cx.notify();
            }
        });
        // The status bar's line and column.
        let moved = cx.observe(&editor, |_, _, cx| cx.notify());
        let find_bar = cx.new(|cx| FindBar::new(editor.clone(), window, cx));
        let found = cx.observe(&find_bar, |_, _, cx| cx.notify());
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| this.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true));
        let map = EditorMap(editor.clone());
        Self {
            path,
            upload,
            editor,
            edited: false,
            unfit,
            forced: false,
            options,
            language,
            chosen_language: false,
            crlf,
            find_bar,
            menu_bar: menu_bar(cx.weak_entity(), cx),
            minimap: cx.new(|_| Minimap::new(map)),
            _subscriptions: [changes, moved, found],
        }
    }

    /// One of the menus, as it is now.
    fn menu<M: Entries>(&self, name: &str, menu: M, cx: &App) -> M {
        let o = &self.options;
        let unfit = self.unfit.is_some();
        match name {
            "File" => menu
                .entry("Save", false, Box::new(Save), unfit)
                .entry("Save as...", false, Box::new(SaveAs), unfit)
                .separator()
                .entry("Reload", false, Box::new(Reload), unfit)
                .separator()
                .entry("Close", false, Box::new(Close), false),
            "Edit" => menu
                .entry("Undo", false, Box::new(input::Undo), unfit)
                .entry("Redo", false, Box::new(input::Redo), unfit)
                .separator()
                .entry("Cut", false, Box::new(input::Cut), unfit)
                .entry("Copy", false, Box::new(input::Copy), unfit)
                .entry("Paste", false, Box::new(input::Paste), unfit)
                .separator()
                .entry("Find...", false, Box::new(input::Search), unfit)
                .entry("Replace...", false, Box::new(input::Replace), unfit)
                .entry("Find next", false, Box::new(FindNext), unfit)
                .entry("Find previous", false, Box::new(FindPrevious), unfit)
                .separator()
                .entry("Go to line...", false, Box::new(GoToLine), unfit),
            "Selection" => {
                let comment = text_edit::line_comment(self.language).is_none();
                menu.entry("Select all", false, Box::new(input::SelectAll), unfit)
                    .entry("Select all occurrences", false, Box::new(SelectAllMatches), unfit)
                    .entry("Add cursor above", false, Box::new(AddCursorAbove), unfit)
                    .entry("Add cursor below", false, Box::new(AddCursorBelow), unfit)
                    .separator()
                    .entry("UPPERCASE", false, Box::new(UpperCase), unfit)
                    .entry("lowercase", false, Box::new(LowerCase), unfit)
                    .entry("Title Case", false, Box::new(TitleCase), unfit)
                    .separator()
                    .entry("Duplicate lines", false, Box::new(DuplicateLines), unfit)
                    .entry("Move lines up", false, Box::new(MoveLinesUp), unfit)
                    .entry("Move lines down", false, Box::new(MoveLinesDown), unfit)
                    .entry("Delete lines", false, Box::new(DeleteLines), unfit)
                    .separator()
                    .entry("Sort lines ascending", false, Box::new(SortLines), unfit)
                    .entry("Sort lines descending", false, Box::new(SortLinesDescending), unfit)
                    .entry("Trim trailing whitespace", false, Box::new(TrimTrailingWhitespace), unfit)
                    .separator()
                    .entry("Toggle line comment", false, Box::new(ToggleComment), unfit || comment)
                    .entry("Indent", false, Box::new(input::Indent), unfit)
                    .entry("Outdent", false, Box::new(input::Outdent), unfit)
            }
            "View" => {
                let mut menu = menu
                    .entry("Word wrap", o.wrap, Box::new(ToggleWrap), false)
                    .entry("Line numbers", o.line_numbers, Box::new(ToggleLineNumbers), false)
                    .entry("Whitespace", o.whitespace, Box::new(ToggleWhitespace), false)
                    .entry("Indent guides", o.indent_guides, Box::new(ToggleIndentGuides), false)
                    .entry("Folding", o.folding, Box::new(ToggleFolding), false)
                    .entry("Unfold all", false, Box::new(UnfoldAll), !o.folding)
                    .separator()
                    .entry("Minimap", crate::minimap_shown(cx), Box::new(ToggleMinimap), false)
                    .separator();
                for width in [2, 4, 8] {
                    menu = menu.entry(format!("Tab width {width}"), o.tab_size == width, Box::new(SetTabSize(width)), false);
                }
                menu.entry("Indent with tabs", o.hard_tabs, Box::new(ToggleHardTabs), false)
            }
            _ => {
                let mut menu = menu.entry("By the file's name", !self.chosen_language, Box::new(SetLanguage(None)), false).separator();
                for &(label, name) in text_edit::LANGUAGES {
                    menu = menu.entry(label, self.language == name, Box::new(SetLanguage(Some(name))), false);
                }
                menu
            }
        }
    }

    fn open_anyway(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, text)) = self.unfit.take() else {
            return;
        };
        self.forced = true;
        self.editor.update(cx, |editor, cx| {
            editor.set_value(text, window, cx);
            editor.focus(window, cx);
        });
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.unfit.is_some() {
            return false;
        }
        if self.forced {
            self.confirm_save(window, cx);
            return false;
        }
        let text = self.editor.read(cx).value();
        match edit::save_text(&self.path, &text) {
            Ok(()) => {
                self.edited = false;
                window.set_window_title(&title(&self.path, false));
                if let Some(upload) = self.upload.clone() {
                    let local = self.path.clone();
                    cx.spawn_in(window, async move |_, cx| {
                        let sent = cx.background_executor().spawn(async move { upload(&local) });
                        let note = match sent.await {
                            Ok(text) => Notification::success(text),
                            Err(err) => Notification::error(format!("Could not save it back: {err}")).autohide(false),
                        };
                        let _ = cx.update(|window, cx| window.push_notification(note, cx));
                    })
                    .detach();
                }
                cx.notify();
                true
            }
            Err(err) => {
                let text = format!("Could not save {}: {err}", self.path.display());
                window.push_notification(Notification::error(text).autohide(false), cx);
                false
            }
        }
    }

    /// Saves to a new file, which the editor edits from then on.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unfit.is_some() {
            return;
        }
        let folder = self.path.parent().unwrap_or(std::path::Path::new("/")).to_owned();
        let name = self.path.file_name().map(|name| name.to_string_lossy().into_owned());
        let target = cx.prompt_for_new_path(&folder, name.as_deref());
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(to))) = target.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let from = std::mem::replace(&mut this.path, to);
                // A server's file is saved back no more: this is another.
                let upload = this.upload.take();
                if !this.save(window, cx) {
                    (this.path, this.upload) = (from, upload);
                    return;
                }
                if !this.chosen_language {
                    this.set_language(None, cx);
                }
            });
        })
        .detach();
    }

    /// The file again from the disk, after asking about changes.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edited {
            return self.read_again(window, cx);
        }
        let this = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let reload = this.clone();
            let buttons = h_flex()
                .gap_2()
                .justify_end()
                .child(Button::new("editor-reload").danger().label("Reload").on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    let _ = reload.update(cx, |this, cx| this.read_again(window, cx));
                }))
                .child(Button::new("editor-reload-cancel").outline().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)));
            dialog
                .title("Reload the file?")
                .overlay_closable(false)
                .child("The changes that are not saved are lost.")
                .footer(buttons)
        });
    }

    fn read_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match edit::load_text(&self.path, self.options.limit_mb) {
            Ok(edit::Loaded::Text(text)) => {
                self.crlf = text.contains("\r\n");
                self.editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
                self.edited = false;
                window.set_window_title(&title(&self.path, false));
                cx.notify();
            }
            Ok(edit::Loaded::Unfit { why, .. }) => {
                window.push_notification(Notification::error(format!("Not reloaded: {why}.")), cx);
            }
            Err(err) => {
                window.push_notification(Notification::error(format!("Could not reload it: {err}")), cx);
            }
        }
    }

    /// Saving what was shown anyway writes the text as shown.
    fn confirm_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let save = this.clone();
            let buttons = h_flex()
                .gap_2()
                .justify_end()
                .child(Button::new("editor-save-anyway").danger().label("Save anyway").on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    let _ = save.update(cx, |this, cx| {
                        this.forced = false;
                        this.save(window, cx)
                    });
                }))
                .child(Button::new("editor-save-cancel").outline().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)));
            dialog
                .title("Save it as text?")
                .overlay_closable(false)
                .child(
                    "The file was not text: saving writes it as shown, \
                     so whatever was not text changes for good.",
                )
                .footer(buttons)
        });
    }

    /// Unsaved changes ask first.
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.edited {
            return true;
        }
        let this = cx.weak_entity();
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        window.open_dialog(cx, move |dialog, _, _| {
            let save = this.clone();
            let buttons = h_flex()
                .gap_2()
                .justify_end()
                .child(Button::new("editor-save").primary().label("Save").on_click(move |_, window, cx| {
                    let saved = save.update(cx, |this, cx| this.save(window, cx));
                    if saved.unwrap_or(false) {
                        window.remove_window();
                    }
                }))
                .child(Button::new("editor-discard").outline().label("Don't save").on_click(|_, window, _| window.remove_window()))
                .child(Button::new("editor-cancel").outline().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)));
            dialog
                .title("Save changes?")
                .overlay_closable(false)
                .child(format!("{name} has changes that are not saved."))
                .footer(buttons)
        });
        false
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.may_close(window, cx) {
            window.remove_window();
        }
    }

    /// A change to the text from `make`, given the text and the selection, as one step to undo.
    fn change(&mut self, make: impl FnOnce(&str, std::ops::Range<usize>) -> Option<TextEdit>, window: &mut Window, cx: &mut Context<Self>) {
        if self.unfit.is_some() {
            return;
        }
        let (text, selection) = {
            let editor = self.editor.read(cx);
            (editor.value(), editor.selected_range())
        };
        let Some(edit) = make(&text, selection) else {
            return;
        };
        if edit.range.is_empty() && edit.text.is_empty() {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.set_selected_range(edit.range, cx);
            editor.replace(edit.text, window, cx);
            editor.set_selected_range(edit.selection, cx);
            editor.focus(window, cx);
        });
    }

    fn toggle_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prefix) = text_edit::line_comment(self.language) else {
            return;
        };
        self.change(|text, selection| Some(text_edit::toggle_comment(text, selection, prefix)), window, cx);
    }

    fn go_to_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.editor.read(cx).cursor_position().line + 1;
        let editor = self.editor.clone();
        crate::name_dialog::open("Go to Line", "Line, or line:column", line.to_string(), window, cx, move |typed, window, cx| {
            let mut parts = typed.trim().splitn(2, ':').map(|part| part.trim().parse::<u32>().ok());
            let Some(Some(line)) = parts.next() else {
                return;
            };
            let column = parts.next().flatten().unwrap_or(1);
            editor.update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(line.saturating_sub(1), column.saturating_sub(1)), window, cx);
            });
        });
    }

    /// Changes a View option, for this file and the next.
    fn view(&mut self, change: impl Fn(&mut EditorSettings), window: &mut Window, cx: &mut Context<Self>) {
        change(&mut self.options);
        let o = self.options.clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_soft_wrap(o.wrap, window, cx);
            editor.set_line_number(o.line_numbers, window, cx);
            editor.set_show_whitespaces(o.whitespace, window, cx);
            editor.set_indent_guides(o.indent_guides, window, cx);
            editor.set_folding(o.folding, window, cx);
            editor.set_tab_size(TabSize { tab_size: o.tab_size, hard_tabs: o.hard_tabs }, cx);
        });
        crate::change_settings(cx, move |settings| change(&mut settings.editor));
        cx.notify();
    }

    fn set_language(&mut self, chosen: Option<&'static str>, cx: &mut Context<Self>) {
        self.chosen_language = chosen.is_some();
        self.language = chosen.unwrap_or_else(|| match self.forced {
            true => "text",
            false => text_edit::language_of(&self.path),
        });
        let language = self.language;
        self.editor.update(cx, |editor, cx| editor.set_highlighter(language, cx));
        cx.notify();
    }

    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        match self.find_bar.read(cx).is_open() {
            true => self.find_bar.update(cx, |bar, cx| bar.close(window, cx)),
            false => cx.propagate(),
        }
    }

    /// Line and column, what is selected, the indent, line ends and language.
    fn status(&self, cx: &App) -> Vec<String> {
        let editor = self.editor.read(cx);
        let at = editor.cursor_position();
        let selected = editor.selected_range();
        let mut place = format!("Ln {}, Col {}", at.line + 1, at.character + 1);
        if !selected.is_empty() {
            place.push_str(&format!(" ({} selected)", editor.selected_text().to_string().chars().count()));
        }
        let indent = match self.options.hard_tabs {
            true => format!("Tab size: {}", self.options.tab_size),
            false => format!("Spaces: {}", self.options.tab_size),
        };
        let language = text_edit::LANGUAGES.iter().find(|(_, name)| *name == self.language).map_or("Plain Text", |(label, _)| label);
        let ends = if self.crlf { "CRLF" } else { "LF" };
        vec![place, indent, "UTF-8".into(), ends.into(), language.into()]
    }
}

impl Render for TextEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        {
            let menus = MENUS.map(|name| Menu::new(name).items(self.menu(name, Vec::new(), cx)));
            crate::menus::show(window, menus.into(), cx);
        }
        let this = cx.weak_entity();
        // Its close button asks about changes, as the desktop's would.
        let header = crate::header::title_bar(title(&self.path, self.edited), window, cx).map(|bar| {
            bar.on_close_window(move |_, window, cx| {
                if this.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true) {
                    window.remove_window();
                }
            })
        });
        let find_open = self.find_bar.read(cx).is_open();
        let status = self.status(cx);
        let theme = cx.theme();
        v_flex()
            .key_context(CONTEXT)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .children(header)
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                this.save(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &Reload, window, cx| this.reload(window, cx)))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &input::Search, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.toggle(false, window, cx))
            }))
            .on_action(cx.listener(|this, _: &input::Replace, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.toggle(true, window, cx))
            }))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.find_again(true, window, cx))
            }))
            .on_action(cx.listener(|this, _: &FindPrevious, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.find_again(false, window, cx))
            }))
            .on_action(cx.listener(|this, _: &FindWordAtCaret, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.find_word_at_caret(window, cx))
            }))
            .on_action(cx.listener(|this, _: &SelectAllMatches, window, cx| {
                this.find_bar.update(cx, |bar, cx| bar.select_all(window, cx))
            }))
            .on_action(cx.listener(|this, _: &GoToLine, window, cx| this.go_to_line(window, cx)))
            .on_action(cx.listener(|this, _: &UpperCase, window, cx| {
                this.change(|text, sel| text_edit::change_case(text, sel, TextCase::Upper), window, cx)
            }))
            .on_action(cx.listener(|this, _: &LowerCase, window, cx| {
                this.change(|text, sel| text_edit::change_case(text, sel, TextCase::Lower), window, cx)
            }))
            .on_action(cx.listener(|this, _: &TitleCase, window, cx| {
                this.change(|text, sel| text_edit::change_case(text, sel, TextCase::Title), window, cx)
            }))
            .on_action(cx.listener(|this, _: &DuplicateLines, window, cx| {
                this.change(|text, sel| Some(text_edit::duplicate_lines(text, sel)), window, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteLines, window, cx| {
                this.change(|text, sel| Some(text_edit::delete_lines(text, sel)), window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLinesUp, window, cx| {
                this.change(|text, sel| text_edit::move_lines(text, sel, true), window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLinesDown, window, cx| {
                this.change(|text, sel| text_edit::move_lines(text, sel, false), window, cx)
            }))
            .on_action(cx.listener(|this, _: &SortLines, window, cx| {
                this.change(|text, sel| Some(text_edit::sort_lines(text, sel, false)), window, cx)
            }))
            .on_action(cx.listener(|this, _: &SortLinesDescending, window, cx| {
                this.change(|text, sel| Some(text_edit::sort_lines(text, sel, true)), window, cx)
            }))
            .on_action(cx.listener(|this, _: &TrimTrailingWhitespace, window, cx| {
                this.change(text_edit::trim_trailing_whitespace, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleComment, window, cx| this.toggle_comment(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleWrap, window, cx| this.view(|o| o.wrap = !o.wrap, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleLineNumbers, window, cx| {
                this.view(|o| o.line_numbers = !o.line_numbers, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleWhitespace, window, cx| {
                this.view(|o| o.whitespace = !o.whitespace, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleIndentGuides, window, cx| {
                this.view(|o| o.indent_guides = !o.indent_guides, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleFolding, window, cx| this.view(|o| o.folding = !o.folding, window, cx)))
            .on_action(cx.listener(|this, _: &UnfoldAll, _, cx| this.editor.update(cx, |editor, cx| editor.unfold_all(cx))))
            .on_action(cx.listener(|this, SetTabSize(width): &SetTabSize, window, cx| {
                let width = *width;
                this.view(move |o| o.tab_size = width, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHardTabs, window, cx| this.view(|o| o.hard_tabs = !o.hard_tabs, window, cx)))
            .on_action(cx.listener(|this, SetLanguage(chosen): &SetLanguage, _, cx| this.set_language(*chosen, cx)))
            .on_action(|_: &ToggleMinimap, _, cx| crate::toggle_minimap(cx))
            // macOS shows the menus on the system's menu bar.
            .when(!cfg!(target_os = "macos"), |el| {
                el.child(div().px_1().border_b_1().border_color(theme.border).child(self.menu_bar.clone()))
            })
            .child(match self.unfit {
                Some((why, _)) => v_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(Icon::new(IconName::TriangleAlert).size(px(40.)).text_color(theme.warning))
                    .child(format!("The file is not shown in the editor because {why}."))
                    .child(
                        Button::new("editor-open-anyway")
                            .primary()
                            .label("Open Anyway")
                            .on_click(cx.listener(|this, _, window, cx| this.open_anyway(window, cx))),
                    )
                    .into_any_element(),
                None => h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .on_action(cx.listener(Self::escape))
                            .child(Editor::new(&self.editor).bordered(false).size_full())
                            .when(find_open, |el| {
                                el.child(div().absolute().top_1().right(px(16.)).child(self.find_bar.clone()))
                            }),
                    )
                    .when(crate::minimap_shown(cx), |el| el.child(self.minimap.clone()))
                    .into_any_element(),
            })
            .child(
                h_flex()
                    .px_2()
                    .gap_4()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(div().flex_1().min_w_0().truncate().child(self.path.display().to_string()))
                    .when(self.edited, |el| el.child("Modified"))
                    .when(self.unfit.is_none(), |el| el.children(status)),
            )
    }
}
