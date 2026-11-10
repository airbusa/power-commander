use std::fs;
use std::io;
use std::path::PathBuf;

use serde::de::DeserializeOwned;

use crate::Filter;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// None follows the system.
    pub theme: Option<String>,
    /// Show the tab header also when there is only one tab.
    pub tab_header_always: bool,
    pub show_hidden: bool,
    pub show_system: bool,
}

/// The settings dialog's pages, with their depth in its tree.
pub const PAGES: &[(u8, &str)] = &[
    (0, "Layout"),
    (0, "Display"),
    (1, "Icons"),
    (1, "Font"),
    (1, "Color"),
    (1, "Tabstops/Data Formats"),
    (1, "Folder Tabs"),
    (1, "Custom columns"),
    (1, "View Mode"),
    (2, "Auto Switch Mode"),
    (1, "Language"),
    (0, "Operation"),
    (1, "Edit/View"),
    (1, "Copy/Delete"),
    (1, "Refresh"),
    (1, "Quick Search"),
    (1, "FTP"),
    (1, "Plugins"),
    (1, "Thumbnails"),
    (1, "Log file"),
    (1, "Ignore list"),
    (1, "History"),
    (0, "Packer"),
    (1, "Zip packer"),
    (1, "7-Zip packer"),
    (0, "Misc."),
];

pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("powercommander"))
}

pub fn themes_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("themes"))
}

impl Settings {
    pub fn filter(&self) -> Filter {
        Filter {
            hidden: self.show_hidden,
            system: self.show_system,
        }
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
