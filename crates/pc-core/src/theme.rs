//! The system's look, by platform: its light or dark colours, its accent
//! and, where the desktop names one, its font. On KDE the user's own
//! colour scheme (kdeglobals); elsewhere the platform's standard colours.

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
    /// Menus, bars and dialogs.
    pub window: u32,
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
    /// Column titles; the inactive panel's path.
    pub header: u32,
    pub header_border: u32,
    /// The sorted column's title; the active panel's path.
    pub sorted_header: u32,
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
            sorted_header: mix(header, accent, share(0.18, 0.12)),
            selected_row: mix(background, accent, share(0.30, 0.22)),
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
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    match desktop.split(':').any(|name| name == "KDE") {
        true => {
            let globals = dirs::config_dir().map(|dir| dir.join("kdeglobals"));
            let text = globals.and_then(|file| std::fs::read_to_string(file).ok());
            kde_or_breeze(text.as_deref(), dark)
        }
        false => adwaita(dark),
    }
}

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
    let text = color("Colors:View", "ForegroundNormal")?;
    let font = values.get(&("General", "font")).and_then(|font| font.split(',').next());
    Some(Palette {
        dark: luminance(window) < 0.5,
        window,
        view: color("Colors:View", "BackgroundNormal")?,
        text,
        dim: color("Colors:View", "ForegroundInactive").unwrap_or(text),
        // Breeze draws frames as a fifth of the text over the window.
        border: mix(window, text, 0.2),
        selection: color("Colors:Selection", "BackgroundNormal")?,
        selection_text: color("Colors:Selection", "ForegroundNormal")?,
        marked: color("Colors:View", "ForegroundNegative").unwrap_or(0xda4453),
        font: font.filter(|font| !font.is_empty()).map(str::to_owned),
    })
}

/// KDE's Breeze Light and Breeze Dark.
fn breeze(dark: bool) -> Palette {
    match dark {
        true => Palette {
            dark,
            window: 0x202326,
            view: 0x141618,
            text: 0xfcfcfc,
            dim: 0xa1a9b1,
            border: mix(0x202326, 0xfcfcfc, 0.2),
            selection: 0x3daee9,
            selection_text: 0xfcfcfc,
            marked: 0xda4453,
            font: None,
        },
        false => Palette {
            dark,
            window: 0xeff0f1,
            view: 0xffffff,
            text: 0x232629,
            dim: 0x707d8a,
            border: mix(0xeff0f1, 0x232629, 0.2),
            selection: 0x3daee9,
            selection_text: 0xffffff,
            marked: 0xda4453,
            font: None,
        },
    }
}

/// GNOME's libadwaita, for desktops without a scheme of their own.
fn adwaita(dark: bool) -> Palette {
    match dark {
        true => Palette {
            dark,
            window: 0x222226,
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
    // shortcut: the default blue accent; read the user's when a Mac can test it.
    match dark {
        true => Palette {
            dark,
            window: 0x323232,
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
        assert_eq!(list.sorted_header, mix(0x27282a, 0x3daee9, 0.18));
    }
}
