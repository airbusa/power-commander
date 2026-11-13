use std::fs;
use std::io;
use std::path::PathBuf;

use serde::de::DeserializeOwned;

use crate::Filter;
use serde::{Deserialize, Serialize};

/// How much the sidebar covers the desktop behind it, in percent, where it
/// can be see-through (macOS's blur, Windows' acrylic): 100 is solid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SidebarOpacity(pub u8);

/// About ForkLift's: the desktop shows through, dimmed.
impl Default for SidebarOpacity {
    fn default() -> Self {
        SidebarOpacity(80)
    }
}

/// Whether the desktop shows through the sidebar blurred (else sharp), as Konsole's "Blur background".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SidebarBlur(pub bool);

impl Default for SidebarBlur {
    fn default() -> Self {
        SidebarBlur(true)
    }
}

/// Look for add-ons' updates at start and daily.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonUpdates(pub bool);

impl Default for AddonUpdates {
    fn default() -> Self {
        AddonUpdates(true)
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: crate::theme::Appearance,
    pub menu: MenuStyle,
    /// Show the tab header also when there is only one tab.
    pub tab_header_always: bool,
    pub show_hidden: bool,
    pub show_system: bool,
    pub sidebar_opacity: SidebarOpacity,
    pub sidebar_blur: SidebarBlur,
    pub viewer: ViewerSettings,
    pub editor: EditorSettings,
    /// Files' own programs for Enter, F3 and F4.
    pub associations: Vec<crate::associations::Association>,
    pub keys: crate::keymap::KeySettings,
    pub addon_updates: AddonUpdates,
    /// Folders sorted as files are, also by time; else always by name.
    pub folders_like_files: bool,
    pub date: crate::dates::DateSettings,
    /// The main window's font; the desktop's when none.
    pub font: Option<FontChoice>,
}

/// A font family and its size in points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontChoice {
    pub family: String,
    pub size: f32,
}

/// Where the menus are: a bar under the title, or behind a ☰ button in the
/// window's own title bar, as GNOME's apps have them. Linux only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MenuStyle {
    /// The ☰ button on GNOME, the bar elsewhere.
    #[default]
    Automatic,
    Bar,
    Button,
}

impl MenuStyle {
    /// Whether the menus go behind the ☰ button.
    pub fn button(self) -> bool {
        let linux = cfg!(any(target_os = "linux", target_os = "freebsd"));
        linux && (self == MenuStyle::Button || self == MenuStyle::Automatic && crate::theme::on_gnome())
    }
}

/// F4 edits in the built-in editor, Ctrl+Shift+F4 in another program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// The largest file the built-in editor opens, in MB: it takes about
    /// eight times as much memory.
    pub limit_mb: u64,
    /// Ctrl+Shift+F4's command line: `%1` is the file, else the file goes last.
    pub external: String,
    /// The View menu's, kept for the next file.
    pub wrap: bool,
    pub line_numbers: bool,
    pub whitespace: bool,
    pub indent_guides: bool,
    pub folding: bool,
    /// Where the file shows none of its own (`text_edit::detect_indent`).
    pub tab_size: usize,
    pub hard_tabs: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        let external = match () {
            _ if cfg!(windows) => "notepad.exe",
            _ if cfg!(target_os = "macos") => "open -t",
            _ => "xdg-open",
        };
        Self {
            limit_mb: 32,
            external: external.into(),
            wrap: false,
            line_numbers: true,
            whitespace: false,
            indent_guides: true,
            folding: true,
            tab_size: 4,
            hard_tabs: false,
        }
    }
}

/// Lister's: what F3 shows a file as at first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartMode {
    Text,
    Binary,
    Hex,
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartFont {
    Ansi,
    Ascii,
    Variable,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerSettings {
    pub start: StartMode,
    pub font: StartFont,
    pub wrap: bool,
    /// Characters a row of wrapped text, and of Binary.
    pub text_width: usize,
    pub binary_width: usize,
    /// Beside the Viewer and the Editor.
    pub minimap: bool,
    /// Code in its language's colours (`viewer::source`).
    pub syntax: bool,
}

/// Lister's defaults.
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            start: StartMode::Auto,
            font: StartFont::Auto,
            wrap: true,
            text_width: 81,
            binary_width: 75,
            minimap: true,
            syntax: true,
        }
    }
}

/// The settings dialog's pages.
pub const PAGES: &[&str] = &["Layout", "Display", "Font", "Date", "Language", "Edit/View", "Add-ons"];

pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("powercommander"))
}

impl Settings {
    pub fn filter(&self) -> Filter {
        Filter {
            hidden: self.show_hidden,
            system: self.show_system,
        }
    }

    /// What the core shows by them everywhere: dates, folders' order.
    pub fn apply_globally(&self) {
        crate::dates::set(&self.date);
        crate::set_folders_like_files(self.folders_like_files);
    }

    pub fn load() -> io::Result<Self> {
        read_json("settings.json")
    }

    pub fn save(&self) -> io::Result<()> {
        write_json("settings.json", self)
    }
}

/// The defaults when the file is not there yet.
pub(crate) fn read_json<T: Default + DeserializeOwned>(file: &str) -> io::Result<T> {
    let Some(dir) = config_dir() else {
        return Ok(T::default());
    };
    match fs::read_to_string(dir.join(file)) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err),
    }
}

/// Write then rename, so a failed save keeps the old file.
pub(crate) fn write_json(file: &str, value: &impl Serialize) -> io::Result<()> {
    let dir = config_dir().ok_or_else(|| io::Error::other("no configuration folder"))?;
    fs::create_dir_all(&dir)?;
    let temp = dir.join(format!("{file}.new"));
    fs::write(&temp, serde_json::to_string_pretty(value)?)?;
    fs::rename(temp, dir.join(file))
}
