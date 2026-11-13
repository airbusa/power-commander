use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Disableable, MacControls, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::addons::{Addon, Builtin, Presence};
use pc_core::settings::{AddonUpdates, MenuStyle, PAGES, Settings, SidebarBlur, SidebarOpacity, StartFont, StartMode};

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

fn fluent() -> bool {
    pc_core::theme::control_style() == Some(pc_core::theme::ControlStyle::Fluent)
}

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
            _addons: cx.observe(&crate::addons::entity(cx), |_, _, cx| cx.notify()),
            commander,
            saved: settings.clone(),
            draft: settings,
            page,
        }
    });
    if PAGES[page].1 == "Add-ons" {
        crate::addons::shown(cx);
    }
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
    _addons: Subscription,
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
        // macOS: a sidebar, its selection a rounded row of the accent.
        let mac = cx.try_global::<MacControls>().copied();
        // Windows 11: a navigation pane, unframed; the selection a faint row with the accent's pill.
        let fluent = fluent();
        let subtle = if theme.mode.is_dark() { hsla(0., 0., 1., 0.06) } else { hsla(0., 0., 0., 0.04) };
        let pill = theme.primary;
        v_flex()
            .id("settings-pages")
            .w(px(190.))
            .h_full()
            .p_1()
            .map(|el| match mac {
                Some(mac) => el.p(px(6.)).bg(mac.group).rounded(px(10.)),
                None if fluent => el.p_0().gap(px(2.)),
                None => el.border_1().border_color(theme.border),
            })
            .overflow_y_scroll()
            .children(PAGES.iter().enumerate().map(|(ix, &(depth, label))| {
                let selected = ix == self.page;
                div()
                    .id(("settings-page", ix))
                    .pl(px(6. + 14. * depth as f32))
                    .pr_2()
                    .when(mac.is_some(), |el| el.flex_none().h(px(22.)).flex().items_center().rounded(px(5.)))
                    .when(fluent, |el| {
                        el.relative()
                            .flex_none()
                            .h(px(32.))
                            .pl(px(14. + 16. * depth as f32))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .hover(|el| el.bg(subtle))
                            .when(selected, |el| {
                                el.bg(subtle).child(div().absolute().left_0().top(px(8.)).w(px(3.)).h(px(16.)).rounded(px(2.)).bg(pill))
                            })
                    })
                    .when(selected && !fluent, |el| match mac {
                        Some(mac) => el.bg(mac.accent).text_color(mac.on_accent),
                        None => el.bg(theme.accent).text_color(theme.accent_foreground),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = ix;
                        if label == "Add-ons" {
                            crate::addons::shown(cx);
                        }
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

    /// Other projects' programs the app uses: the add-ons, each with its
    /// files, versions and download; then those that come with it; then the
    /// look-ups for updates.
    fn render_addons(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let state = crate::addons::entity(cx).read(cx);
        let dim = cx.theme().muted_foreground;
        let row = |label: &'static str, value: AnyElement| {
            h_flex()
                .gap_2()
                .child(div().w(px(80.)).flex_none().text_color(dim).child(label))
                .child(div().flex_1().min_w_0().truncate().child(value))
        };
        let website = |id: &'static str, (shown, url): (&'static str, &'static str)| {
            let link = Button::new(id).link().label(shown).on_click(move |_, _, cx| cx.open_url(url));
            h_flex().child(link).into_any_element()
        };
        let mut groups = vec![GroupBox::new().child(div().text_color(dim).child(
            "Power Commander uses these programs of other projects for some of what it does. The add-ons \
             are downloaded apart from it, when you choose; 7-Zip and PDFium come with it.",
        ))];
        groups.extend(Addon::ALL.into_iter().map(|addon| {
            let offer = state.manifest.as_ref().and_then(|manifest| manifest.offer(addon));
            let offered = offer.map(|(release, download)| (release.version.clone(), download.size));
            let presence = state.presence.get(&addon).cloned().unwrap_or(Presence::Missing);
            let installed = match &presence {
                Presence::Ours(version) => version.clone(),
                Presence::Elsewhere(_) => "the system's".to_owned(),
                Presence::Missing => "none".to_owned(),
            };
            let online = match &offered {
                Some((version, size)) => format!("{version} ({})", crate::addons::megabytes(*size)),
                None if state.checking => "looking...".to_owned(),
                None if state.check_error.is_some() => "unknown".to_owned(),
                None if state.manifest.is_some() => "none for this system".to_owned(),
                None => "not looked up yet".to_owned(),
            };
            let progress = state.progress.get(&addon).copied();
            // Something to get: none here, or another version online.
            let newer = offered.as_ref().is_some_and(|(version, _)| match &presence {
                Presence::Ours(installed) => installed != version,
                _ => true,
            });
            let buttons = h_flex()
                .flex_none()
                .gap_2()
                .child(
                    Button::new(("addon-get", addon as usize))
                        .label("Download")
                        .disabled(!newer || progress.is_some())
                        .on_click(move |_, _, cx| crate::addons::install(addon, cx)),
                )
                .when(matches!(presence, Presence::Ours(_)), |el| {
                    el.child(
                        Button::new(("addon-remove", addon as usize))
                            .label("Remove")
                            .disabled(progress.is_some())
                            .on_click(move |_, _, cx| crate::addons::remove(addon, cx)),
                    )
                });
            let site = ["addon-site-0", "addon-site-1", "addon-site-2"][addon as usize];
            GroupBox::new().outline().title(addon.name()).child(
                v_flex()
                    .gap_2()
                    .child(div().text_color(dim).child(addon.purpose()))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_end()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(row("Website", website(site, addon.website())))
                                    .child(row("File", addon.files().join(", ").into_any_element()))
                                    .child(row("Installed", installed.into_any_element()))
                                    .child(row("Online", online.into_any_element())),
                            )
                            .child(buttons),
                    )
                    .when_some(progress, |el, (done, of)| {
                        el.child(format!(
                            "Downloading: {} of {}",
                            crate::addons::megabytes(done),
                            crate::addons::megabytes(of)
                        ))
                    })
                    .when_some(state.failed.get(&addon), |el, why| {
                        el.child(div().text_color(cx.theme().danger).child(why.clone()))
                    }),
            )
        }));
        groups.extend(Builtin::ALL.into_iter().map(|builtin| {
            let site = ["builtin-site-0", "builtin-site-1"][builtin as usize];
            GroupBox::new().outline().title(builtin.name()).child(
                v_flex()
                    .gap_2()
                    .child(div().text_color(dim).child(builtin.purpose()))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(row("Website", website(site, builtin.website())))
                            .child(row("File", builtin.file().into_any_element()))
                            .child(row("Version", format!("{}, comes with Power Commander", builtin.version()).into_any_element())),
                    ),
            )
        }));
        let looked_up = match (state.checking, &state.check_error, &state.manifest) {
            (true, _, _) => "Looking for updates...".to_owned(),
            (false, Some(err), _) => format!("Could not look for updates: {err}"),
            (false, None, Some(_)) => "The list of add-ons is up to date.".to_owned(),
            (false, None, None) => String::new(),
        };
        let checking = state.checking;
        groups.push(
            GroupBox::new().outline().title("Updates").child(
                v_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("addon-updates")
                            .label("Look for add-on updates at start and daily, and offer them")
                            .checked(self.draft.addon_updates.0)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.addon_updates = AddonUpdates(*checked);
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                Button::new("addon-check")
                                    .label("Look now")
                                    .disabled(checking)
                                    .on_click(|_, _, cx| crate::addons::check(false, cx)),
                            )
                            .child(div().text_color(dim).child(looked_up)),
                    ),
            ),
        );
        groups
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
            "Add-ons" => self.render_addons(cx),
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
        // macOS and Windows 11: the page's name as a pane's title, bold, unboxed.
        let mac = cx.has_global::<MacControls>();
        let fluent = fluent();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_3()
            .child(
                div()
                    .py_1()
                    .map(|el| match () {
                        _ if mac => el.text_base().font_semibold(),
                        _ if fluent => el.text_xl().font_semibold(),
                        _ => el.text_center().bg(theme.muted).border_1().border_color(theme.border),
                    })
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
