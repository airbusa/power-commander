//! Commands > Activity Log: what was done to files, and how each ended,
//! kept across runs as JSON lines in the app's data folder, the newest
//! `KEPT`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since 1970.
    pub at: u64,
    /// What: "Copy 3 items to /home/me".
    pub title: String,
    pub outcome: Outcome,
    /// Why it failed, or what a task found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Entry {
    pub fn time(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.at)
    }
}

/// How many are kept.
const KEPT: usize = 1000;

fn file() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("powercommander/activity.jsonl"))
}

/// Notes it; never fails what it notes. Tests write none.
pub fn record(title: impl Into<String>, outcome: Outcome, note: Option<String>) {
    if cfg!(test) {
        return;
    }
    let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let entry = Entry { at, title: title.into(), outcome, note };
    let (Some(file), Ok(line)) = (file(), serde_json::to_string(&entry)) else {
        return;
    };
    let _ = file.parent().map(fs::create_dir_all);
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = writeln!(log, "{line}");
    }
    // Twice the kept: cut back to them.
    if fs::metadata(&file).is_ok_and(|meta| meta.len() > (KEPT as u64) * 2 * 200) {
        let kept: Vec<String> = recent().iter().rev().filter_map(|entry| serde_json::to_string(entry).ok()).collect();
        let _ = fs::write(&file, kept.join("\n") + "\n");
    }
}

/// The newest `KEPT`, newest first; lines it cannot read are left out.
pub fn recent() -> Vec<Entry> {
    let text = file().and_then(|file| fs::read_to_string(file).ok()).unwrap_or_default();
    let mut entries: Vec<Entry> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    entries.reverse();
    entries.truncate(KEPT);
    entries
}

pub fn clear() -> std::io::Result<()> {
    match file() {
        Some(file) if file.exists() => fs::remove_file(file),
        _ => Ok(()),
    }
}

/// What `result` says of `title`, noted.
pub fn note<T>(title: impl Into<String>, result: &std::io::Result<T>) {
    match result {
        Ok(_) => record(title, Outcome::Done, None),
        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => record(title, Outcome::Cancelled, None),
        Err(err) => record(title, Outcome::Failed, Some(err.to_string())),
    }
}
