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
    /// Widths of the Ext, Size, Date and Attr columns.
    pub columns: Vec<f32>,
    /// The left panel's share of the width.
    pub split: f32,
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
}

impl Session {
    pub fn load() -> io::Result<Self> {
        settings::read_json("session.json")
    }

    pub fn save(&self) -> io::Result<()> {
        settings::write_json("session.json", self)
    }
}
