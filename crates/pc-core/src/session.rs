use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings;

/// Both sides' tabs, kept between runs.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub sides: Vec<SideState>,
    /// The side the keys go to.
    pub active: usize,
    /// Widths of the Name, Ext, Size, Date and Attr columns; Name 0 fills the rest.
    pub columns: Vec<f32>,
    /// The left panel's share of the width.
    pub split: f32,
    /// The sidebar's tab; none while it is hidden.
    pub sidebar: Option<crate::sidebar::SidebarTab>,
    pub sidebar_width: f32,
    /// The panels one above the other.
    pub vertical: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SideState {
    pub tabs: Vec<TabState>,
    pub active: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    /// The folder, or the archive.
    pub path: PathBuf,
    /// The folder inside the archive.
    pub archive_dir: Option<PathBuf>,
    pub locked: bool,
    pub name: Option<String>,
    /// A server's tab: `path` is on it. Restored as a local tab, not reconnected.
    pub url: Option<String>,
    pub view: crate::View,
}

/// The window's tabs; the terminal's are its own.
pub const WINDOW: &str = "session.json";
pub const TERMINAL: &str = "session-terminal.json";

impl Session {
    pub fn load() -> io::Result<Self> {
        Self::load_from(WINDOW)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(WINDOW)
    }

    pub fn load_from(file: &str) -> io::Result<Self> {
        settings::read_json(file)
    }

    pub fn save_to(&self, file: &str) -> io::Result<()> {
        settings::write_json(file, self)
    }
}
