use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::settings::{PAGES, Settings, StartFont, StartMode};

use crate::Commander;

const WIDTH: f32 = 720.;
const BODY_HEIGHT: f32 = 460.;

const START_MODES: [StartMode; 4] = [
    StartMode::Text,
    StartMode::Binary,
    StartMode::Hex,
    StartMode::Auto,
];
const START_FONTS: [StartFont; 4] = [
    StartFont::Ansi,
    StartFont::Ascii,
    StartFont::Variable,
    StartFont::Auto,
];

/// Opens on `PAGES[page]`.
pub fn open(
    commander: WeakEntity<Commander>,
    settings: Settings,
    page: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let mut width = |value: usize, cx: &mut App| {
        cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_string())
                .validate(|text, _| text.len() <= 4 && text.chars().all(|c| c.is_ascii_digit()))
        })
    };
    let text_width = width(settings.viewer.text_width, cx);
    let binary_width = width(settings.viewer.binary_width, cx);
    let view = cx.new(|cx| {
        let changed =
            |cx: &mut Context<SettingsDialog>, input: &Entity<InputState>, binary: bool| {
                cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                    let InputEvent::Change = event else {
                        return;
                    };
                    if let Ok(width @ 1..) = input.read(cx).value().parse() {
                        let viewer = &mut this.draft.viewer;
                        *(if binary {
                            &mut viewer.binary_width
                        } else {
                            &mut viewer.text_width
                        }) = width;
                        cx.notify();
                    }
                })
            };
        SettingsDialog {
            _widths: [
                changed(cx, &text_width, false),
                changed(cx, &binary_width, true),
            ],
            text_width,
            binary_width,
            commander,
            saved: settings.clone(),
            draft: settings,
            page,
        }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Settings")
            .width(px(WIDTH))
            .overlay_closable(false)
            .child(view.clone())
    });
}

struct SettingsDialog {
    commander: WeakEntity<Commander>,
    /// As last applied.
    saved: Settings,
    draft: Settings,
    page: usize,
    text_width: Entity<InputState>,
    binary_width: Entity<InputState>,
    _widths: [Subscription; 2],
}

impl SettingsDialog {
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft == self.saved {
            return;
        }
        let draft = self.draft.clone();
        let _ = self.commander.update(cx, |commander, cx| {
            commander.apply_settings(draft, window, cx)
        });
        self.saved = self.draft.clone();
        cx.notify();
    }

    fn render_pages(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("settings-pages")
            .w(px(190.))
            .h_full()
            .p_1()
            .border_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .children(PAGES.iter().enumerate().map(|(ix, &(depth, label))| {
                div()
                    .id(("settings-page", ix))
                    .pl(px(6. + 14. * depth as f32))
                    .pr_2()
                    .when(ix == self.page, |el| {
                        el.bg(theme.accent).text_color(theme.accent_foreground)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = ix;
                        cx.notify();
                    }))
                    .child(label)
            }))
    }

    fn render_viewer(&self, cx: &mut Context<Self>) -> GroupBox {
        let viewer = &self.draft.viewer;
        let start = START_MODES.iter().position(|&mode| mode == viewer.start);
        let font = START_FONTS.iter().position(|&font| font == viewer.font);
        let width = |label: &'static str, input: &Entity<InputState>| {
            h_flex()
                .gap_2()
                .child(label)
                .child(div().w(px(70.)).child(Input::new(input)))
        };
        let start = v_flex().gap_2().child("Display at startup:").child(
            RadioGroup::vertical("viewer-start")
                .children(["Text", "Binary", "Hex", "Autodetect"])
                .selected_index(start)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.draft.viewer.start = START_MODES[*ix];
                    cx.notify();
                })),
        );
        let font = v_flex().gap_2().child("Font at startup:").child(
            RadioGroup::vertical("viewer-font")
                .children([
                    "ANSI (Windows)",
                    "ASCII (DOS)",
                    "Variable width",
                    "Autodetect",
                ])
                .selected_index(font)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.draft.viewer.font = START_FONTS[*ix];
                    cx.notify();
                })),
        );
        GroupBox::new().outline().title("Viewer (F3)").child(
            v_flex()
                .gap_4()
                .child(h_flex().items_start().gap_12().child(start).child(font))
                .child(
                    Checkbox::new("viewer-wrap")
                        .label("Wrap text")
                        .checked(viewer.wrap)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.viewer.wrap = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("viewer-minimap")
                        .label("Show the minimap (Viewer and Editor)")
                        .checked(viewer.minimap)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.viewer.minimap = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    h_flex()
                        .gap_6()
                        .child("Display width (characters):")
                        .child(width("Text", &self.text_width))
                        .child(width("Binary", &self.binary_width)),
                ),
        )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, title) = PAGES[self.page];
        let content = match title {
            "Display" => Some(
                GroupBox::new().outline().title("File display").child(
                    h_flex()
                        .gap_6()
                        .child(
                            Checkbox::new("show-hidden")
                                .label("Show hidden files")
                                .checked(self.draft.show_hidden)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.draft.show_hidden = *checked;
                                    cx.notify();
                                })),
                        )
                        .child(
                            // Linux and macOS have no system attribute.
                            Checkbox::new("show-system")
                                .label("Show system files")
                                .checked(self.draft.show_system)
                                .disabled(!cfg!(windows))
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.draft.show_system = *checked;
                                    cx.notify();
                                })),
                        ),
                ),
            ),
            "Edit/View" => Some(self.render_viewer(cx)),
            "Folder Tabs" => Some(
                GroupBox::new().outline().title("Folder tab headers").child(
                    Checkbox::new("tab-header-always")
                        .label("Show tab header also when there is only one tab")
                        .checked(self.draft.tab_header_always)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.tab_header_always = *checked;
                            cx.notify();
                        })),
                ),
            ),
            _ => None,
        };
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .h_full()
            .gap_3()
            .child(
                div()
                    .py_1()
                    .text_center()
                    .bg(theme.muted)
                    .border_1()
                    .border_color(theme.border)
                    .child(title),
            )
            .children(content)
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let changed = self.draft != self.saved;
        v_flex()
            .w_full()
            .h(px(BODY_HEIGHT))
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .gap_3()
                    .child(self.render_pages(cx))
                    .child(self.render_page(cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("settings-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(window, cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("settings-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("settings-apply")
                            .outline()
                            .label("Apply")
                            .min_w(px(80.))
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}
