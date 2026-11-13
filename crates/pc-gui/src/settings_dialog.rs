use std::collections::HashSet;

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
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{Icon, IndexPath};
use pc_core::dates::{self, Clock};
use pc_core::settings::{AddonUpdates, FontChoice, MenuStyle, PAGES, Settings, SidebarBlur, SidebarOpacity, StartFont, StartMode};
use pc_core::theme::Appearance;

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

/// The main window's font's sample, as Total Commander's.
const SAMPLE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz äöüéèêàñç 0123456789";
const SIZES: [&str; 15] = ["7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "18", "20", "22", "24", "28"];
/// Each page's icon in the rail, as `PAGES`.
const ICONS: [Lucide; 7] = [
    Lucide::PanelsTopLeft,
    Lucide::Eye,
    Lucide::Type,
    Lucide::CalendarClock,
    Lucide::Languages,
    Lucide::FilePen,
    Lucide::Puzzle,
];

type Fonts = SelectState<SearchableVec<SharedString>>;
type Sizes = SelectState<Vec<SharedString>>;

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
    let pattern = input(settings.date.custom.clone().unwrap_or_else(|| "yyyy-MM-dd".into()), false, cx);
    let opacity = cx.new(|_| {
        let value = f32::from(settings.sidebar_opacity.0);
        SliderState::new().min(0.).max(100.).step(5.).default_value(value)
    });
    let mut families: Vec<SharedString> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    let installed: HashSet<SharedString> = families.iter().cloned().collect();
    let main = Picker::new(families.clone(), &settings.font, system_font(&settings, cx), window, cx);
    // The fixed-width ones known: all of them until measured once (`fixed_width`).
    let fixed_families = FIXED_WIDTH.get().cloned().unwrap_or_else(|| families.clone());
    let fixed = Picker::new(fixed_families, &settings.fixed_font, system_fixed_font(&settings, cx), window, cx);
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
        let pickers = [main.listen(Which::Main, cx), fixed.listen(Which::Fixed, cx)];
        // The list of fixed-width fonts, measured once in the background.
        let measuring = match FIXED_WIDTH.get() {
            Some(_) => Task::ready(()),
            None => {
                let text_system = cx.text_system().clone();
                cx.spawn_in(window, async move |this, cx| {
                    let fixed = cx
                        .background_executor()
                        .spawn(async move { families.into_iter().filter(|family| fixed_width(&text_system, family)).collect::<Vec<_>>() })
                        .await;
                    let fixed = FIXED_WIDTH.get_or_init(|| fixed).clone();
                    let _ = this.update_in(cx, |this, window, cx| this.fixed.set_families(fixed, &this.draft.fixed_font, window, cx));
                })
            }
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
                // Typed into while "Custom" is chosen.
                bind(cx, &pattern, |s, text| {
                    if s.date.custom.is_some() {
                        s.date.custom = Some(text.to_owned());
                    }
                }),
            ],
            _pickers: pickers.into_iter().flatten().collect(),
            _measuring: measuring,
            text_width,
            binary_width,
            limit,
            external,
            pattern,
            main,
            fixed,
            installed,
            opacity,
            _opacity: trying,
            _addons: cx.observe(&crate::addons::entity(cx), |_, _, cx| cx.notify()),
            commander,
            saved: settings.clone(),
            draft: settings,
            page,
        }
    });
    if PAGES[page] == "Add-ons" {
        crate::addons::shown(cx);
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        let closing = view.clone();
        dialog
            .title("Settings")
            .width(px(WIDTH))
            .overlay_closable(false)
            .child(view.clone())
            .footer(buttons(&view, cx))
            .on_close(move |_, window, cx| closing.update(cx, |this, cx| this.untry(window, cx)))
    });
}

/// The desktop's UI font and size in points; GPUI Kit's where it names none.
fn system_font(settings: &Settings, cx: &App) -> FontChoice {
    let theme = cx.theme();
    let palette = pc_core::theme::system(theme.mode.is_dark());
    // The theme has the system's font while no other is chosen.
    let (family, pixels) = match &settings.font {
        None => (theme.font_family.to_string(), f32::from(theme.font_size) * 0.875),
        Some(_) => (palette.font.clone().unwrap_or_else(|| ".SystemUIFont".into()), 14.),
    };
    FontChoice {
        family: palette.font.unwrap_or(family),
        size: pc_core::theme::font_points(palette.font_size.unwrap_or(pixels)).round(),
    }
}

/// The desktop's fixed-width font; GPUI Kit's where it names none.
fn system_fixed_font(settings: &Settings, cx: &App) -> FontChoice {
    let theme = cx.theme();
    // The theme has the system's while no other is chosen.
    let current = (theme.mono_font_family.to_string(), pc_core::theme::font_points(f32::from(theme.mono_font_size)));
    let (family, size) = match &settings.fixed_font {
        None => current,
        Some(_) => pc_core::theme::desktop_fixed_font().unwrap_or(current),
    };
    FontChoice { family, size: size.round() }
}

/// The fixed-width families among `families`, measured once.
static FIXED_WIDTH: std::sync::OnceLock<Vec<SharedString>> = std::sync::OnceLock::new();

/// A font whose i, W and m are as wide.
fn fixed_width(text_system: &TextSystem, family: &str) -> bool {
    let id = text_system.resolve_font(&font(family.to_owned()));
    let width = |c| text_system.advance(id, px(16.), c).ok().map(|size| size.width);
    match (width('i'), width('W'), width('m')) {
        (Some(i), Some(w), Some(m)) => (i - w).abs() < px(0.05) && (i - m).abs() < px(0.05),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Which {
    Main,
    Fixed,
}

/// A font's family and size, chosen or the system's.
struct Picker {
    fonts: Entity<Fonts>,
    sizes: Entity<Sizes>,
    system: FontChoice,
}

impl Picker {
    fn new(families: Vec<SharedString>, chosen: &Option<FontChoice>, system: FontChoice, window: &mut Window, cx: &mut App) -> Self {
        let shown = chosen.clone().unwrap_or_else(|| system.clone());
        let at = families.iter().position(|name| name.as_ref() == shown.family);
        let fonts = cx.new(|cx| SelectState::new(SearchableVec::new(families), at.map(IndexPath::new), window, cx).searchable(true));
        let size: SharedString = format!("{}", shown.size.round()).into();
        let list: Vec<SharedString> = SIZES.iter().map(|&size| size.into()).collect();
        let at = list.iter().position(|item| *item == size);
        let sizes = cx.new(|cx| SelectState::new(list, at.map(IndexPath::new), window, cx));
        Picker { fonts, sizes, system }
    }

    /// Choosing either sets the draft's font of `which`.
    fn listen(&self, which: Which, cx: &mut Context<SettingsDialog>) -> [Subscription; 2] {
        let family = cx.subscribe(&self.fonts, move |this: &mut SettingsDialog, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(family)) = event else { return };
            let size = this.chosen(which).map_or(this.picker(which).system.size, |font| font.size);
            *this.chosen_mut(which) = Some(FontChoice { family: family.to_string(), size });
            cx.notify();
        });
        let size = cx.subscribe(&self.sizes, move |this: &mut SettingsDialog, _, event: &SelectEvent<Vec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(size)) = event else { return };
            let Ok(size) = size.parse() else { return };
            let family = this.chosen(which).map_or_else(|| this.picker(which).system.family.clone(), |font| font.family.clone());
            *this.chosen_mut(which) = Some(FontChoice { family, size });
            cx.notify();
        });
        [family, size]
    }

    /// Another list of families, the shown one kept.
    fn set_families(&self, families: Vec<SharedString>, chosen: &Option<FontChoice>, window: &mut Window, cx: &mut App) {
        let shown: SharedString = chosen.as_ref().map_or_else(|| self.system.family.clone(), |font| font.family.clone()).into();
        self.fonts.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(families), window, cx);
            select.set_selected_value(&shown, window, cx);
        });
    }

    /// Back to the system's font.
    fn reset(&self, window: &mut Window, cx: &mut App) {
        let family: SharedString = self.system.family.clone().into();
        let size: SharedString = format!("{}", self.system.size).into();
        self.fonts.update(cx, |select, cx| select.set_selected_value(&family, window, cx));
        self.sizes.update(cx, |select, cx| select.set_selected_value(&size, window, cx));
    }
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
    pattern: Entity<InputState>,
    main: Picker,
    fixed: Picker,
    /// The fonts this computer has, for the samples.
    installed: HashSet<SharedString>,
    opacity: Entity<SliderState>,
    _inputs: [Subscription; 5],
    _pickers: Vec<Subscription>,
    _measuring: Task<()>,
    _opacity: Subscription,
    _addons: Subscription,
}

impl SettingsDialog {
    fn chosen(&self, which: Which) -> Option<&FontChoice> {
        match which {
            Which::Main => self.draft.font.as_ref(),
            Which::Fixed => self.draft.fixed_font.as_ref(),
        }
    }

    fn chosen_mut(&mut self, which: Which) -> &mut Option<FontChoice> {
        match which {
            Which::Main => &mut self.draft.font,
            Which::Fixed => &mut self.draft.fixed_font,
        }
    }

    fn picker(&self, which: Which) -> &Picker {
        match which {
            Which::Main => &self.main,
            Which::Fixed => &self.fixed,
        }
    }

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

    /// The pages, a row each with its icon: a rail down the left.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // macOS: a sidebar, its selection a rounded row of the accent.
        let mac = cx.try_global::<MacControls>().copied();
        // Windows 11: a navigation pane, unframed; the selection a faint row with the accent's pill.
        let fluent = fluent();
        let subtle = if theme.mode.is_dark() { hsla(0., 0., 1., 0.06) } else { hsla(0., 0., 0., 0.05) };
        let (pill, rail) = (theme.primary, theme.muted);
        v_flex()
            .id("settings-rail")
            .w(px(172.))
            .flex_none()
            .h_full()
            .gap(px(2.))
            .p(px(6.))
            .map(|el| match mac {
                Some(mac) => el.bg(mac.group).rounded(px(10.)),
                None if fluent => el.p_0(),
                None => el.bg(rail).rounded(px(8.)),
            })
            .children(PAGES.iter().enumerate().map(|(ix, &label)| {
                let selected = ix == self.page;
                h_flex()
                    .id(("settings-page", ix))
                    .relative()
                    .flex_none()
                    .h(px(if mac.is_some() { 28. } else { 34. }))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .child(Icon::new(ICONS[ix].clone()).size_4())
                    .child(label)
                    .when(!selected, |el| el.hover(|el| el.bg(subtle)))
                    .when(selected, |el| match mac {
                        Some(mac) => el.bg(mac.accent).text_color(mac.on_accent),
                        None if fluent => el
                            .bg(subtle)
                            .child(div().absolute().left_0().top(px(9.)).w(px(3.)).h(px(16.)).rounded(px(2.)).bg(pill)),
                        None => el.bg(theme.primary).text_color(theme.primary_foreground),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = ix;
                        if label == "Add-ons" {
                            crate::addons::shown(cx);
                        }
                        cx.notify();
                    }))
            }))
    }

    /// Light or dark, where the desktop does not decide it alone.
    fn render_appearance(&self, cx: &mut Context<Self>) -> Option<GroupBox> {
        if pc_core::theme::desktop_dark().is_some() {
            return None;
        }
        let looks = [(Appearance::System, "As the system"), (Appearance::Light, "Light"), (Appearance::Dark, "Dark")];
        Some(GroupBox::new().outline().title("Appearance").child(
            RadioGroup::horizontal("appearance")
                .children(looks.map(|(_, label)| label))
                .selected_index(looks.iter().position(|(look, _)| *look == self.draft.appearance))
                .on_click(cx.listener(move |this, at: &usize, _, cx| {
                    this.draft.appearance = looks[*at].0;
                    cx.notify();
                })),
        ))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> GroupBox {
        GroupBox::new().outline().title("Folder tabs").child(
            Checkbox::new("tab-header-always")
                .label("Show the tab header also when there is only one tab")
                .checked(self.draft.tab_header_always)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.draft.tab_header_always = *checked;
                    cx.notify();
                })),
        )
    }

    fn render_files(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let folders = ["Always by name", "Like files (also by time)"];
        vec![
            GroupBox::new().outline().title("Files shown").child(
                h_flex()
                    .gap_6()
                    .child(
                        Checkbox::new("show-hidden")
                            .label("Hidden files")
                            .checked(self.draft.show_hidden)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.show_hidden = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        // Linux and macOS have no system attribute.
                        Checkbox::new("show-system")
                            .label("System files")
                            .checked(self.draft.show_system)
                            .disabled(!cfg!(windows))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.show_system = *checked;
                                cx.notify();
                            })),
                    ),
            ),
            GroupBox::new().outline().title("Sorting folders").child(
                RadioGroup::horizontal("folders-sort")
                    .children(folders)
                    .selected_index(Some(usize::from(self.draft.folders_like_files)))
                    .on_click(cx.listener(|this, at: &usize, _, cx| {
                        this.draft.folders_like_files = *at == 1;
                        cx.notify();
                    })),
            ),
        ]
    }

    /// A font of `which`: chosen, or the desktop's; with its sample.
    fn render_font(&self, which: Which, cx: &mut Context<Self>) -> GroupBox {
        let picker = self.picker(which);
        let chosen = self.chosen(which);
        let font = chosen.cloned().unwrap_or_else(|| picker.system.clone());
        // Never a family this computer lacks: GPUI panics on it.
        let family = match self.installed.contains(font.family.as_str()) {
            true => font.family.clone(),
            false => picker.system.family.clone(),
        };
        let theme = cx.theme();
        let dim = theme.muted_foreground;
        let system = &picker.system;
        let state = match chosen {
            None => "The system's font".to_owned(),
            Some(_) => format!("Chosen; the system's is {}, {}", system.family, system.size),
        };
        let (title, note, id) = match which {
            Which::Main => ("Main window font", None, "font-system"),
            Which::Fixed => ("Viewer", Some("Fixed-width fonts only; also the editor's and file comparison's."), "fixed-system"),
        };
        GroupBox::new().outline().title(title).child(
            v_flex()
                .gap_3()
                .children(note.map(|note| div().text_color(dim).child(note)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(Select::new(&picker.fonts).menu_max_h(px(320.))))
                        .child(div().w(px(84.)).flex_none().child(Select::new(&picker.sizes)))
                        .child(
                            Button::new(id)
                                .outline()
                                .label("As the system")
                                .disabled(chosen.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    *this.chosen_mut(which) = None;
                                    this.picker(which).reset(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().text_color(dim).child(state))
                .child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .font_family(family)
                        .text_size(px(pc_core::theme::font_pixels(font.size)))
                        .child(SAMPLE),
                ),
        )
    }

    /// Dates and times as the system shows them, or a pattern and clock of one's own.
    fn render_date(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let now = std::time::SystemTime::now();
        let dim = cx.theme().muted_foreground;
        let custom = self.draft.date.custom.is_some();
        let pattern = self.pattern.clone();
        let clocks = [Clock::System, Clock::H24, Clock::H12];
        vec![
            GroupBox::new().outline().title("Date").child(
                v_flex()
                    .gap_2()
                    .child(
                        RadioGroup::vertical("date-kind")
                            .children([format!("As the system: {}", dates::system_date(now)), "Custom:".to_owned()])
                            .selected_index(Some(usize::from(custom)))
                            .on_click(cx.listener(move |this, at: &usize, _, cx| {
                                this.draft.date.custom = (*at == 1).then(|| pattern.read(cx).value().to_string());
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .pl_6()
                            .gap_1()
                            .child(div().w(px(240.)).child(Input::new(&self.pattern).disabled(!custom)))
                            .child(div().text_color(dim).child("yyyy year, MM month (MMM its name), dd day (ddd the weekday)")),
                    ),
            ),
            GroupBox::new().outline().title("Time").child(
                RadioGroup::horizontal("clock")
                    .children(["As the system", "24 hours (14:05)", "12 hours (2:05 PM)"])
                    .selected_index(clocks.iter().position(|clock| *clock == self.draft.date.clock))
                    .on_click(cx.listener(move |this, at: &usize, _, cx| {
                        this.draft.date.clock = clocks[*at];
                        cx.notify();
                    })),
            ),
            GroupBox::new().outline().title("Shown as").child(
                h_flex()
                    .gap_2()
                    .child(dates::preview(&self.draft.date, now))
                    .child(div().text_color(dim).child("in the file lists; dialogs add the seconds")),
            ),
        ]
    }

    fn render_language(&self) -> GroupBox {
        GroupBox::new().outline().title("Language").child(
            RadioGroup::vertical("language").children(["English"]).selected_index(Some(0)),
        )
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
                Presence::Elsewhere(_) => "comes with Power Commander".to_owned(),
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
            GroupBox::new().outline().title("Offers and updates").child(
                v_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("addon-never-offer")
                            .label("Never offer add-ons when a file needs one")
                            .checked(self.draft.never_offer_addons)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.never_offer_addons = *checked;
                                cx.notify();
                            })),
                    )
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
        let title = PAGES[self.page];
        let content = match title {
            "Layout" => {
                let mut groups: Vec<GroupBox> = self.render_appearance(cx).into_iter().collect();
                // GNOME's apps have their menus behind a ☰ button in the title bar.
                if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                    groups.push(self.render_menus(cx));
                }
                // Where the desktop can show through it.
                if pc_core::theme::sidebar_see_through() {
                    groups.push(self.render_sidebar(cx));
                }
                groups.push(self.render_tabs(cx));
                groups
            }
            "Display" => self.render_files(cx),
            "Font" => vec![self.render_font(Which::Main, cx), self.render_font(Which::Fixed, cx)],
            "Date" => self.render_date(cx),
            "Language" => vec![self.render_language()],
            "Edit/View" => vec![self.render_viewer(cx), self.render_editor(cx)],
            "Add-ons" => self.render_addons(cx),
            _ => Vec::new(),
        };
        // The page's name as a pane's title, bold, unboxed; macOS's smaller.
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
                        _ => el.text_lg().font_semibold(),
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
        h_flex()
            .w_full()
            .h(px(BODY_HEIGHT))
            .items_start()
            .gap_3()
            .text_sm()
            .child(self.render_rail(cx))
            .child(self.render_page(cx))
    }
}

/// OK, Cancel and Apply, in the dialog's footer.
fn buttons(view: &Entity<SettingsDialog>, cx: &App) -> impl IntoElement {
    let changed = view.read(cx).draft != view.read(cx).saved;
    let (ok, cancel, apply) = (view.clone(), view.clone(), view.clone());
    h_flex()
        .gap_2()
        .justify_end()
        .child(
            Button::new("settings-ok")
                .primary()
                .label("OK")
                .min_w(px(80.))
                .on_click(move |_, window, cx| {
                    ok.update(cx, |this, cx| this.apply(window, cx));
                    window.close_dialog(cx);
                }),
        )
        .child(
            Button::new("settings-cancel")
                .outline()
                .label("Cancel")
                .min_w(px(80.))
                .on_click(move |_, window, cx| {
                    cancel.update(cx, |this, cx| this.untry(window, cx));
                    window.close_dialog(cx);
                }),
        )
        .child(
            Button::new("settings-apply")
                .outline()
                .label("Apply")
                .min_w(px(80.))
                .disabled(!changed)
                .on_click(move |_, window, cx| apply.update(cx, |this, cx| this.apply(window, cx))),
        )
}
