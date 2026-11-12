//! The system's look, by platform: its light or dark colours, its accent
//! and, where the desktop names one, its font. On KDE the user's own
//! colour scheme (kdeglobals); elsewhere the platform's standard colours.

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
        false => Palette {
            font: desktop_font(),
            ..adwaita(dark)
        },
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
fn on_desktop(name: &str) -> bool {
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
    let text = color("Colors:View", "ForegroundNormal")?;
    let font = values.get(&("General", "font")).and_then(|font| font.split(',').next());
    Some(Palette {
        dark: luminance(window) < 0.5,
        window,
        bar,
        view: color("Colors:View", "BackgroundNormal")?,
        text,
        dim: color("Colors:View", "ForegroundInactive").unwrap_or(text),
        // Breeze draws frames as a fifth of the text over the window.
        border: mix(window, text, 0.2),
        selection: color("Colors:Selection", "BackgroundNormal")?,
        selection_text: color("Colors:Selection", "ForegroundNormal")?,
        marked: color("Colors:View", "ForegroundNegative").unwrap_or(0xda4453),
        font: Some(font.filter(|font| !font.is_empty()).unwrap_or(PLASMA_FONT).to_owned()),
    })
}

/// KDE's Breeze Light and Breeze Dark.
fn breeze(dark: bool) -> Palette {
    match dark {
        true => Palette {
            dark,
            window: 0x202326,
            bar: 0x292c30,
            view: 0x141618,
            text: 0xfcfcfc,
            dim: 0xa1a9b1,
            border: mix(0x202326, 0xfcfcfc, 0.2),
            selection: 0x3daee9,
            selection_text: 0xfcfcfc,
            marked: 0xda4453,
            font: Some(PLASMA_FONT.into()),
        },
        false => Palette {
            dark,
            window: 0xeff0f1,
            bar: 0xdee0e2,
            view: 0xffffff,
            text: 0x232629,
            dim: 0x707d8a,
            border: mix(0xeff0f1, 0x232629, 0.2),
            selection: 0x3daee9,
            selection_text: 0xffffff,
            marked: 0xda4453,
            font: Some(PLASMA_FONT.into()),
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
            view: 0x1d1d20,
            text: 0xffffff,
            dim: 0x9a9a9e,
            border: 0x3a3a3f,
            selection: 0x3584e4,
            selection_text: 0xffffff,
            marked: 0xe01b24,
            font: None,
        },
        false => Palette {
            dark,
            window: 0xfafafb,
            bar: 0xebebed,
            view: 0xffffff,
            text: 0x2e2e33,
            dim: 0x6e6e73,
            border: 0xdcdcde,
            selection: 0x3584e4,
            selection_text: 0xffffff,
            marked: 0xe01b24,
            font: None,
        },
    }
}

/// Windows 11's, with its default accent.
fn windows(dark: bool) -> Palette {
    // shortcut: the default accent; read the user's from the registry with the Windows build.
    match dark {
        true => Palette {
            dark,
            window: 0x202020,
            bar: 0x202020,
            view: 0x1c1c1c,
            text: 0xffffff,
            dim: 0xc5c5c5,
            border: 0x3d3d3d,
            selection: 0x0078d4,
            selection_text: 0xffffff,
            marked: 0xff99a4,
            font: Some("Segoe UI".into()),
        },
        false => Palette {
            dark,
            window: 0xf3f3f3,
            bar: 0xf3f3f3,
            view: 0xffffff,
            text: 0x1a1a1a,
            dim: 0x5d5d5d,
            border: 0xe5e5e5,
            selection: 0x0078d4,
            selection_text: 0xffffff,
            marked: 0xc42b1c,
            font: Some("Segoe UI".into()),
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
            view: 0x1e1e1e,
            text: 0xe6e6e6,
            dim: 0x989898,
            border: 0x3d3d3d,
            selection: 0x0058d0,
            selection_text: 0xffffff,
            marked: 0xff453a,
            font: None,
        },
        false => Palette {
            dark,
            window: 0xececec,
            bar: 0xf5f5f5,
            view: 0xffffff,
            text: 0x262626,
            dim: 0x808080,
            border: 0xd8d8d8,
            selection: 0x0063e1,
            selection_text: 0xffffff,
            marked: 0xff3b30,
            font: None,
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
