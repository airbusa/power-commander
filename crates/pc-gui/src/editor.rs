use std::io;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use pc_core::edit;

actions!(editor, [Save]);

const CONTEXT: &str = "TextEditor";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-s", Save, Some(CONTEXT))]);
}

/// Refuses files that editing could change by accident; F3 views those.
pub fn open(path: PathBuf, cx: &mut App) -> io::Result<()> {
    let text = edit::load_text(&path)?;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1000.), px(700.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(title(&path, false).into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| TextEditor::new(path, text, window, cx))
    })
    .map(drop)
    .map_err(|err| io::Error::other(err.to_string()))
}

fn title(path: &std::path::Path, edited: bool) -> String {
    let mark = if edited { "*" } else { "" };
    format!("{mark}{} — Power Commander", path.display())
}

struct TextEditor {
    path: PathBuf,
    editor: Entity<EditorState>,
    edited: bool,
    _changes: Subscription,
}

impl TextEditor {
    fn new(path: PathBuf, text: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language("text")
                .line_number(true)
                .soft_wrap(false)
                .searchable(true);
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
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.may_close(window, cx))
                .unwrap_or(true)
        });
        Self {
            path,
            editor,
            edited: false,
            _changes: changes,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.editor.read(cx).value();
        match edit::save_text(&self.path, &text) {
            Ok(()) => {
                self.edited = false;
                window.set_window_title(&title(&self.path, false));
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

    /// Unsaved changes ask first.
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.edited {
            return true;
        }
        let this = cx.weak_entity();
        let name = self
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        window.open_dialog(cx, move |dialog, _, _| {
            let save = this.clone();
            let buttons = h_flex()
                .gap_2()
                .justify_end()
                .child(Button::new("editor-save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let saved = save.update(cx, |this, cx| this.save(window, cx));
                        if saved.unwrap_or(false) {
                            window.remove_window();
                        }
                    },
                ))
                .child(
                    Button::new("editor-discard")
                        .outline()
                        .label("Don't save")
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("editor-cancel")
                        .outline()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                );
            dialog
                .title("Save changes?")
                .overlay_closable(false)
                .child(format!("{name} has changes that are not saved."))
                .footer(buttons)
        });
        false
    }
}

impl Render for TextEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .key_context(CONTEXT)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                this.save(window, cx);
            }))
            .child(Editor::new(&self.editor).bordered(false).size_full())
            .child(
                h_flex()
                    .px_2()
                    .gap_4()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.path.display().to_string()),
                    )
                    .child(if self.edited { "Modified" } else { "" })
                    .child("Ctrl+S saves"),
            )
    }
}
