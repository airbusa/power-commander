use std::io;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Icon, IconName, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::minimap::{self, Minimap};
use pc_core::edit;

pub use pc_core::origin::Upload;

actions!(editor, [Save]);

const CONTEXT: &str = "TextEditor";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-s", Save, Some(CONTEXT))]);
}

/// Refuses files over `limit_mb`; ones editing could change by accident
/// open to a warning, as VS Code's, and as text only on request.
pub fn open(
    path: PathBuf,
    limit_mb: u64,
    upload: Option<Upload>,
    over: crate::Over,
    cx: &mut App,
) -> io::Result<()> {
    let (text, unfit) = match edit::load_text(&path, limit_mb)? {
        edit::Loaded::Text(text) => (text, None),
        edit::Loaded::Unfit { why, anyway } => (String::new(), Some((why, anyway))),
    };
    let options = crate::over_main_window(title(&path, false), over, cx);
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| TextEditor::new(path, text, unfit, upload, window, cx))
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
    upload: Option<Upload>,
    editor: Entity<EditorState>,
    edited: bool,
    /// Not to edit as text: why, and the text shown on request.
    unfit: Option<(&'static str, String)>,
    /// Shown anyway: the first save asks, as it changes what is not text.
    forced: bool,
    minimap: Entity<Minimap>,
    _changes: Subscription,
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
        let lines = rows.map(|row| {
            text.slice_line(row)
                .chars()
                .take(minimap::COLUMNS)
                .collect()
        });
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

impl TextEditor {
    fn new(
        path: PathBuf,
        text: String,
        unfit: Option<(&'static str, String)>,
        upload: Option<Upload>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // macOS: the application and Window menus only.
        #[cfg(target_os = "macos")]
        cx.observe_window_activation(window, |_, window, cx| crate::menus::show(window, Vec::new(), cx))
            .detach();
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
        let map = EditorMap(editor.clone());
        Self {
            path,
            upload,
            editor,
            edited: false,
            unfit,
            forced: false,
            minimap: cx.new(|_| Minimap::new(map)),
            _changes: changes,
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
                        let sent = cx
                            .background_executor()
                            .spawn(async move { upload(&local) });
                        let note = match sent.await {
                            Ok(text) => Notification::success(text),
                            Err(err) => {
                                Notification::error(format!("Could not save it back: {err}"))
                                    .autohide(false)
                            }
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

    /// Saving what was shown anyway writes the text as shown.
    fn confirm_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let save = this.clone();
            let buttons = h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("editor-save-anyway")
                        .danger()
                        .label("Save anyway")
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let _ = save.update(cx, |this, cx| {
                                this.forced = false;
                                this.save(window, cx)
                            });
                        }),
                )
                .child(
                    Button::new("editor-save-cancel")
                        .outline()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                );
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        // Its close button asks about changes, as the desktop's would.
        let header = crate::header::title_bar(title(&self.path, self.edited), window, cx).map(|bar| {
            bar.on_close_window(move |_, window, cx| {
                if this.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true) {
                    window.remove_window();
                }
            })
        });
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
            .child(match self.unfit {
                Some((why, _)) => v_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size(px(40.))
                            .text_color(theme.warning),
                    )
                    .child(format!("The file is not shown in the editor because {why}."))
                    .child(
                        Button::new("editor-open-anyway")
                            .primary()
                            .label("Open Anyway")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_anyway(window, cx)
                            })),
                    )
                    .into_any_element(),
                None => h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.editor).bordered(false).flex_1().h_full())
                    .when(crate::minimap_shown(cx), |el| {
                        el.child(self.minimap.clone())
                    })
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
