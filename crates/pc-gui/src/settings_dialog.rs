use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::settings::{PAGES, Settings};

use crate::Commander;

const WIDTH: f32 = 720.;
const BODY_HEIGHT: f32 = 460.;

pub fn open(
    commander: WeakEntity<Commander>,
    settings: Settings,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|_| SettingsDialog {
        commander,
        saved: settings.clone(),
        draft: settings,
        page: 0,
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

    fn render_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
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
