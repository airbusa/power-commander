use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::settings::{MenuStyle, PAGES, Settings, SidebarBlur, SidebarOpacity, StartFont, StartMode};

use crate::Commander;

const WIDTH: f32 = 720.;
const BODY_HEIGHT: f32 = 550.;

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
    let mut input = |value: String, digits: bool, cx: &mut App| {
        cx.new(|cx| {
            let state = InputState::new(window, cx).default_value(value);
            match digits {
                true => state.validate(|text, _| {
                    text.len() <= 4 && text.chars().all(|c| c.is_ascii_digit())
                }),
                false => state,
            }
        })
    };
    let text_width = input(settings.viewer.text_width.to_string(), true, cx);
    let binary_width = input(settings.viewer.binary_width.to_string(), true, cx);
    let limit = input(settings.editor.limit_mb.to_string(), true, cx);
    let external = input(settings.editor.external.clone(), false, cx);
    let opacity = cx.new(|_| {
        let value = f32::from(settings.sidebar_opacity.0);
        SliderState::new().min(0.).max(100.).step(5.).default_value(value)
    });
    let view = cx.new(|cx| {
        // Tried on the sidebar as it moves; closed, the dialog puts back what was applied.
        let trying = cx.subscribe_in(&opacity, window, |this: &mut SettingsDialog, _, event: &SliderEvent, window, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let opacity = value.end().round().clamp(0., 100.) as u8;
            this.draft.sidebar_opacity = SidebarOpacity(opacity);
            this.try_backdrop(window, cx);
        });
        // Each input's text into the draft, as typed.
        let bind = |cx: &mut Context<SettingsDialog>,
                    input: &Entity<InputState>,
                    set: fn(&mut Settings, &str)| {
            cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    set(&mut this.draft, &input.read(cx).value());
                    cx.notify();
                }
            })
        };
        SettingsDialog {
            _inputs: [
                bind(cx, &text_width, |s, text| {
                    if let Ok(width @ 1..) = text.parse() {
                        s.viewer.text_width = width;
                    }
                }),
                bind(cx, &binary_width, |s, text| {
                    if let Ok(width @ 1..) = text.parse() {
                        s.viewer.binary_width = width;
                    }
                }),
                bind(cx, &limit, |s, text| {
                    if let Ok(mb @ 1..) = text.parse() {
                        s.editor.limit_mb = mb;
                    }
                }),
                bind(cx, &external, |s, text| {
                    s.editor.external = text.trim().to_owned()
                }),
            ],
            text_width,
            binary_width,
            limit,
            external,
            opacity,
            _opacity: trying,
            commander,
            saved: settings.clone(),
            draft: settings,
            page,
        }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let closing = view.clone();
        dialog
            .title("Settings")
            .width(px(WIDTH))
            .overlay_closable(false)
            .child(view.clone())
            .on_close(move |_, window, cx| closing.update(cx, |this, cx| this.untry(window, cx)))
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
    limit: Entity<InputState>,
    external: Entity<InputState>,
    opacity: Entity<SliderState>,
    _inputs: [Subscription; 4],
    _opacity: Subscription,
}

impl SettingsDialog {
    /// The sidebar as last applied, whatever was tried.
    fn untry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (opacity, blur) = (self.saved.sidebar_opacity.0, self.saved.sidebar_blur.0);
        let _ = self.commander.update(cx, |commander, cx| commander.show_sidebar_backdrop(opacity, blur, window, cx));
    }

    /// The draft's sidebar shown, before it is applied.
    fn try_backdrop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (opacity, blur) = (self.draft.sidebar_opacity.0, self.draft.sidebar_blur.0);
        let _ = self.commander.update(cx, |commander, cx| commander.show_sidebar_backdrop(opacity, blur, window, cx));
        cx.notify();
    }

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
        GroupBox::new().outline().title("Viewer for F3").child(
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .items_start()
                        .gap_8()
                        .child(start)
                        .child(font)
                        .child(
                            v_flex()
                                .gap_2()
                                .child("Show:")
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
                                    // Beside the editor too.
                                    Checkbox::new("viewer-minimap")
                                        .label("Minimap")
                                        .checked(viewer.minimap)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.draft.viewer.minimap = *checked;
                                            cx.notify();
                                        })),
                                ),
                        ),
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

    fn render_editor(&self, cx: &mut Context<Self>) -> GroupBox {
        let theme = cx.theme();
        let external = self.external.clone();
        let browse = Button::new("editor-browse")
            .outline()
            .label("Browse...")
            .on_click(cx.listener(move |_, _, window, cx| {
                let paths = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Choose".into()),
                });
                let external = external.clone();
                cx.spawn_in(window, async move |_, cx| {
                    let Ok(Ok(Some(paths))) = paths.await else {
                        return;
                    };
                    let program = paths[0].to_string_lossy().into_owned();
                    let program = match program.contains(' ') {
                        true => format!("\"{program}\""),
                        false => program,
                    };
                    let _ = cx.update(|window, cx| {
                        external.update(cx, |input, cx| input.set_value(program, window, cx))
                    });
                })
                .detach();
            }));
        GroupBox::new().outline().title("Editor for F4").child(
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child("Built-in editor (F4) opens files up to")
                        .child(div().w(px(70.)).child(Input::new(&self.limit)))
                        .child("MB"),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child("External editor (Ctrl+Shift+F4):")
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&self.external)))
                                .child(browse),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child("%1 is the file, else it goes last; e.g. code --wait %1"),
                        ),
                ),
        )
    }

    fn render_menus(&self, cx: &mut Context<Self>) -> GroupBox {
        let styles = [
            (MenuStyle::Automatic, "Automatic: a ☰ button on GNOME, a menu bar elsewhere"),
            (MenuStyle::Bar, "Menu bar"),
            (MenuStyle::Button, "☰ button in the title bar"),
        ];
        GroupBox::new().outline().title("Menus").child(
            RadioGroup::vertical("menu-style")
                .children(styles.map(|(_, label)| label))
                .selected_index(styles.iter().position(|(style, _)| *style == self.draft.menu))
                .on_click(cx.listener(move |this, at: &usize, _, cx| {
                    this.draft.menu = styles[*at].0;
                    cx.notify();
                })),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> GroupBox {
        let opacity = self.draft.sidebar_opacity.0;
        let shown = match opacity {
            100 => "100%, solid".to_owned(),
            0 => "0%, only the desktop".to_owned(),
            _ => format!("{opacity}%"),
        };
        GroupBox::new().outline().title("Sidebar").child(
            v_flex()
                .gap_2()
                .child("How much it covers the desktop behind it:")
                .child(
                    h_flex()
                        .gap_4()
                        .child(div().flex_1().child(Slider::new(&self.opacity)))
                        .child(div().w(px(150.)).child(shown)),
                )
                .child(
                    Checkbox::new("sidebar-blur")
                        .label("Blur the desktop behind it")
                        .checked(self.draft.sidebar_blur.0)
                        .on_click(cx.listener(|this, checked: &bool, window, cx| {
                            this.draft.sidebar_blur = SidebarBlur(*checked);
                            this.try_backdrop(window, cx);
                        })),
                ),
        )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, title) = PAGES[self.page];
        let content = match title {
            "Layout" if cfg!(any(target_os = "linux", target_os = "freebsd")) || pc_core::theme::sidebar_see_through() => {
                let mut groups = Vec::new();
                // GNOME's apps have their menus behind a ☰ button in the title bar.
                if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                    groups.push(self.render_menus(cx));
                }
                // Where the desktop can show through it.
                if pc_core::theme::sidebar_see_through() {
                    groups.push(self.render_sidebar(cx));
                }
                groups
            }
            "Display" => vec![
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
            ],
            "Edit/View" => vec![self.render_viewer(cx), self.render_editor(cx)],
            "Folder Tabs" => vec![
                GroupBox::new().outline().title("Folder tab headers").child(
                    Checkbox::new("tab-header-always")
                        .label("Show tab header also when there is only one tab")
                        .checked(self.draft.tab_header_always)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.tab_header_always = *checked;
                            cx.notify();
                        })),
                ),
            ],
            _ => Vec::new(),
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
            .child(
                v_flex()
                    .id("settings-page-body")
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .overflow_y_scroll()
                    .children(content),
            )
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.untry(window, cx);
                                window.close_dialog(cx);
                            })),
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
