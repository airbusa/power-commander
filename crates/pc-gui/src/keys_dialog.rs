//! Configuration > Shortcuts: each command's keys, a preset's, changed one by one.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::keymap::{self, KeySettings, Preset};
use pc_core::menu::{self, Command};

use crate::Commander;

const WIDTH: f32 = 780.;
const BODY_HEIGHT: f32 = 560.;
const KEY_WIDTH: f32 = 150.;
/// Keys that only change others; a shortcut needs one more.
const MODIFIERS: [&str; 9] = [
    "", "shift", "control", "ctrl", "alt", "platform", "cmd", "super", "function",
];

/// A key as keyboards label it, Ctrl+Shift+F4; a chord's keys one after another.
pub(crate) fn shown(key: &str) -> String {
    let strokes = key.split_whitespace().map(|stroke| {
        Keystroke::parse(stroke).map_or_else(|_| stroke.to_owned(), |key| Kbd::format(&key))
    });
    strokes.collect::<Vec<_>>().join(" ")
}

pub fn open(
    commander: WeakEntity<Commander>,
    keys: KeySettings,
    window: &mut Window,
    cx: &mut App,
) {
    let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search for a command or a key"));
    let view = cx.new(|cx| {
        let this = cx.entity().downgrade();
        // Before the key bindings, so a captured key runs nothing.
        let capture = cx.intercept_keystrokes(move |event, _, cx| {
            let key = &event.keystroke;
            let waiting = this.read_with(cx, |this: &KeysDialog, _| this.capturing.is_some()).unwrap_or(false);
            if waiting && !MODIFIERS.contains(&key.key.as_str()) {
                cx.stop_propagation();
                let _ = this.update(cx, |this, cx| this.captured(key, cx));
            }
        });
        KeysDialog {
            commander,
            saved: keys.clone(),
            draft: keys,
            _search: cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify()
                }
            }),
            search,
            capturing: None,
            _capture: capture,
            note: None,
            groups: menu::groups(),
        }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Shortcuts")
            .width(px(WIDTH))
            .overlay_closable(false)
            .child(view.clone())
    });
}

struct KeysDialog {
    commander: WeakEntity<Commander>,
    /// As last applied.
    saved: KeySettings,
    draft: KeySettings,
    search: Entity<InputState>,
    _search: Subscription,
    /// The slot waiting for a key: a command's shortcut (0) or alternate (1).
    capturing: Option<(Command, usize)>,
    _capture: Subscription,
    /// What the last change did.
    note: Option<(SharedString, bool)>,
    groups: Vec<(&'static str, Vec<(&'static str, Command)>)>,
}

impl KeysDialog {
    fn label(&self, command: Command) -> &'static str {
        let mut all = self.groups.iter().flat_map(|(_, commands)| commands);
        all.find(|(_, c)| *c == command).map_or("", |(label, _)| label)
    }

    fn captured(&mut self, key: &Keystroke, cx: &mut Context<Self>) {
        let Some((command, slot)) = self.capturing.take() else {
            return;
        };
        cx.notify();
        if key.key == "escape" && !key.modifiers.modified() {
            self.note = None;
            return;
        }
        let key = key.unparse();
        let taken = self.draft.set(command, slot, key.clone());
        let name = shown(&key);
        self.note = Some(match (keymap::taken_by(&key, command), taken) {
            (Some(by), _) => (format!("{name} is taken by {by}, so it never reaches PowerCommander.").into(), true),
            (None, Some(other)) => {
                let text = format!("{name} is {} now, no longer {}.", self.label(command), self.label(other));
                (text.into(), false)
            }
            (None, None) => (format!("{name} is {} now.", self.label(command)).into(), false),
        });
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft == self.saved {
            return;
        }
        let keys = self.draft.clone();
        let _ = self.commander.update(cx, |commander, cx| {
            let mut settings = commander.settings.clone();
            settings.keys = keys;
            commander.apply_settings(settings, window, cx)
        });
        self.saved = self.draft.clone();
        cx.notify();
    }

    fn render_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = Preset::ALL.iter().position(|&p| p == self.draft.preset);
        h_flex().gap_3().child("Preset:").child(
            RadioGroup::horizontal("keys-preset")
                .children(Preset::ALL.map(Preset::name))
                .selected_index(chosen)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    let preset = Preset::ALL[*ix];
                    if preset != this.draft.preset {
                        let dropped = !this.draft.changed.is_empty();
                        this.draft = KeySettings { preset, ..Default::default() };
                        this.note = dropped.then(|| {
                            let text = format!("Your own keys gave way to {}'s.", preset.name());
                            (text.into(), false)
                        });
                        cx.notify();
                    }
                })),
        )
    }

    fn render_key(&self, command: Command, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.draft.keys_of(command)[slot].clone();
        let waiting = self.capturing == Some((command, slot));
        let taken = keymap::taken_by(&key, command).filter(|_| !key.is_empty());
        let theme = cx.theme();
        let name = ["key-0", "key-1"][slot];
        let text = match waiting {
            true => "Press a key…".to_owned(),
            false => shown(&key),
        };
        let cell = h_flex()
            .id((name, command as usize))
            .flex_1()
            .min_w_0()
            .h(px(24.))
            .px_2()
            .gap_1()
            .border_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .border_color(if waiting { theme.primary } else { theme.border })
            .when(waiting, |el| el.bg(theme.primary.opacity(0.15)))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(taken.is_some(), |el| el.text_color(theme.danger))
                    .child(text),
            )
            .when_some(taken, |el, by| {
                el.child(Icon::new(IconName::TriangleAlert).small().text_color(theme.warning))
                    .tooltip(move |window, cx| Tooltip::new(format!("Taken by {by}")).build(window, cx))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.capturing = Some((command, slot));
                this.note = Some(("Press the new key; Esc leaves it as it was.".into(), false));
                cx.notify();
            }));
        let clear = Button::new((["clear-0", "clear-1"][slot], command as usize))
            .ghost()
            .xsmall()
            .icon(IconName::Close)
            .tooltip("No key")
            .disabled(key.is_empty())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.draft.set(command, slot, String::new());
                cx.notify();
            }));
        h_flex().w(px(KEY_WIDTH)).gap_1().child(cell).child(clear)
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let matches = |label: &str, command: Command| {
            let keys = self.draft.keys_of(command).map(|key| shown(&key).to_lowercase());
            query.is_empty()
                || label.to_lowercase().contains(&query)
                || keys.iter().any(|key| key.contains(&query))
        };
        let mut list = v_flex();
        for (group, commands) in &self.groups {
            let rows: Vec<_> = commands.iter().filter(|(label, c)| matches(label, *c)).collect();
            if rows.is_empty() {
                continue;
            }
            let theme = cx.theme();
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .bg(theme.muted)
                    .text_color(theme.muted_foreground)
                    .child(*group),
            );
            for &&(label, command) in &rows {
                let changed = self.draft.is_changed(command);
                let reset = Button::new(("reset", command as usize))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Undo2)
                    .tooltip(format!("Back to {}'s keys", self.draft.preset.name()))
                    .disabled(!changed)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.reset(command);
                        cx.notify();
                    }));
                list = list.child(
                    h_flex()
                        .h(px(30.))
                        .px_2()
                        .gap_2()
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .child(self.render_key(command, 0, cx))
                        .child(self.render_key(command, 1, cx))
                        .child(reset),
                );
            }
        }
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_h_0()
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child("Command"))
                    .child(div().w(px(KEY_WIDTH)).child("Shortcut"))
                    .child(div().w(px(KEY_WIDTH)).child("Alternate"))
                    .child(div().w(px(22.))),
            )
            .child(
                div()
                    .id("keys-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
    }
}

impl Render for KeysDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let changed = self.draft != self.saved;
        let note = self.note.clone().map(|(text, warn)| {
            let theme = cx.theme();
            div()
                .flex_1()
                .min_w_0()
                .text_color(if warn { theme.danger } else { theme.muted_foreground })
                .child(text)
        });
        v_flex()
            .w_full()
            .h(px(BODY_HEIGHT))
            .gap_3()
            .text_sm()
            .child(self.render_presets(cx))
            .child(Input::new(&self.search).small())
            .child(self.render_list(cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().children(note))
                    .child(
                        Button::new("keys-reset-all")
                            .outline()
                            .label("Reset All")
                            .disabled(self.draft.changed.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.changed.clear();
                                this.note = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("keys-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(window, cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("keys-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("keys-apply")
                            .outline()
                            .label("Apply")
                            .min_w(px(80.))
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{KeySettings, Keystroke, Preset};

    #[test]
    fn every_preset_key_parses() {
        for preset in Preset::ALL {
            let keys = KeySettings { preset, ..Default::default() };
            for (key, _) in keys.bindings() {
                for stroke in key.split_whitespace() {
                    assert!(Keystroke::parse(stroke).is_ok(), "{}: {key}", preset.name());
                }
            }
        }
    }
}
