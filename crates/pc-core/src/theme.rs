//! VS Code color themes; missing keys get VS Code's registry defaults (MIT).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

/// Order matches the [`DEFAULTS`] columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
    HighContrastDark,
    HighContrastLight,
}

impl Kind {
    pub fn is_dark(self) -> bool {
        matches!(self, Kind::Dark | Kind::HighContrastDark)
    }
}

pub struct Theme {
    pub name: String,
    pub kind: Kind,
    /// 0xRRGGBBAA.
    colors: HashMap<String, u32>,
}

impl Theme {
    pub fn color(&self, key: &str) -> Option<u32> {
        self.resolve(key, 0)
    }

    fn resolve(&self, key: &str, depth: u8) -> Option<u32> {
        if let Some(&color) = self.colors.get(key) {
            return Some(color);
        }
        // Cycle guard.
        if depth > 8 {
            return None;
        }
        let (_, defaults) = DEFAULTS.iter().find(|(k, _)| *k == key)?;
        match defaults[self.kind as usize] {
            D::None => None,
            D::Rgb(rgb) => Some(rgb << 8 | 0xFF),
            D::Rgba(rgba) => Some(rgba),
            D::Key(other) => self.resolve(other, depth + 1),
            D::Alpha(other, factor) => self.resolve(other, depth + 1).map(|c| {
                let alpha = ((c & 0xFF) as f32 * factor).round() as u32;
                c & !0xFF | alpha
            }),
        }
    }
}

pub fn default_name(dark: bool) -> &'static str {
    if dark { "Dark Modern" } else { "Light Modern" }
}

pub fn builtin() -> Vec<Theme> {
    BUILTIN
        .iter()
        .map(|&(label, kind, file, text)| {
            let mut theme =
                parse(text, file, Some(kind), &read_builtin).expect("bundled themes parse");
            theme.name = label.to_owned();
            theme
        })
        .collect()
}

fn read_builtin(file: &str) -> io::Result<String> {
    BUILTIN
        .iter()
        .find(|(_, _, name, _)| *name == file)
        .map(|(_, _, _, text)| text.to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, file))
}

pub fn load_dir(dir: &Path) -> (Vec<Theme>, Vec<String>) {
    let mut paths: Vec<_> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return (Vec::new(), vec![format!("{}: {err}", dir.display())]),
    };
    paths.sort();
    let read = |include: &str| fs::read_to_string(dir.join(include));
    let mut themes = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        match fs::read_to_string(&path).and_then(|text| parse(&text, &stem, None, &read)) {
            Ok(theme) => themes.push(theme),
            Err(err) => problems.push(format!("{}: {err}", path.display())),
        }
    }
    (themes, problems)
}

fn parse(
    text: &str,
    fallback_name: &str,
    kind: Option<Kind>,
    read: &dyn Fn(&str) -> io::Result<String>,
) -> io::Result<Theme> {
    let json: Value = serde_json::from_str(&strip_jsonc(text))?;
    let mut colors = HashMap::new();
    merge_colors(&json, read, &mut colors, 0)?;
    let kind = kind
        .or_else(|| json["type"].as_str().and_then(kind_of_type))
        .unwrap_or(match colors.get("editor.background") {
            Some(&bg) if luminance(bg) > 0.5 => Kind::Light,
            _ => Kind::Dark,
        });
    let name = json["name"].as_str().unwrap_or(fallback_name).to_owned();
    Ok(Theme { name, kind, colors })
}

fn merge_colors(
    json: &Value,
    read: &dyn Fn(&str) -> io::Result<String>,
    colors: &mut HashMap<String, u32>,
    depth: u8,
) -> io::Result<()> {
    if let Some(include) = json["include"].as_str() {
        if depth > 8 {
            let err = "includes nest too deep";
            return Err(io::Error::new(io::ErrorKind::InvalidData, err));
        }
        let text = read(include.trim_start_matches("./"))?;
        let included: Value = serde_json::from_str(&strip_jsonc(&text))?;
        merge_colors(&included, read, colors, depth + 1)?;
    }
    if let Some(own) = json["colors"].as_object() {
        // Non-colors (null, typos) fall back to defaults, as in VS Code.
        colors.extend(
            own.iter()
                .filter_map(|(key, value)| Some((key.clone(), parse_color(value.as_str()?)?))),
        );
    }
    Ok(())
}

fn kind_of_type(kind: &str) -> Option<Kind> {
    match kind.to_ascii_lowercase().as_str() {
        "dark" | "vs-dark" => Some(Kind::Dark),
        "light" | "vs" => Some(Kind::Light),
        "hc" | "hcdark" | "hc-black" => Some(Kind::HighContrastDark),
        "hclight" | "hc-light" => Some(Kind::HighContrastLight),
        _ => None,
    }
}

fn luminance(rgba: u32) -> f32 {
    let [r, g, b, _] = rgba.to_be_bytes();
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.
}

fn parse_color(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let full: String = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => hex.to_owned(),
        _ => return None,
    };
    let value = u32::from_str_radix(&full, 16).ok()?;
    Some(if full.len() == 6 {
        value << 8 | 0xFF
    } else {
        value
    })
}

/// VS Code themes are JSONC.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => while chars.next_if(|&c| c != '\n').is_some() {},
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            ']' | '}' => {
                // Drop a trailing comma.
                let kept = out.trim_end().len();
                if out[..kept].ends_with(',') {
                    out.truncate(kept - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

macro_rules! builtin {
    ($label:literal, $kind:ident, $file:literal) => {
        (
            $label,
            Kind::$kind,
            $file,
            include_str!(concat!("../themes/", $file)),
        )
    };
}

const BUILTIN: &[(&str, Kind, &str, &str)] = &[
    builtin!("Dark Modern", Dark, "dark_modern.json"),
    builtin!("Dark+", Dark, "dark_plus.json"),
    builtin!("Dark 2026", Dark, "2026-dark.json"),
    builtin!("Dark (Visual Studio)", Dark, "dark_vs.json"),
    builtin!("Light Modern", Light, "light_modern.json"),
    builtin!("Light+", Light, "light_plus.json"),
    builtin!("Light 2026", Light, "2026-light.json"),
    builtin!("Light (Visual Studio)", Light, "light_vs.json"),
    builtin!("Dark High Contrast", HighContrastDark, "hc_black.json"),
    builtin!("Light High Contrast", HighContrastLight, "hc_light.json"),
    builtin!("Abyss", Dark, "abyss-color-theme.json"),
    builtin!("Kimbie Dark", Dark, "kimbie-dark-color-theme.json"),
    builtin!("Monokai", Dark, "monokai-color-theme.json"),
    builtin!("Monokai Dimmed", Dark, "dimmed-monokai-color-theme.json"),
    builtin!("Quiet Light", Light, "quietlight-color-theme.json"),
    builtin!("Red", Dark, "Red-color-theme.json"),
    builtin!("Solarized Dark", Dark, "solarized-dark-color-theme.json"),
    builtin!("Solarized Light", Light, "solarized-light-color-theme.json"),
    builtin!(
        "Tomorrow Night Blue",
        Dark,
        "tomorrow-night-blue-color-theme.json"
    ),
];

#[derive(Clone, Copy)]
enum D {
    None,
    Rgb(u32),
    Rgba(u32),
    Key(&'static str),
    /// VS Code's `transparent()`.
    Alpha(&'static str, f32),
}

use D::{Alpha, Key, Rgb, Rgba};

// shortcut: only keys the UI uses; add a key's registry default when the UI needs it.
const DEFAULTS: &[(&str, [D; 4])] = &[
    (
        "foreground",
        [Rgb(0xCCCCCC), Rgb(0x616161), Rgb(0xFFFFFF), Rgb(0x292929)],
    ),
    (
        "descriptionForeground",
        [
            Alpha("foreground", 0.7),
            Rgb(0x717171),
            Alpha("foreground", 0.7),
            Alpha("foreground", 0.7),
        ],
    ),
    (
        "editor.background",
        [Rgb(0x1E1E1E), Rgb(0xFFFFFF), Rgb(0x000000), Rgb(0xFFFFFF)],
    ),
    (
        "contrastBorder",
        [D::None, D::None, Rgb(0x6FC3DF), Rgb(0x0F4A85)],
    ),
    (
        "contrastActiveBorder",
        [D::None, D::None, Key("focusBorder"), Key("focusBorder")],
    ),
    (
        "focusBorder",
        [Rgb(0x007FD4), Rgb(0x0090F1), Rgb(0xF38518), Rgb(0x006BBD)],
    ),
    (
        "panel.border",
        [
            Rgba(0x80808059),
            Rgba(0x80808059),
            Key("contrastBorder"),
            Key("contrastBorder"),
        ],
    ),
    (
        "sideBar.background",
        [Rgb(0x252526), Rgb(0xF3F3F3), Rgb(0x000000), Rgb(0xFFFFFF)],
    ),
    (
        "button.background",
        [Rgb(0x0E639C), Rgb(0x007ACC), Rgb(0x000000), Rgb(0x0F4A85)],
    ),
    ("button.foreground", [Rgb(0xFFFFFF); 4]),
    (
        "dropdown.background",
        [Rgb(0x3C3C3C), Rgb(0xFFFFFF), Rgb(0x000000), Rgb(0xFFFFFF)],
    ),
    (
        "dropdown.foreground",
        [
            Rgb(0xF0F0F0),
            Key("foreground"),
            Rgb(0xFFFFFF),
            Key("foreground"),
        ],
    ),
    ("menu.background", [Key("dropdown.background"); 4]),
    ("menu.foreground", [Key("dropdown.foreground"); 4]),
    (
        "menu.selectionBackground",
        [Key("list.activeSelectionBackground"); 4],
    ),
    (
        "menu.selectionForeground",
        [Key("list.activeSelectionForeground"); 4],
    ),
    (
        "list.activeSelectionBackground",
        [Rgb(0x04395E), Rgb(0x0060C0), D::None, Rgba(0x0F4A851A)],
    ),
    (
        "list.activeSelectionForeground",
        [Rgb(0xFFFFFF), Rgb(0xFFFFFF), D::None, D::None],
    ),
    (
        "list.inactiveSelectionBackground",
        [Rgb(0x37373D), Rgb(0xE4E6F1), D::None, Rgba(0x0F4A851A)],
    ),
    (
        "list.focusOutline",
        [
            Key("focusBorder"),
            Key("focusBorder"),
            Key("contrastActiveBorder"),
            Key("contrastActiveBorder"),
        ],
    ),
    (
        "editorGroupHeader.tabsBackground",
        [
            Rgb(0x252526),
            Rgb(0xF3F3F3),
            Key("editor.background"),
            Key("editor.background"),
        ],
    ),
    (
        "sideBarSectionHeader.background",
        [Rgba(0x80808033), Rgba(0x80808033), D::None, D::None],
    ),
    ("tab.activeBackground", [Key("editor.background"); 4]),
    (
        "tab.activeForeground",
        [Rgb(0xFFFFFF), Rgb(0x333333), Rgb(0xFFFFFF), Rgb(0x292929)],
    ),
    (
        "tab.inactiveForeground",
        [
            Alpha("tab.activeForeground", 0.5),
            Alpha("tab.activeForeground", 0.7),
            Rgb(0xFFFFFF),
            Rgb(0x292929),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn named<'a>(themes: &'a [Theme], name: &str) -> &'a Theme {
        themes.iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn reads_the_builtin_themes() {
        let themes = builtin();
        assert_eq!(themes.len(), 19);
        // Dark Modern includes Dark+, which includes Dark (Visual Studio).
        let modern = named(&themes, "Dark Modern");
        assert_eq!(modern.color("editor.background"), Some(0x1F1F1FFF));
        let dark_plus = named(&themes, "Dark+");
        assert_eq!(
            dark_plus.color("editor.background"),
            Some(0x1E1E1EFF),
            "from dark_vs.json"
        );
        let light_plus = named(&themes, "Light+");
        assert_eq!(
            light_plus.color("menu.selectionBackground"),
            Some(0x0060C0FF)
        );
        let hc_light = named(&themes, "Light High Contrast");
        assert_eq!(hc_light.color("editor.background"), Some(0xFFFFFFFF));
        assert_eq!(hc_light.color("panel.border"), Some(0x0F4A85FF));
        let abyss = named(&themes, "Abyss");
        assert_eq!(
            abyss.color("menu.background"),
            abyss.color("dropdown.background")
        );
        assert_ne!(abyss.color("menu.background"), Some(0x3C3C3CFF));
        assert_eq!(dark_plus.color("descriptionForeground"), Some(0xCCCCCCB3));
        assert_eq!(dark_plus.color("contrastBorder"), None);
    }

    #[test]
    fn reads_a_users_theme_with_comments_and_an_include() {
        let dir = std::env::temp_dir().join(format!("pc-theme-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let theme = r##"{
            // "name": "commented out",
            "include": "./base.json",
            "colors": {
                "foreground": "#123", /* short form */
                "button.background": null,
                "url": "http://x//y", "editor.background": "#00000080",
            },
        }"##;
        fs::write(dir.join("Mine.json"), theme).unwrap();
        fs::write(
            dir.join("base.json"),
            r##"{"type": "light", "colors": {"foreground": "#fff", "focusBorder": "#ABCDEF"}}"##,
        )
        .unwrap();
        fs::write(dir.join("broken.json"), "{").unwrap();
        fs::write(dir.join("notes.txt"), "not a theme").unwrap();

        let (themes, problems) = load_dir(&dir);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("broken.json"));
        let mine = named(&themes, "Mine");
        assert_eq!(
            mine.kind,
            Kind::Dark,
            "no type of its own: its background decides"
        );
        assert_eq!(mine.color("foreground"), Some(0x112233FF));
        assert_eq!(
            mine.color("focusBorder"),
            Some(0xABCDEFFF),
            "from the include"
        );
        assert_eq!(mine.color("editor.background"), Some(0x00000080));
        assert_eq!(
            mine.color("button.background"),
            Some(0x0E639CFF),
            "null is the default"
        );
        assert_eq!(named(&themes, "base").kind, Kind::Light);
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#abc"), Some(0xAABBCCFF));
        assert_eq!(parse_color("#abcd"), Some(0xAABBCCDD));
        assert_eq!(parse_color("#A1B2C3"), Some(0xA1B2C3FF));
        assert_eq!(parse_color("#A1B2C3D4"), Some(0xA1B2C3D4));
        assert_eq!(parse_color("#+12345"), None);
        assert_eq!(parse_color("red"), None);
    }
}
