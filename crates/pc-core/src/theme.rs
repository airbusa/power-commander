//! The system's look, by platform: its light or dark colours, its accent
//! and, where the desktop names one, its font. On KDE the user's own
//! colour scheme (kdeglobals); on Xfce, MATE and Cinnamon their GTK 3
//! theme's; elsewhere the platform's standard colours.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Settings' choice: the system's light or dark, or always one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// A look's colours, as 0xRRGGBB.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Dialogs.
    pub window: u32,
    /// The menu bar and the button bar: KDE's header colour.
    pub bar: u32,
    /// Buttons' and combo boxes' fill: KDE's button colour.
    pub button: u32,
    /// The file lists, the viewer's and editor's pages.
    pub view: u32,
    pub text: u32,
    /// Less important text.
    pub dim: u32,
    pub border: u32,
    /// The cursor, the active panel's path, a menu's highlighted item.
    pub selection: u32,
    pub selection_text: u32,
    /// Marked files' names.
    pub marked: u32,
    /// The UI font's family, where the desktop names one.
    pub font: Option<String>,
    /// The UI font's size in pixels, where the desktop names one.
    pub font_size: Option<f32>,
}

/// The file lists' colours, from the look's background, text and accent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListColors {
    pub row: u32,
    /// Every other row, for the eye to follow a line.
    pub alternate_row: u32,
    /// Column titles and the button bar.
    pub header: u32,
    pub header_border: u32,
    /// The paths above the panels.
    pub active_path: u32,
    pub inactive_path: u32,
    /// The cursor's row.
    pub selected_row: u32,
}

impl Palette {
    pub fn list(&self) -> ListColors {
        let share = |dark: f32, light: f32| if self.dark { dark } else { light };
        let (background, text, accent) = (self.view, self.text, self.selection);
        let header = mix(background, text, share(0.08, 0.06));
        ListColors {
            row: background,
            alternate_row: mix(background, text, share(0.05, 0.03)),
            header,
            header_border: mix(background, text, share(0.20, 0.16)),
            active_path: mix(background, accent, share(0.14, 0.10)),
            inactive_path: mix(background, text, share(0.04, 0.02)),
            selected_row: mix(background, accent, share(0.30, 0.22)),
        }
    }
}

/// Menus as KDE's Breeze draws them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuColors {
    /// The window's colour, a little toward the view's.
    pub popup: u32,
    pub frame: u32,
    /// The item under the pointer or the keys: the accent, faint, and framed.
    pub hover: u32,
    pub hover_frame: u32,
}

impl Palette {
    pub fn menu(&self) -> MenuColors {
        let popup = mix(self.window, self.view, 0.3);
        MenuColors {
            popup,
            frame: mix(self.window, self.text, 0.2),
            hover: mix(popup, self.selection, 0.3),
            hover_frame: mix(self.selection, self.selection_text, 0.15),
        }
    }
}

/// The look of this platform and desktop.
pub fn system(dark: bool) -> Palette {
    if cfg!(windows) {
        return windows(dark);
    }
    if cfg!(target_os = "macos") {
        return mac(dark);
    }
    match on_kde() {
        true => {
            let globals = dirs::config_dir().map(|dir| dir.join("kdeglobals"));
            let text = globals.and_then(|file| std::fs::read_to_string(file).ok());
            kde_or_breeze(text.as_deref(), dark)
        }
        // GTK's apps write in the desktop's font.
        false => {
            let palette = match gtk_theme() {
                Some(theme) if theme.dark == dark => theme,
                Some(theme) => Palette { selection: theme.selection, selection_text: theme.selection_text, ..adwaita(dark) },
                None => Palette { selection: gnome_accent(interface_setting("accent-color").as_deref()), ..adwaita(dark) },
            };
            Palette { font: desktop_font(), font_size: None, ..palette }
        }
    }
}

/// Whether the desktop's own look is dark, where its apps go by their
/// theme's colours rather than a dark mode: Xfce's, MATE's, Cinnamon's.
pub fn desktop_dark() -> Option<bool> {
    gtk_theme().map(|theme| theme.dark)
}

/// The GTK 3 theme's colours on Xfce, MATE and Cinnamon.
fn gtk_theme() -> Option<Palette> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    return crate::toolkit_style::gtk_colors().and_then(gtk);
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    None
}

/// A GTK 3 theme's colours, as `pc-gtkstyle` reads them.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn gtk(theme: &crate::toolkit_style::Drawn) -> Option<Palette> {
    let color = |name: &str| theme.colors.get(name).copied();
    let window = color("theme_bg_color")?;
    let view = color("theme_base_color").unwrap_or(window);
    let text = color("theme_text_color").or(color("theme_fg_color"))?;
    Some(Palette {
        dark: theme.dark,
        window,
        bar: window,
        button: color("button").unwrap_or(window),
        view,
        text,
        dim: mix(view, text, 0.6),
        border: color("borders").unwrap_or(mix(window, text, 0.2)),
        selection: color("theme_selected_bg_color")?,
        selection_text: color("theme_selected_fg_color").unwrap_or(0xffffff),
        marked: color("error_color").unwrap_or(0xe01b24),
        font: None,
        font_size: None,
    })
}

/// GNOME's accent (GNOME 47's accent-color) as libadwaita draws it; its blue
/// before, and elsewhere.
fn gnome_accent(name: Option<&str>) -> u32 {
    match name.unwrap_or_default() {
        "teal" => 0x2190a4,
        "green" => 0x3a944a,
        "yellow" => 0xc88800,
        "orange" => 0xed5b00,
        "red" => 0xe62d42,
        "pink" => 0xd56199,
        "purple" => 0x9141ac,
        "slate" => 0x6f8396,
        _ => 0x3584e4,
    }
}

/// One of the desktop's interface settings: Cinnamon's, MATE's, else GNOME's.
pub(crate) fn interface_setting(key: &str) -> Option<String> {
    let schema = match () {
        _ if on_desktop("X-Cinnamon") => "org.cinnamon.desktop.interface",
        _ if on_desktop("MATE") => "org.mate.interface",
        _ => "org.gnome.desktop.interface",
    };
    gsetting(schema, key)
}

/// A GSettings key's value.
pub(crate) fn gsetting(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("gsettings").args(["get", schema, key]).output().ok()?;
    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().trim_matches('\'').to_owned()).filter(|value| !value.is_empty())
}

/// The family of a Pango font name: "Noto Sans,  10", "Ubuntu Sans 11".
fn font_family(pango: &str) -> String {
    let family = pango.split(',').next().unwrap_or(pango).trim();
    let sized = family.rsplit_once(' ').filter(|(_, size)| size.parse::<f32>().is_ok());
    sized.map_or(family, |(name, _)| name.trim_end()).to_owned()
}

/// GNOME, Ubuntu's own desktop among them (`ubuntu:GNOME`).
pub fn on_gnome() -> bool {
    on_desktop("GNOME")
}

/// The interface font of GNOME, Cinnamon, MATE or Xfce.
fn desktop_font() -> Option<String> {
    static FONT: OnceLock<Option<String>> = OnceLock::new();
    let find = || {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if on_desktop("XFCE") {
            return crate::desktop::xfce_setting("FontName").map(|name| font_family(&name));
        }
        let gtk = on_gnome() || on_desktop("X-Cinnamon") || on_desktop("MATE");
        gtk.then(|| interface_setting("font-name").map(|name| font_family(&name))).flatten()
    };
    FONT.get_or_init(|| find().map(real_family)).clone()
}

/// The family a fontconfig alias ("Sans", Cinnamon's) stands for, which GPUI needs.
fn real_family(family: String) -> String {
    let alias = ["sans", "sans-serif", "sans serif", "serif", "monospace", "mono"].contains(&family.to_lowercase().as_str());
    let matched = alias.then(|| Command::new("fc-match").args(["-f", "%{family[0]}", &family]).output().ok()).flatten();
    matched.and_then(|out| String::from_utf8(out.stdout).ok()).filter(|f| !f.is_empty()).unwrap_or(family)
}

fn on_kde() -> bool {
    on_desktop("KDE")
}

/// One of XDG_CURRENT_DESKTOP's names.
pub(crate) fn on_desktop(name: &str) -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').any(|desktop| desktop == name)
}

/// How much of the window colour the sidebar paints over the system's own
/// material behind it, macOS's blur, Windows 11's Mica, KWin's blur on
/// Wayland, from its opacity in percent. None: solid, at 100% and on other
/// Linux desktops, which do not blur what is behind.
pub fn sidebar_tint(opacity: u8) -> Option<f32> {
    (sidebar_see_through() && opacity < 100).then_some(f32::from(opacity) / 100.)
}

/// Whether the desktop can show through the sidebar: macOS, Windows, KWin on Wayland.
pub fn sidebar_see_through() -> bool {
    // shortcut: KWin with its blur effect off shows the desktop sharp; ask its capability if that matters.
    let kwin = on_kde() && std::env::var_os("WAYLAND_DISPLAY").is_some();
    cfg!(any(target_os = "macos", windows)) || kwin
}

/// The desktop style whose controls the app draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlStyle {
    /// KDE's Breeze, whatever its widget style, Breeze being the closest we draw.
    Breeze,
    /// Windows 11's (WinUI's Fluent controls).
    Fluent,
    /// GNOME's (libadwaita's).
    Adwaita,
}

/// This desktop's control style; none where GPUI Kit's own look stands.
pub fn control_style() -> Option<ControlStyle> {
    match () {
        _ if cfg!(windows) => Some(ControlStyle::Fluent),
        _ if cfg!(any(target_os = "linux", target_os = "freebsd")) && on_kde() => Some(ControlStyle::Breeze),
        _ if cfg!(any(target_os = "linux", target_os = "freebsd")) && on_desktop("GNOME") => Some(ControlStyle::Adwaita),
        _ => None,
    }
}

/// The controls' colours as the desktop's style draws them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlColors {
    /// Check boxes and radio buttons: unchecked, their frame and fill; checked; the mark on them.
    pub check_frame: u32,
    pub check_fill: u32,
    pub checked_frame: u32,
    pub checked_fill: u32,
    pub mark: u32,
    /// Sliders and progress bars: the groove, what fills it; a slider's handle and its middle.
    pub groove: u32,
    pub fill: u32,
    pub handle: u32,
    pub handle_middle: u32,
    /// Windows': the handle's frame.
    pub handle_frame: Option<u32>,
    /// Push buttons' fill and frame, normal, hovered and pressed; the default
    /// button's (a dialog's OK); their text.
    pub button: [[u32; 2]; 3],
    pub default_button: [[u32; 2]; 3],
    pub button_text: u32,
    pub default_button_text: u32,
    /// Windows': the bottom edge of a button's frame, of the default button's.
    pub button_edge: Option<[u32; 2]>,
    /// Breeze's tabs: the selected one, the others, the selected one's line.
    pub tabs: Option<[u32; 3]>,
    /// Windows': a text field's bottom line, the checked fill when focused.
    pub field_line: Option<u32>,
    /// Text fields' fill, where the style has its own.
    pub field_fill: Option<u32>,
}

impl Palette {
    pub fn controls(&self, style: ControlStyle) -> ControlColors {
        match style {
            ControlStyle::Breeze => self.breeze(),
            ControlStyle::Fluent => self.fluent(windows_accents()),
            ControlStyle::Adwaita => self.adwaita(),
        }
    }

    /// As KDE's Breeze style mixes them from the scheme (`breezehelper.cpp`, `breezestyle.cpp`).
    fn breeze(&self) -> ControlColors {
        // Frames: a fifth of the text over the button.
        let frame = mix(self.button, self.text, 0.2);
        let checked = mix(self.button, self.selection, 0.3);
        let default_fill = mix(self.button, self.selection, 0.2);
        ControlColors {
            check_frame: self.border,
            check_fill: self.button,
            checked_frame: self.selection,
            checked_fill: checked,
            mark: self.text,
            groove: mix(self.window, self.text, 0.14),
            fill: mix(self.window, self.selection, 0.7),
            handle: frame,
            handle_middle: self.button,
            handle_frame: None,
            button: [[self.button, frame], [self.button, self.selection], [checked, self.selection]],
            default_button: [
                [default_fill, mix(self.selection, frame, 0.5)],
                [default_fill, self.selection],
                [checked, self.selection],
            ],
            button_text: self.text,
            default_button_text: self.text,
            button_edge: None,
            tabs: Some([mix(self.window, self.selection, 0.2), self.view, self.selection]),
            field_line: None,
            field_fill: None,
        }
    }

    /// Windows 11's (WinUI's theme resources), its translucent colours laid
    /// over the window; `accents` the user's, light to dark.
    fn fluent(&self, accents: [u32; 7]) -> ControlColors {
        let over = |under: u32, argb: u32| mix(under, argb & 0xffffff, (argb >> 24) as f32 / 255.);
        let on = |argb: u32| over(self.window, argb);
        // AccentFillColorDefault: lighter on dark, darker on light; hovered 90%, pressed 80%.
        let accent = if self.dark { accents[1] } else { accents[4] };
        let accent_at = |share: f32| mix(self.window, accent, share);
        let (stroke, strong, text, on_accent) = match self.dark {
            true => (0x12ffffff, 0x8bffffff, 0xffffff, 0x000000),
            false => (0x0f000000, 0x72000000, on(0xe4000000), 0xffffff),
        };
        let fills: [u32; 3] = match self.dark {
            true => [0x0fffffff, 0x15ffffff, 0x08ffffff],
            false => [0xb3ffffff, 0x80f9f9f9, 0x4df9f9f9],
        };
        let accent_frame = over(accent, 0x14ffffff);
        ControlColors {
            check_frame: on(strong),
            check_fill: on(if self.dark { 0x19000000 } else { 0x06000000 }),
            checked_frame: accent,
            checked_fill: accent,
            mark: on_accent,
            groove: on(strong),
            fill: accent,
            handle: if self.dark { 0x454545 } else { 0xffffff },
            handle_middle: accent,
            // ControlElevationBorder: its darker edge on light, which a white handle needs.
            handle_frame: Some(on(if self.dark { stroke } else { 0x29000000 })),
            button: fills.map(|fill| [on(fill), on(stroke)]),
            default_button: [[accent, accent_frame], [accent_at(0.9), accent_frame], [accent_at(0.8), accent_at(0.8)]],
            button_text: text,
            default_button_text: on_accent,
            // The bottom of the frame: darker on light; dark has its lighter edge on top.
            button_edge: Some(match self.dark {
                true => [on(stroke), over(accent, 0x23000000)],
                false => [on(0x29000000), over(accent, 0x66000000)],
            }),
            tabs: None,
            field_line: Some(on(strong)),
            field_fill: Some(on(fills[0])),
        }
    }

    /// GNOME's (libadwaita's stylesheet): fills of the text over the window,
    /// unframed; the selection is the accent.
    fn adwaita(&self) -> ControlColors {
        let ink = |share: f32| mix(self.window, self.text, share);
        let accent = self.selection;
        let white = 0xffffff;
        // The slider's knob: white 80% over the view; its rim, a faint shade.
        let knob = mix(self.view, white, 0.8);
        let fills = [ink(0.1), ink(0.15), ink(0.3)];
        ControlColors {
            check_frame: ink(0.15),
            check_fill: self.window,
            checked_frame: accent,
            checked_fill: accent,
            mark: white,
            groove: ink(0.15),
            fill: accent,
            handle: knob,
            handle_middle: knob,
            handle_frame: Some(mix(self.window, 0x000006, if self.dark { 0.35 } else { 0.1 })),
            button: fills.map(|fill| [fill, fill]),
            default_button: [accent, mix(accent, white, 0.1), mix(accent, 0x000006, 0.2)].map(|fill| [fill, fill]),
            button_text: self.text,
            default_button_text: white,
            button_edge: None,
            tabs: None,
            field_line: None,
            field_fill: Some(fills[0]),
        }
    }
}

/// Windows' accent and its shades, light to dark (Light3 … Dark3), as the
/// Explorer keeps them; Windows' default blue elsewhere, or unread.
fn windows_accents() -> [u32; 7] {
    #[cfg(windows)]
    if let Some(accents) = registry_binary(ACCENT_KEY, "AccentPalette").and_then(|bytes| accent_palette(&bytes)) {
        return accents;
    }
    [0x99ebff, 0x4cc2ff, 0x0091f8, 0x0078d4, 0x0067c0, 0x003e92, 0x001a68]
}

#[cfg(windows)]
const ACCENT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";

/// The Explorer's AccentPalette: R, G, B and an unused byte for each shade.
#[cfg(any(windows, test))]
fn accent_palette(bytes: &[u8]) -> Option<[u32; 7]> {
    (bytes.len() >= 28).then(|| std::array::from_fn(|i| u32::from_be_bytes([0, bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2]])))
}

/// A binary value of the user's registry.
#[cfg(windows)]
fn registry_binary(key: &str, value: &str) -> Option<Vec<u8>> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW};
    use windows::core::HSTRING;
    let (key, value) = (HSTRING::from(key), HSTRING::from(value));
    let mut data = vec![0u8; 64];
    let mut size = data.len() as u32;
    let flags = RRF_RT_REG_BINARY;
    let read = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &value, flags, None, Some(data.as_mut_ptr().cast()), Some(&mut size)) };
    read.is_ok().then(|| {
        data.truncate(size as usize);
        data
    })
}

/// GTK's header bar as the GTK theme draws it (measured with GTK 3, which
/// GNOME Terminal is); libadwaita's, as GNOME's own apps draw it, for a theme
/// not known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderBar {
    pub height: f32,
    /// Its top corners'.
    pub radius: f32,
    /// Its colour in the window focused, then not ("backdrop"), light.
    pub light: [u32; 2],
    pub dark: [u32; 2],
}

impl HeaderBar {
    pub fn color(&self, dark: bool, focused: bool) -> u32 {
        let [active, backdrop] = if dark { self.dark } else { self.light };
        if focused { active } else { backdrop }
    }
}

/// The GTK theme's header bar; none off Linux.
pub fn header_bar() -> Option<HeaderBar> {
    static BAR: OnceLock<Option<HeaderBar>> = OnceLock::new();
    let find = || {
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return None;
        }
        let theme = interface_setting("gtk-theme").unwrap_or_default();
        // Breeze's colours are the KDE scheme's, which KDE writes for GTK.
        let colors = dirs::config_dir().and_then(|dir| std::fs::read_to_string(dir.join("gtk-3.0/colors.css")).ok());
        Some(header_bar_of(&theme, colors.as_deref()))
    };
    *BAR.get_or_init(find)
}

/// "Yaru-dark" draws as "Yaru" does.
fn header_bar_of(theme: &str, kde_colors: Option<&str>) -> HeaderBar {
    let bar = |height, radius, light, dark| HeaderBar { height, radius, light, dark };
    let libadwaita = bar(47., 12., [0xffffff, 0xfafafa], [0x303030, 0x242424]);
    match theme.split('-').next().unwrap_or_default() {
        "Breeze" => {
            let color = |name: &str| kde_colors.and_then(|css| css_color(css, name));
            let kde = color("theme_titlebar_background_breeze").zip(color("theme_titlebar_background_backdrop_breeze"));
            let [active, backdrop] = kde.map_or([0xdee0e2, 0xeff0f1], |(active, backdrop)| [active, backdrop]);
            bar(48., 6., [active, backdrop], kde.map_or(libadwaita.dark, |_| [active, backdrop]))
        }
        "Adwaita" => bar(47., 8., [0xdddad6, 0xf6f5f4], [0x282828, 0x353535]),
        "Yaru" => bar(47., 12., [0xebebeb, 0xfafafa], [0x222222, 0x2c2c2c]),
        _ => libadwaita,
    }
}

/// `@define-color name #rrggbb;` of GTK's CSS.
fn css_color(css: &str, name: &str) -> Option<u32> {
    let line = css.lines().find_map(|line| line.trim().strip_prefix("@define-color ")?.strip_prefix(name)?.strip_prefix(' '))?;
    u32::from_str_radix(line.trim().trim_end_matches(';').strip_prefix('#')?, 16).ok()
}

/// The window buttons GTK draws, from the desktop's icon theme: minimize,
/// maximize, restore and close, as SVG files. None without GNOME's settings.
pub fn window_buttons() -> Option<&'static [PathBuf; 4]> {
    static BUTTONS: OnceLock<Option<[PathBuf; 4]>> = OnceLock::new();
    let find = || {
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return None;
        }
        let theme = interface_setting("icon-theme")?;
        let bases = icon_bases();
        // GTK's own fallback is Adwaita.
        let themes = [theme.as_str(), "Adwaita", "hicolor"];
        let icon = |name: &str| themes.iter().find_map(|theme| find_icon(&bases, theme, name, 5));
        let [minimize, maximize, restore, close] =
            ["minimize", "maximize", "restore", "close"].map(|button| icon(&format!("window-{button}-symbolic")));
        Some([minimize?, maximize?, restore?, close?])
    };
    BUTTONS.get_or_init(find).as_ref()
}

/// Where icon themes live, the user's first.
fn icon_bases() -> Vec<PathBuf> {
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|dirs| !dirs.is_empty());
    let system = system.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let user = [dirs::data_dir().map(|dir| dir.join("icons")), dirs::home_dir().map(|home| home.join(".icons"))];
    user.into_iter().flatten().chain(system.split(':').map(|dir| Path::new(dir).join("icons"))).collect()
}

/// An SVG icon of a theme, or of the themes it inherits from.
fn find_icon(bases: &[PathBuf], theme: &str, name: &str, depth: u8) -> Option<PathBuf> {
    let index = bases.iter().find_map(|base| std::fs::read_to_string(base.join(theme).join("index.theme")).ok())?;
    let value = |key: &str| index.lines().find_map(|line| line.strip_prefix(key)).unwrap_or_default();
    let file = name.to_owned() + ".svg";
    let dirs = value("Directories=").split(',').map(str::trim);
    let found = dirs.flat_map(|dir| bases.iter().map(move |base| base.join(theme).join(dir)));
    if let Some(found) = found.map(|dir| dir.join(&file)).find(|path| path.is_file()) {
        return Some(found);
    }
    let mut parents = value("Inherits=").split(',').map(str::trim).filter(|parent| !parent.is_empty() && depth > 0);
    parents.find_map(|parent| find_icon(bases, parent, name, depth - 1))
}

/// Plasma's font where kdeglobals names none.
const PLASMA_FONT: &str = "Noto Sans";

/// The user's KDE colour scheme when it is as dark as asked; else Breeze,
/// with the scheme's accent and font.
fn kde_or_breeze(kdeglobals: Option<&str>, dark: bool) -> Palette {
    let breeze = breeze(dark);
    let Some(scheme) = kdeglobals.and_then(kde) else {
        return breeze;
    };
    match scheme.dark == dark {
        true => scheme,
        false => Palette {
            selection: scheme.selection,
            selection_text: scheme.selection_text,
            font: scheme.font,
            font_size: scheme.font_size,
            ..breeze
        },
    }
}

/// A KDE colour scheme, as kdeglobals keeps it.
fn kde(kdeglobals: &str) -> Option<Palette> {
    let mut section = "";
    let mut values = std::collections::HashMap::new();
    for line in kdeglobals.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[') {
            section = name.trim_end_matches(']');
        } else if let Some((key, value)) = line.split_once('=') {
            values.insert((section, key), value);
        }
    }
    let color = |section: &str, key: &str| -> Option<u32> {
        let value = values.get(&(section, key))?;
        let rgb: Vec<u32> = value.split(',').map(|c| c.trim().parse().ok()).collect::<Option<_>>()?;
        match rgb[..] {
            [r, g, b, ..] => Some(r << 16 | g << 8 | b),
            _ => None,
        }
    };
    let window = color("Colors:Window", "BackgroundNormal")?;
    let bar = color("Colors:Header", "BackgroundNormal").unwrap_or(window);
    let button = color("Colors:Button", "BackgroundNormal").unwrap_or(window);
    let text = color("Colors:View", "ForegroundNormal")?;
    let font_entry = values.get(&("General", "font"));
    let font = font_entry.and_then(|font| font.split(',').next());
    Some(Palette {
        dark: luminance(window) < 0.5,
        window,
        bar,
        button,
        view: color("Colors:View", "BackgroundNormal")?,
        text,
        dim: color("Colors:View", "ForegroundInactive").unwrap_or(text),
        // Breeze draws frames as a fifth of the text over the window.
        border: mix(window, text, 0.2),
        selection: color("Colors:Selection", "BackgroundNormal")?,
        selection_text: color("Colors:Selection", "ForegroundNormal")?,
        marked: color("Colors:View", "ForegroundNegative").unwrap_or(0xda4453),
        font: Some(font.filter(|font| !font.is_empty()).unwrap_or(PLASMA_FONT).to_owned()),
        font_size: font_entry.and_then(|font| qt_font_pixels(font)),
    })
}

/// A Qt font entry's size in pixels: "Noto Sans,10,-1,5,…" is 10 points, at
/// 96 pixels an inch; "…,-1,13,…" 13 pixels.
fn qt_font_pixels(entry: &str) -> Option<f32> {
    let mut fields = entry.split(',').skip(1).map(|field| field.trim().parse::<f32>().ok());
    match (fields.next()??, fields.next().flatten()) {
        (points, _) if points > 0. => Some(points * 96. / 72.),
        (_, Some(pixels)) if pixels > 0. => Some(pixels),
        _ => None,
    }
}

/// KDE's Breeze Light and Breeze Dark.
fn breeze(dark: bool) -> Palette {
    match dark {
        true => Palette {
            dark,
            window: 0x202326,
            bar: 0x292c30,
            button: 0x292c30,
            view: 0x141618,
            text: 0xfcfcfc,
            dim: 0xa1a9b1,
            border: mix(0x202326, 0xfcfcfc, 0.2),
            selection: 0x3daee9,
            selection_text: 0xfcfcfc,
            marked: 0xda4453,
            font: Some(PLASMA_FONT.into()),
            font_size: None,
        },
        false => Palette {
            dark,
            window: 0xeff0f1,
            bar: 0xdee0e2,
            button: 0xfcfcfc,
            view: 0xffffff,
            text: 0x232629,
            dim: 0x707d8a,
            border: mix(0xeff0f1, 0x232629, 0.2),
            selection: 0x3daee9,
            selection_text: 0xffffff,
            marked: 0xda4453,
            font: Some(PLASMA_FONT.into()),
            font_size: None,
        },
    }
}

/// GNOME's libadwaita, for desktops without a scheme of their own.
fn adwaita(dark: bool) -> Palette {
    match dark {
        true => Palette {
            dark,
            window: 0x222226,
            bar: 0x2e2e32,
            button: 0x2e2e32,
            view: 0x1d1d20,
            text: 0xffffff,
            dim: 0x9a9a9e,
            border: 0x3a3a3f,
            selection: 0x3584e4,
            selection_text: 0xffffff,
            marked: 0xe01b24,
            font: None,
            font_size: None,
        },
        false => Palette {
            dark,
            window: 0xfafafb,
            bar: 0xebebed,
            button: 0xebebed,
            view: 0xffffff,
            text: 0x2e2e33,
            dim: 0x6e6e73,
            border: 0xdcdcde,
            selection: 0x3584e4,
            selection_text: 0xffffff,
            marked: 0xe01b24,
            font: None,
            font_size: None,
        },
    }
}

/// Windows 11's, with its default accent.
fn windows(dark: bool) -> Palette {
    let accent = windows_accents()[3];
    match dark {
        true => Palette {
            dark,
            window: 0x202020,
            bar: 0x202020,
            button: 0x202020,
            view: 0x1c1c1c,
            text: 0xffffff,
            dim: 0xc5c5c5,
            border: 0x3d3d3d,
            selection: accent,
            selection_text: 0xffffff,
            marked: 0xff99a4,
            font: Some("Segoe UI".into()),
            font_size: None,
        },
        false => Palette {
            dark,
            window: 0xf3f3f3,
            bar: 0xf3f3f3,
            button: 0xf3f3f3,
            view: 0xffffff,
            text: 0x1a1a1a,
            dim: 0x5d5d5d,
            border: 0xe5e5e5,
            selection: accent,
            selection_text: 0xffffff,
            marked: 0xc42b1c,
            font: Some("Segoe UI".into()),
            font_size: None,
        },
    }
}

/// macOS's, with its default accent.
fn mac(dark: bool) -> Palette {
    // The default blue accent; pc-gui puts the user's in.
    match dark {
        true => Palette {
            dark,
            window: 0x323232,
            bar: 0x2a2a2a,
            button: 0x2a2a2a,
            view: 0x1e1e1e,
            text: 0xe6e6e6,
            dim: 0x989898,
            border: 0x3d3d3d,
            selection: 0x0058d0,
            selection_text: 0xffffff,
            marked: 0xff453a,
            font: None,
            // AppKit's system font size.
            font_size: Some(13.),
        },
        false => Palette {
            dark,
            window: 0xececec,
            bar: 0xf5f5f5,
            button: 0xf5f5f5,
            view: 0xffffff,
            text: 0x262626,
            dim: 0x808080,
            border: 0xd8d8d8,
            selection: 0x0063e1,
            selection_text: 0xffffff,
            marked: 0xff3b30,
            font: None,
            font_size: Some(13.),
        },
    }
}

/// `share` of the way from `from` to `to`.
fn mix(from: u32, to: u32, share: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = ((from >> shift & 0xff) as f32, (to >> shift & 0xff) as f32);
        ((a + (b - a) * share.clamp(0., 1.)).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| (rgb >> shift & 0xff) as f32 / 255.;
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_11_controls() {
        // Windows' default blue, as its registry has it.
        let bytes = [
            0x99, 0xeb, 0xff, 0, 0x4c, 0xc2, 0xff, 0, 0x00, 0x91, 0xf8, 0, 0x00, 0x78, 0xd4, 0, 0x00, 0x67, 0xc0, 0, 0x00, 0x3e,
            0x92, 0, 0x00, 0x1a, 0x68, 0, 0xf7, 0x63, 0x0c, 0,
        ];
        let accents = accent_palette(&bytes).unwrap();
        assert_eq!((accents[1], accents[3], accents[4]), (0x4cc2ff, 0x0078d4, 0x0067c0));
        assert_eq!(accent_palette(&bytes[..20]), None);
        // The lighter shade on dark, the darker on light; marks in black, white.
        let dark = windows(true).fluent(accents);
        assert_eq!((dark.checked_fill, dark.mark, dark.default_button_text), (0x4cc2ff, 0x000000, 0x000000));
        let light = windows(false).fluent(accents);
        assert_eq!((light.checked_fill, light.mark), (0x0067c0, 0xffffff));
        // ControlFillColorDefault, 6% white, over dark's #202020.
        assert_eq!(dark.button[0][0], 0x2d2d2d);
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn gtk_theme_colors() {
        let rgb = [("theme_bg_color", 0xcecece), ("theme_base_color", 0xfcfcfc), ("theme_text_color", 0x212121), ("theme_selected_bg_color", 0x398ee7)];
        let colors: std::collections::HashMap<String, u32> = rgb.into_iter().map(|(name, rgb)| (name.to_owned(), rgb)).collect();
        let mut theme = crate::toolkit_style::Drawn { style: "Greybird".into(), dark: false, images: Default::default(), colors };
        let palette = gtk(&theme).unwrap();
        assert_eq!((palette.window, palette.view, palette.text, palette.selection), (0xcecece, 0xfcfcfc, 0x212121, 0x398ee7));
        assert_eq!((palette.button, palette.selection_text, palette.dark), (0xcecece, 0xffffff, false));
        theme.colors.remove("theme_bg_color");
        assert!(gtk(&theme).is_none());
    }

    #[test]
    fn gnome_controls() {
        assert_eq!((gnome_accent(Some("teal")), gnome_accent(None)), (0x2190a4, 0x3584e4));
        let light = adwaita(false).adwaita();
        // Buttons: the text at 10% over the window, unframed; the suggested one in the accent.
        assert_eq!(light.button[0], [mix(0xfafafb, 0x2e2e33, 0.1); 2]);
        assert_eq!((light.default_button[0][0], light.default_button_text), (0x3584e4, 0xffffff));
        assert_eq!(light.field_fill, Some(light.button[0][0]));
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_accent_from_the_registry() {
        let bytes = registry_binary(ACCENT_KEY, "AccentPalette").expect("AccentPalette");
        assert!(accent_palette(&bytes).is_some());
    }

    #[test]
    fn qt_font_sizes() {
        assert_eq!(qt_font_pixels("Segoe UI,10,-1,5,50,0,0,0,0,0"), Some(10. * 96. / 72.));
        assert_eq!(qt_font_pixels("Noto Sans,-1,13,5,400,0"), Some(13.));
        assert_eq!(qt_font_pixels("Noto Sans"), None);
    }

    #[test]
    fn header_bars_by_theme() {
        let kde = "@define-color theme_titlebar_background_backdrop_breeze #31363b;\n@define-color theme_titlebar_background_breeze #2a2e32;\n";
        let breeze = header_bar_of("Breeze-Dark", Some(kde));
        assert_eq!((breeze.height, breeze.radius, breeze.light), (48., 6., [0x2a2e32, 0x31363b]));
        assert_eq!(header_bar_of("Breeze", None).color(false, false), 0xeff0f1);
        assert_eq!(header_bar_of("Yaru-prussiangreen-dark", None).color(true, true), 0x222222);
        assert_eq!(header_bar_of("Adwaita", None).radius, 8.);
        assert_eq!(header_bar_of("", None).color(false, true), 0xffffff);
    }

    const BREEZE_DARK: &str = "[Colors:Header]\nBackgroundNormal=41,44,48\n\
        [Colors:Selection]\nBackgroundNormal=61,174,233\nForegroundNormal=252,252,252\n\
        [Colors:View]\nBackgroundNormal=20,22,24\nForegroundInactive=161,169,177\n\
        ForegroundNegative=218,68,83\nForegroundNormal=252,252,252\n\
        [Colors:Window]\nBackgroundNormal=32,35,38\n\
        [General]\nfont=Segoe UI,10,-1,5,50,0,0,0,0,0\n";

    #[test]
    fn reads_kde_schemes() {
        let scheme = kde(BREEZE_DARK).unwrap();
        assert!(scheme.dark);
        assert_eq!((scheme.window, scheme.view), (0x202326, 0x141618));
        assert_eq!((scheme.selection, scheme.marked), (0x3daee9, 0xda4453));
        assert_eq!(scheme.font.as_deref(), Some("Segoe UI"));
        assert_eq!(kde_or_breeze(Some(BREEZE_DARK), true), scheme);
        // Light asked of a dark scheme: Breeze Light, the scheme's accent and font.
        let light = kde_or_breeze(Some(BREEZE_DARK), false);
        assert!(!light.dark && light.view == 0xffffff);
        assert_eq!(light.font.as_deref(), Some("Segoe UI"));
        assert_eq!(kde_or_breeze(None, false), breeze(false));
        assert!(kde("[General]\nfont=x\n").is_none(), "no colours");
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        let list = scheme.list();
        assert_eq!((list.header, list.selected_row), (0x27282a, 0x204457));
        assert_eq!(list.active_path, mix(0x141618, 0x3daee9, 0.14));
        assert_eq!(scheme.bar, 0x292c30);
        // As a screenshot of Krusader's menu has them.
        let menu = scheme.menu();
        assert_eq!((menu.popup, menu.frame), (0x1c1f22, 0x4c4e51));
        assert_eq!((menu.hover, menu.hover_frame), (0x264a5e, 0x5abaec));
    }

    #[test]
    fn finds_icons_in_inherited_themes() {
        let root = std::env::temp_dir().join(format!("pc-icons-{}", std::process::id()));
        let theme = |name: &str, index: &str| {
            std::fs::create_dir_all(root.join(name).join("actions/symbolic")).unwrap();
            std::fs::write(root.join(name).join("index.theme"), index).unwrap();
        };
        theme("child", "[Icon Theme]\nInherits=parent\nDirectories=actions/16,actions/symbolic\n");
        theme("parent", "[Icon Theme]\nDirectories=actions/symbolic\n");
        let close = root.join("parent/actions/symbolic/window-close-symbolic.svg");
        std::fs::write(&close, "<svg/>").unwrap();
        let bases = [root.join("missing"), root.clone()];
        assert_eq!(find_icon(&bases, "child", "window-close-symbolic", 5), Some(close));
        assert_eq!(find_icon(&bases, "child", "window-close-symbolic", 0), None, "no inheriting");
        assert_eq!(find_icon(&bases, "child", "go-up", 5), None);
        assert_eq!(font_family("Noto Sans,  10"), "Noto Sans");
        assert_eq!(font_family("Ubuntu Sans 11"), "Ubuntu Sans");
        assert_eq!(font_family("Cantarell"), "Cantarell");
        std::fs::remove_dir_all(root).unwrap();
    }
}
