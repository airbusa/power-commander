//! Find Files, as Total Commander's: by name, text, date, size and
//! attributes, inside archives too, from the disk or the locate index; and
//! duplicate files.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use chrono::{Local, NaiveDate, TimeZone};
use md5::Digest;
use regex::Regex;
use regex::bytes::Regex as BytesRegex;
use serde::{Deserialize, Serialize};

use crate::{archive, associations, codepage, settings};

/// What to look for; Load/Save keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    /// `*.txt *.md | backup* old/`: masks, a name without `*` or `?` being
    /// part of the name; after `|` those left out, folders by their `/`.
    pub names: String,
    /// `names` is a regular expression.
    pub regex: bool,
    /// Folders to search, or files to check, `;` between them.
    pub places: String,
    /// Levels of folders below them: None all, 0 none.
    pub depth: Option<u32>,
    pub archives: bool,
    /// Names from the locate index instead of the disk.
    pub locate: bool,
    pub text: Option<Text>,
    /// Modified on these days or between them, `2026-11-12`.
    pub between: Option<(String, String)>,
    pub newer: Option<(u32, Age)>,
    pub older: Option<(u32, Age)>,
    pub size: Option<(SizeOp, u64, Unit)>,
    /// Those it must have (true) or must not.
    pub attributes: Vec<(Attribute, bool)>,
    pub duplicates: Option<Duplicates>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Text {
    pub text: String,
    pub whole_words: bool,
    pub case_sensitive: bool,
    pub regex: bool,
    /// `text` is bytes: `48 65 6c 6c 6f`.
    pub hex: bool,
    /// Files without it.
    pub not_containing: bool,
    pub utf8: bool,
    pub utf16: bool,
    /// The system's legacy code page, as Windows' ANSI one.
    pub ansi: bool,
    /// docx, xlsx, pptx, odt, ods, odp: their text, not their zip.
    pub office: bool,
}

impl Default for Text {
    fn default() -> Self {
        Text {
            text: String::new(),
            whole_words: false,
            case_sensitive: false,
            regex: false,
            hex: false,
            not_containing: false,
            utf8: true,
            utf16: false,
            ansi: false,
            office: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Age {
    Minutes,
    Hours,
    #[default]
    Days,
    Weeks,
    Months,
    Years,
}

impl Age {
    pub const ALL: [Age; 6] = [Age::Minutes, Age::Hours, Age::Days, Age::Weeks, Age::Months, Age::Years];

    pub fn name(self) -> &'static str {
        match self {
            Age::Minutes => "minute(s)",
            Age::Hours => "hour(s)",
            Age::Days => "day(s)",
            Age::Weeks => "week(s)",
            Age::Months => "month(s)",
            Age::Years => "year(s)",
        }
    }

    /// `n` of these before `now`.
    fn before(self, n: u32, now: chrono::DateTime<Local>) -> SystemTime {
        let minutes = |m: i64| now - chrono::Duration::minutes(m);
        let time = match self {
            Age::Minutes => minutes(n as i64),
            Age::Hours => minutes(n as i64 * 60),
            Age::Days => minutes(n as i64 * 60 * 24),
            Age::Weeks => minutes(n as i64 * 60 * 24 * 7),
            Age::Months => now.checked_sub_months(chrono::Months::new(n)).unwrap_or(now),
            Age::Years => now.checked_sub_months(chrono::Months::new(n.saturating_mul(12))).unwrap_or(now),
        };
        time.into()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeOp {
    #[default]
    Equal,
    Less,
    More,
}

impl SizeOp {
    pub const ALL: [SizeOp; 3] = [SizeOp::Equal, SizeOp::Less, SizeOp::More];

    pub fn name(self) -> &'static str {
        match self {
            SizeOp::Equal => "=",
            SizeOp::Less => "<",
            SizeOp::More => ">",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Bytes,
    #[default]
    Kilo,
    Mega,
    Giga,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Bytes, Unit::Kilo, Unit::Mega, Unit::Giga];

    pub fn name(self) -> &'static str {
        match self {
            Unit::Bytes => "bytes",
            Unit::Kilo => "KB",
            Unit::Mega => "MB",
            Unit::Giga => "GB",
        }
    }

    fn bytes(self) -> u64 {
        match self {
            Unit::Bytes => 1,
            Unit::Kilo => 1 << 10,
            Unit::Mega => 1 << 20,
            Unit::Giga => 1 << 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribute {
    Directory,
    /// A dot file.
    Hidden,
    /// Its owner may not write it.
    ReadOnly,
    Executable,
    Link,
}

impl Attribute {
    pub const ALL: [Attribute; 5] =
        [Attribute::Directory, Attribute::Hidden, Attribute::ReadOnly, Attribute::Executable, Attribute::Link];

    pub fn name(self) -> &'static str {
        match self {
            Attribute::Directory => "Directory",
            Attribute::Hidden => "Hidden",
            Attribute::ReadOnly => "Read only",
            Attribute::Executable => "Executable",
            Attribute::Link => "Symbolic link",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Duplicates {
    pub name: bool,
    pub size: bool,
    pub content: bool,
}

/// What a search reports as it goes.
pub enum Event {
    /// Looking in this folder now.
    Folder(PathBuf),
    Found(Found),
}

#[derive(Clone, Debug)]
pub struct Found {
    pub path: PathBuf,
    /// A file inside the archive at `path`.
    pub member: Option<archive::Member>,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Duplicates: the set of alike files it is in.
    pub group: Option<usize>,
}

impl Found {
    /// `/home/me/a.zip/docs/b.txt` for a file in an archive.
    pub fn shown(&self) -> String {
        match &self.member {
            Some(member) => self.path.join(&member.inside).display().to_string(),
            None => self.path.display().to_string(),
        }
    }
}

/// Searches `places`: folders, or files to check (what a search found
/// before); `places(&query.places)` unless so. Blocks, so run it off the
/// UI thread. `stop` ends it early.
pub fn run(query: &Query, places: &[PathBuf], stop: &AtomicBool, event: &mut dyn FnMut(Event)) -> Result<(), String> {
    let search = Search::new(query, stop)?;
    if places.is_empty() {
        return Err("Search in names no folder".into());
    }
    // Duplicates are compared once all are found.
    let mut found = Vec::new();
    let mut out = |reported: Event| match (reported, query.duplicates) {
        (Event::Found(hit), Some(_)) => found.push(hit),
        (reported, _) => event(reported),
    };
    match query.locate {
        true => {
            for path in search.located(places)? {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(hit) = search.check_path(&path) {
                    out(Event::Found(hit));
                }
            }
        }
        false => {
            for place in places {
                match fs::metadata(place) {
                    Ok(meta) if meta.is_dir() => search.walk(place, 0, &mut out),
                    // A file chosen to check.
                    Ok(_) => {
                        if let Some(hit) = search.check_path(place) {
                            out(Event::Found(hit));
                        }
                    }
                    Err(err) => return Err(format!("{}: {err}", place.display())),
                }
            }
        }
    }
    if let Some(how) = query.duplicates {
        for hit in duplicates(found, how, stop) {
            event(Event::Found(hit));
        }
    }
    Ok(())
}

/// Whether the locate index can be searched.
pub fn has_locate() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| dir.join("locate").is_file())
}

/// Where "Drives" offers to search: the root, home, and mounted disks.
pub fn drives() -> Vec<PathBuf> {
    let mut drives = vec![PathBuf::from("/")];
    drives.extend(std::env::home_dir());
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for line in mounts.lines() {
        let mut fields = line.split(' ');
        let (Some(device), Some(point)) = (fields.next(), fields.next()) else {
            continue;
        };
        // `\040` is a space in a mount point.
        let point = PathBuf::from(point.replace("\\040", " "));
        let disk = device.starts_with("/dev/") || line.contains(" fuse.") || line.contains(" nfs");
        let user = ["/media/", "/run/media/", "/mnt/"].iter().any(|dir| point.starts_with(dir))
            || std::env::home_dir().is_some_and(|home| point.starts_with(&home) && point != home);
        if disk && user && !drives.contains(&point) {
            drives.push(point);
        }
    }
    drives
}

/// A day as saved searches keep it: `2026-11-12`.
pub fn day(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

/// The folders of "Search in", `~` being home.
pub fn places(text: &str) -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    let places = text.split(';').map(str::trim).filter(|place| !place.is_empty());
    places.map(|place| crate::command::resolve(place, &home)).collect()
}

enum Names {
    Masks(String),
    Regex(Regex),
}

struct Search<'a> {
    stop: &'a AtomicBool,
    names: Names,
    /// Files and folders left out, and folders not looked into.
    left_out: String,
    folders_left_out: String,
    depth: Option<u32>,
    archives: bool,
    text: Option<TextSearch>,
    since: Option<SystemTime>,
    until: Option<SystemTime>,
    size: Option<(SizeOp, u64, Unit)>,
    attributes: Vec<(Attribute, bool)>,
    /// What locate is asked: names' globs, or a regular expression.
    locate_args: Vec<String>,
}

/// What a file's data is checked for.
struct TextSearch {
    hex: Option<Vec<u8>>,
    bytes: Option<BytesRegex>,
    text: Option<Regex>,
    utf8: bool,
    utf16: bool,
    ansi: Option<u16>,
    office: bool,
    not_containing: bool,
}

/// A file or folder, as the criteria see it.
struct Candidate<'a> {
    name: &'a str,
    is_dir: bool,
    is_link: bool,
    size: u64,
    modified: Option<SystemTime>,
    /// Unix permission bits.
    mode: Option<u32>,
}

const CHUNK: u64 = 1 << 20;
/// Kept from a chunk for the next, for a match across the two.
// shortcut: a regular expression's match longer than this across chunks is missed; join chunks if one ever is.
const OVERLAP: usize = 64 << 10;

impl<'a> Search<'a> {
    fn new(query: &Query, stop: &'a AtomicBool) -> Result<Self, String> {
        // A regular expression's `|` is its own.
        let split = query.names.split_once('|').filter(|_| !query.regex);
        let (wanted, left_out) = split.unwrap_or((&query.names, ""));
        let (mut files_out, mut folders_out) = (Vec::new(), Vec::new());
        for mask in left_out.split([' ', ';']).filter(|mask| !mask.is_empty()) {
            match mask.strip_suffix(['/', '\\']) {
                Some(folder) => folders_out.push(contains(folder)),
                None => files_out.push(contains(mask)),
            }
        }
        let names = match query.regex {
            true => Names::Regex(
                Regex::new(&format!("(?i){}", wanted.trim())).map_err(|err| format!("The name's regular expression: {err}"))?,
            ),
            false => {
                let masks: Vec<String> = wanted.split([' ', ';']).filter(|m| !m.is_empty()).map(contains).collect();
                Names::Masks(if masks.is_empty() { "*".into() } else { masks.join(" ") })
            }
        };
        // Globs on the name let plocate use its index; a regular expression reads it all.
        let locate_args = match &names {
            Names::Regex(_) => vec!["--regex".into(), wanted.trim().to_owned()],
            Names::Masks(masks) => ["-b"].into_iter().chain(masks.split(' ')).map(str::to_owned).collect(),
        };
        let day = |text: &str, end: bool| -> Result<SystemTime, String> {
            let date = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
                .map_err(|_| format!("{text} is no date such as 2026-11-12"))?;
            let date = if end { date.succ_opt().unwrap_or(date) } else { date };
            let midnight = date.and_hms_opt(0, 0, 0).expect("midnight");
            let local = Local.from_local_datetime(&midnight).earliest().ok_or(format!("{text} is no local time"))?;
            Ok(local.into())
        };
        let now = Local::now();
        let mut since = query.newer.map(|(n, age)| age.before(n, now));
        let mut until = query.older.map(|(n, age)| age.before(n, now));
        if let Some((from, to)) = &query.between {
            let (from, to) = (day(from, false)?, day(to, true)?);
            since = Some(since.map_or(from, |since| since.max(from)));
            until = Some(until.map_or(to, |until| until.min(to)));
        }
        let text = query.text.as_ref().map(TextSearch::new).transpose()?;
        Ok(Search {
            stop,
            names,
            left_out: files_out.join(" "),
            folders_left_out: folders_out.join(" "),
            depth: query.depth,
            archives: query.archives,
            text,
            since,
            until,
            size: query.size,
            attributes: query.attributes.clone(),
            locate_args,
        })
    }

    fn walk(&self, dir: &Path, level: u32, out: &mut dyn FnMut(Event)) {
        if self.stop.load(Ordering::Relaxed) {
            return;
        }
        out(Event::Folder(dir.to_owned()));
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut folders = Vec::new();
        for entry in entries.flatten() {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let path = entry.path();
            let Some(hit) = self.stat(&path) else {
                continue;
            };
            if let Some(found) = self.check(&path, &hit) {
                out(Event::Found(found));
            }
            let name = entry.file_name();
            // Links to folders are not followed: they may loop.
            if hit.is_dir && !hit.is_link && !associations::matches(&self.folders_left_out, &name.to_string_lossy()) {
                folders.push(path);
            } else if self.archives && !hit.is_dir && archive::is_archive(&name) {
                self.in_archive(&path, out);
            }
        }
        if self.depth.is_none_or(|depth| level < depth) {
            for folder in folders {
                self.walk(&folder, level + 1, out);
            }
        }
    }

    fn stat(&self, path: &Path) -> Option<Stat> {
        let meta = fs::symlink_metadata(path).ok()?;
        let is_link = meta.file_type().is_symlink();
        let meta = if is_link { fs::metadata(path).unwrap_or(meta) } else { meta };
        #[cfg(unix)]
        let mode = Some(std::os::unix::fs::PermissionsExt::mode(&meta.permissions()));
        #[cfg(not(unix))]
        let mode = meta.permissions().readonly().then_some(0o444);
        Some(Stat {
            is_dir: meta.is_dir(),
            is_link,
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            mode,
        })
    }

    /// A path named, not walked to: the locate index's, or one chosen.
    fn check_path(&self, path: &Path) -> Option<Found> {
        let stat = self.stat(path)?;
        self.check(path, &stat)
    }

    fn check(&self, path: &Path, stat: &Stat) -> Option<Found> {
        let name = path.file_name()?.to_string_lossy();
        let candidate = Candidate {
            name: &name,
            is_dir: stat.is_dir,
            is_link: stat.is_link,
            size: stat.size,
            modified: stat.modified,
            mode: stat.mode,
        };
        if !self.fits(&candidate) {
            return None;
        }
        if let Some(text) = &self.text
            && !text.in_file(path, self.stop)
        {
            return None;
        }
        Some(Found {
            path: path.to_owned(),
            member: None,
            is_dir: stat.is_dir,
            size: stat.size,
            modified: stat.modified,
            group: None,
        })
    }

    /// All but the text.
    fn fits(&self, candidate: &Candidate) -> bool {
        let named = match &self.names {
            Names::Masks(masks) => associations::matches(masks, candidate.name),
            Names::Regex(regex) => regex.is_match(candidate.name),
        };
        let left_out = match candidate.is_dir {
            true => associations::matches(&self.folders_left_out, candidate.name),
            false => associations::matches(&self.left_out, candidate.name),
        };
        if !named || left_out {
            return false;
        }
        // Folders have no text, nor a size to compare.
        if candidate.is_dir && (self.text.is_some() || self.size.is_some()) {
            return false;
        }
        let modified = candidate.modified;
        if self.since.is_some_and(|since| modified.is_none_or(|time| time < since))
            || self.until.is_some_and(|until| modified.is_none_or(|time| time >= until))
        {
            return false;
        }
        if let Some((op, n, unit)) = self.size {
            let size = candidate.size;
            let fits = match op {
                SizeOp::Equal => size / unit.bytes() == n,
                SizeOp::Less => size < n * unit.bytes(),
                SizeOp::More => size > n * unit.bytes(),
            };
            if !fits {
                return false;
            }
        }
        self.attributes.iter().all(|&(attribute, wanted)| {
            let mode = candidate.mode.unwrap_or(0o644);
            let has = match attribute {
                Attribute::Directory => candidate.is_dir,
                Attribute::Hidden => candidate.name.starts_with('.'),
                Attribute::ReadOnly => mode & 0o200 == 0,
                Attribute::Executable => !candidate.is_dir && mode & 0o111 != 0,
                Attribute::Link => candidate.is_link,
            };
            has == wanted
        })
    }

    /// The archive's files that fit; their text from a copy taken out.
    fn in_archive(&self, path: &Path, out: &mut dyn FnMut(Event)) {
        let Ok(items) = archive::list(path) else {
            return;
        };
        let fits = items.into_iter().filter(|item| {
            let name = item.path.file_name().unwrap_or_default().to_string_lossy();
            let attrs = &item.entry.attributes;
            let candidate = Candidate {
                name: &name,
                is_dir: item.entry.is_dir,
                is_link: attrs.starts_with('l'),
                size: item.entry.size,
                modified: item.entry.modified,
                mode: unix_mode(attrs),
            };
            self.fits(&candidate)
        });
        let fits: Vec<archive::Item> = fits.collect();
        let hit = |item: &archive::Item| Found {
            path: path.to_owned(),
            member: Some(archive::Member {
                archive: path.to_owned(),
                listed: item.listed.clone(),
                name: item.path.file_name().unwrap_or_default().to_owned(),
                inside: item.path.clone(),
            }),
            is_dir: item.entry.is_dir,
            size: item.entry.size,
            modified: item.entry.modified,
            group: None,
        };
        let Some(text) = &self.text else {
            return fits.iter().for_each(|item| out(Event::Found(hit(item))));
        };
        if fits.is_empty() {
            return;
        }
        let Ok(dir) = archive::fresh_scratch() else {
            return;
        };
        let listed: Vec<&str> = fits.iter().map(|item| item.listed.as_str()).collect();
        if archive::extract(path, &listed, &dir, true).is_ok() {
            for item in &fits {
                if text.in_file(&dir.join(&item.path), self.stop) {
                    out(Event::Found(hit(item)));
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// The index's paths in `places`, as deep as asked.
    fn located(&self, places: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
        let output = Command::new("locate")
            .args(["-0", "-e", "-i"])
            .args(&self.locate_args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|err| format!("locate: {err}"))?;
        // 1: nothing found.
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(format!("locate: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        let paths = output.stdout.split(|&byte| byte == 0).filter(|path| !path.is_empty());
        let paths = paths.map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()));
        let within = |path: &PathBuf| {
            places.iter().any(|place| {
                let Ok(below) = path.strip_prefix(place) else {
                    return false;
                };
                let levels = below.components().count() as u32;
                levels > 0 && self.depth.is_none_or(|depth| levels <= depth + 1)
            })
        };
        Ok(paths.filter(within).collect())
    }
}

struct Stat {
    is_dir: bool,
    is_link: bool,
    size: u64,
    modified: Option<SystemTime>,
    mode: Option<u32>,
}

/// A name without wildcards is any name with it in.
fn contains(mask: &str) -> String {
    match mask.contains(['*', '?']) {
        true => mask.to_owned(),
        false => format!("*{mask}*"),
    }
}

/// `-rwxr-xr-x` as 7-Zip lists a Unix file, as permission bits.
fn unix_mode(attributes: &str) -> Option<u32> {
    let bits = attributes.get(attributes.len().checked_sub(9)?..)?;
    let mode = bits.chars().fold(0, |mode, c| mode << 1 | u32::from(c != '-'));
    bits.chars().all(|c| "rwxsStT-".contains(c)).then_some(mode)
}

impl TextSearch {
    fn new(text: &Text) -> Result<Self, String> {
        if text.text.is_empty() {
            return Err("Find text names no text".into());
        }
        if text.hex {
            let digits: String = text.text.chars().filter(|c| !c.is_whitespace()).collect();
            let bytes = (0..digits.len())
                .step_by(2)
                .map(|at| digits.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
                .collect::<Option<Vec<u8>>>()
                .filter(|bytes| !bytes.is_empty())
                .ok_or_else(|| format!("{} is no hex string such as 48 65 6c", text.text))?;
            return Ok(TextSearch {
                hex: Some(bytes),
                bytes: None,
                text: None,
                utf8: false,
                utf16: false,
                ansi: None,
                office: false,
                not_containing: text.not_containing,
            });
        }
        let pattern = match text.regex {
            true => text.text.clone(),
            false => regex::escape(&text.text),
        };
        let pattern = if text.whole_words { format!(r"\b(?:{pattern})\b") } else { pattern };
        let pattern = if text.case_sensitive { pattern } else { format!("(?i){pattern}") };
        let fail = |err: regex::Error| format!("The text's regular expression: {err}");
        let any = text.utf8 || text.utf16 || text.ansi;
        Ok(TextSearch {
            hex: None,
            bytes: Some(BytesRegex::new(&pattern).map_err(fail)?),
            text: Some(Regex::new(&pattern).map_err(fail)?),
            // None chosen: UTF-8.
            utf8: text.utf8 || !any,
            utf16: text.utf16,
            ansi: text.ansi.then(|| codepage::system().0),
            office: text.office,
            not_containing: text.not_containing,
        })
    }

    /// Whether the file has it, or has not where that is asked; an unreadable one fits neither.
    fn in_file(&self, path: &Path, stop: &AtomicBool) -> bool {
        let has = match self.office.then(|| office_text(path)).flatten() {
            Some(text) => Ok(self.text.as_ref().is_some_and(|regex| regex.is_match(&text))),
            None => self.in_data(path, stop),
        };
        has.is_ok_and(|has| has != self.not_containing)
    }

    fn in_data(&self, path: &Path, stop: &AtomicBool) -> io::Result<bool> {
        let mut file = File::open(path)?;
        let mut buffer = Vec::with_capacity(CHUNK as usize + OVERLAP);
        loop {
            let read = (&mut file).take(CHUNK).read_to_end(&mut buffer)?;
            if self.in_bytes(&buffer) {
                return Ok(true);
            }
            if (read as u64) < CHUNK || stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            // Even, for UTF-16.
            let keep = buffer.len().saturating_sub(OVERLAP) & !1;
            buffer.drain(..keep);
        }
    }

    fn in_bytes(&self, bytes: &[u8]) -> bool {
        if let Some(hex) = &self.hex {
            return memchr::memmem::find(bytes, hex).is_some();
        }
        let text = |decoded: &str| self.text.as_ref().is_some_and(|regex| regex.is_match(decoded));
        (self.utf8 && self.bytes.as_ref().is_some_and(|regex| regex.is_match(bytes)))
            || (self.utf16 && text(&utf16(bytes)))
            || self.ansi.is_some_and(|page| text(&codepage::decode(page, bytes)))
    }
}

/// Little-endian unless the byte order mark says big.
fn utf16(bytes: &[u8]) -> String {
    let encoding = match bytes.starts_with(&[0xfe, 0xff]) {
        true => encoding_rs::UTF_16BE,
        false => encoding_rs::UTF_16LE,
    };
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// The text of an Office or OpenDocument file; None for other files.
fn office_text(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let parts: &[&str] = match extension.as_str() {
        "docx" | "docm" | "dotx" => &["word/document.xml", "word/header*.xml", "word/footer*.xml"],
        "xlsx" | "xlsm" => &["xl/sharedStrings.xml", "xl/worksheets/*.xml"],
        "pptx" | "pptm" => &["ppt/slides/*.xml"],
        "odt" | "ods" | "odp" | "odg" => &["content.xml"],
        _ => return None,
    };
    let output = archive::seven_zip(&["e", "-so"], Some(path)).args(parts).stderr(Stdio::null()).output().ok()?;
    Some(xml_text(&String::from_utf8_lossy(&output.stdout)))
}

/// XML's text: tags out, a paragraph's end a line's, entities decoded.
fn xml_text(xml: &str) -> String {
    let mut text = String::with_capacity(xml.len() / 2);
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        text.push_str(&decode_entities(&rest[..open]));
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        // Paragraphs, cells and breaks keep words apart.
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        if tag.starts_with('/') && matches!(local, "p" | "c" | "si" | "table-cell" | "h") || matches!(local, "br" | "tab" | "s") {
            text.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    text.push_str(&decode_entities(rest));
    text
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let entity = rest[at + 1..].split_once(';').map(|(name, _)| name).filter(|name| name.len() <= 8);
        let decoded = entity.and_then(|name| match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let number = name.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (decoded, entity) {
            (Some(c), Some(name)) => {
                out.push(c);
                rest = &rest[at + name.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The files alike in what `how` names, a set after another; sets of one go.
fn duplicates(found: Vec<Found>, how: Duplicates, stop: &AtomicBool) -> Vec<Found> {
    // shortcut: files in archives are not compared; take them out first if asked for.
    let files = found.into_iter().filter(|hit| !hit.is_dir && hit.member.is_none());
    let mut sets: HashMap<(Option<std::ffi::OsString>, Option<u64>), Vec<Found>> = HashMap::new();
    for hit in files {
        let name = how.name.then(|| hit.path.file_name().unwrap_or_default().to_owned());
        let size = (how.size || how.content).then_some(hit.size);
        sets.entry((name, size)).or_default().push(hit);
    }
    let mut sets: Vec<Vec<Found>> = sets.into_values().filter(|set| set.len() > 1).collect();
    if how.content {
        let mut same = Vec::new();
        for set in sets {
            let mut by_content: HashMap<[u8; 16], Vec<Found>> = HashMap::new();
            for hit in set {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(digest) = md5_of(&hit.path) {
                    by_content.entry(digest).or_default().push(hit);
                }
            }
            same.extend(by_content.into_values().filter(|set| set.len() > 1));
        }
        sets = same;
    }
    for set in &mut sets {
        set.sort_by(|a, b| a.path.cmp(&b.path));
    }
    sets.sort_by(|a, b| a[0].path.cmp(&b[0].path));
    let numbered = sets.into_iter().enumerate();
    numbered.flat_map(|(group, set)| set.into_iter().map(move |hit| Found { group: Some(group), ..hit })).collect()
}

fn md5_of(path: &Path) -> io::Result<[u8; 16]> {
    let mut file = File::open(path)?;
    let mut hasher = md5::Md5::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().into())
}

/// Saved searches, and what was typed before; the viewer's Find keeps its own.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub searches: Vec<(String, Query)>,
    pub names: Vec<String>,
    pub places: Vec<String>,
    pub texts: Vec<String>,
}

impl Saved {
    pub fn load() -> io::Result<Self> {
        settings::read_json("find_files.json")
    }

    pub fn save(&self) -> io::Result<()> {
        settings::write_json("find_files.json", self)
    }

    /// Puts what was typed first in its history.
    pub fn remember(list: &mut Vec<String>, typed: &str) {
        let typed = typed.trim();
        if typed.is_empty() {
            return;
        }
        list.retain(|old| old != typed);
        list.insert(0, typed.to_owned());
        list.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pc-search-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, data) in [
            ("a.txt", "Hello World\n".as_bytes()),
            ("b.md", b"hello again"),
            ("sub/c.txt", b"nothing here"),
            ("sub/deeper/d.txt", b"Hello deep"),
            ("backup/e.txt", b"Hello backup"),
            (".hidden", b"x"),
            ("copy/a.txt", b"Hello World\n"),
            ("other/a.txt", b"Different!!\n"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        let utf16: Vec<u8> = "Grüße aus Ungarn".encode_utf16().flat_map(u16::to_le_bytes).collect();
        fs::write(root.join("wide.txt"), utf16).unwrap();
        root
    }

    fn names(query: &Query) -> Vec<String> {
        let stop = AtomicBool::new(false);
        let mut found = Vec::new();
        run(query, &places(&query.places), &stop, &mut |event| {
            if let Event::Found(hit) = event {
                let root = Path::new(&query.places);
                let shown = hit.path.strip_prefix(root).unwrap().display().to_string();
                found.push(match hit.group {
                    Some(group) => format!("{group}:{shown}"),
                    None => shown,
                });
            }
        })
        .unwrap();
        if query.duplicates.is_none() {
            found.sort();
        }
        found
    }

    #[test]
    fn finds_by_name() {
        let root = tree();
        let query = |names: &str| Query {
            names: names.into(),
            places: root.display().to_string(),
            ..Query::default()
        };
        let all_txt = ["a.txt", "backup/e.txt", "copy/a.txt", "other/a.txt", "sub/c.txt", "sub/deeper/d.txt", "wide.txt"];
        assert_eq!(names(&query("*.txt")), all_txt);
        // No wildcard: a part of the name; a folder found by name too.
        assert_eq!(names(&query("deep")), ["sub/deeper"]);
        assert_eq!(names(&query("*.txt | backup/ copy/ other/ wide*")), ["a.txt", "sub/c.txt", "sub/deeper/d.txt"]);
        assert_eq!(names(&Query { depth: Some(0), ..query("*.txt") }), ["a.txt", "wide.txt"]);
        assert_eq!(names(&Query { depth: Some(1), ..query("*.txt") }).len(), 6);
        assert_eq!(names(&Query { regex: true, ..query(r"^[ab]\.(txt|md)$") }), ["a.txt", "b.md", "copy/a.txt", "other/a.txt"]);
        let hidden = Query { attributes: vec![(Attribute::Hidden, true)], ..query("") };
        assert_eq!(names(&hidden), [".hidden"]);
        let folders = Query { attributes: vec![(Attribute::Directory, true)], ..query("") };
        assert_eq!(names(&folders), ["backup", "copy", "other", "sub", "sub/deeper"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_by_text() {
        let root = tree();
        let query = |text: Text| Query {
            names: "*.txt *.md".into(),
            places: root.display().to_string(),
            text: Some(text),
            ..Query::default()
        };
        let text = |text: &str| Text { text: text.into(), ..Text::default() };
        let hello = ["a.txt", "b.md", "backup/e.txt", "copy/a.txt", "sub/deeper/d.txt"];
        assert_eq!(names(&query(text("hello"))), hello);
        assert_eq!(names(&query(Text { case_sensitive: true, ..text("hello") })), ["b.md"]);
        assert_eq!(names(&query(Text { whole_words: true, ..text("hell") })), Vec::<String>::new());
        assert_eq!(names(&query(Text { regex: true, ..text(r"wor\w+") })), ["a.txt", "copy/a.txt"]);
        assert_eq!(names(&query(Text { hex: true, ..text("44 69 66") })), ["other/a.txt"]);
        let not = names(&query(Text { not_containing: true, ..text("hello") }));
        assert_eq!(not, ["other/a.txt", "sub/c.txt", "wide.txt"]);
        assert_eq!(names(&query(text("Grüße"))), Vec::<String>::new());
        assert_eq!(names(&query(Text { utf16: true, utf8: false, ..text("grüße") })), ["wide.txt"]);
        let fails = |query: Query| run(&query, &places(&query.places), &AtomicBool::new(false), &mut |_| {}).is_err();
        assert!(fails(query(text(""))));
        assert!(fails(query(Text { hex: true, ..text("4g") })));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_by_size_and_date() {
        let root = tree();
        let query = Query {
            names: "*.txt".into(),
            places: root.display().to_string(),
            ..Query::default()
        };
        let bigger = Query { size: Some((SizeOp::More, 12, Unit::Bytes)), ..query.clone() };
        assert_eq!(names(&bigger), ["wide.txt"]);
        let exact = Query { size: Some((SizeOp::Equal, 12, Unit::Bytes)), ..query.clone() };
        assert_eq!(names(&exact), ["a.txt", "backup/e.txt", "copy/a.txt", "other/a.txt", "sub/c.txt"]);
        let old = SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
        File::options().write(true).open(root.join("a.txt")).unwrap().set_modified(old).unwrap();
        let recent = Query { newer: Some((2, Age::Days)), ..query.clone() };
        assert!(!names(&recent).contains(&"a.txt".to_owned()));
        assert_eq!(names(&Query { older: Some((2, Age::Days)), ..query.clone() }), ["a.txt"]);
        let today = Local::now().format("%Y-%m-%d").to_string();
        let between = Query { between: Some((today.clone(), today)), ..query.clone() };
        assert_eq!(names(&between).len(), 6);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_duplicates() {
        let root = tree();
        let query = |how| Query {
            names: "a.txt".into(),
            places: root.display().to_string(),
            duplicates: Some(how),
            ..Query::default()
        };
        let named = Duplicates { name: true, size: true, content: false };
        assert_eq!(names(&query(named)), ["0:a.txt", "0:copy/a.txt", "0:other/a.txt"]);
        let same = Duplicates { name: true, size: true, content: true };
        assert_eq!(names(&query(same)), ["0:a.txt", "0:copy/a.txt"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn searches_archives_and_office_files() {
        let root = tree();
        let pack = |kind: &str, archive: &str, what: &str, dir: &Path| {
            let status = archive::seven_zip(&["a", "-bso0", kind], Some(&root.join(archive)))
                .arg(what)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(status.success());
        };
        pack("-tzip", "pack.zip", "sub", &root);
        // A document's text is in its XML, deflated in a zip.
        let doc = root.join("doc");
        fs::create_dir_all(doc.join("word")).unwrap();
        let xml = "<w:document><w:p><w:r><w:t>Quarterly </w:t></w:r><w:r><w:t>report</w:t></w:r></w:p></w:document>";
        fs::write(doc.join("word/document.xml"), xml).unwrap();
        pack("-tzip", "report.docx", "word", &doc);
        fs::remove_dir_all(&doc).unwrap();
        let query = |names: &str, text: Option<Text>| Query {
            names: names.into(),
            places: root.display().to_string(),
            depth: Some(0),
            archives: true,
            text,
            ..Query::default()
        };
        let stop = AtomicBool::new(false);
        let mut found = Vec::new();
        let hello = Text { text: "hello".into(), ..Text::default() };
        let places = [root.clone()];
        run(&query("*.txt", Some(hello)), &places, &stop, &mut |event| {
            if let Event::Found(hit) = event {
                found.push(hit.shown().strip_prefix(&format!("{}/", root.display())).unwrap().to_owned());
            }
        })
        .unwrap();
        found.sort();
        assert_eq!(found, ["a.txt", "pack.zip/sub/deeper/d.txt"]);
        let office = Text { text: "quarterly report".into(), office: true, ..Text::default() };
        assert_eq!(names(&query("*.docx", Some(office.clone()))), ["report.docx"]);
        assert!(names(&query("*.docx", Some(Text { office: false, ..office }))).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    /// Reads the system's locate database: `cargo test -p pc-core locates -- --ignored`.
    #[test]
    #[ignore]
    fn locates_names() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let query = Query {
            names: "Cargo.toml".into(),
            locate: true,
            depth: Some(1),
            ..Query::default()
        };
        let mut found = Vec::new();
        run(&query, &[repo.clone()], &AtomicBool::new(false), &mut |event| {
            if let Event::Found(hit) = event {
                found.push(hit.path.strip_prefix(&repo).unwrap().display().to_string());
            }
        })
        .unwrap();
        found.sort();
        // One level down: crates/pc-core/Cargo.toml is two.
        assert_eq!(found, ["Cargo.toml"]);
    }

    #[test]
    fn reads_office_xml() {
        let xml = "<w:p><w:r><w:t>Hel</w:t></w:r><w:r><w:t>lo &amp; bye</w:t></w:r></w:p><w:p><w:t>Next&#x21;</w:t></w:p>";
        assert_eq!(xml_text(xml), "Hello & bye\nNext!\n");
        assert_eq!(decode_entities("a &unknown; b & c"), "a &unknown; b & c");
        assert_eq!(unix_mode("-rwxr-x---"), Some(0o750));
        assert_eq!(unix_mode("D"), None);
        let mut history = vec!["a".to_owned(), "b".to_owned()];
        Saved::remember(&mut history, " b ");
        assert_eq!(history, ["b", "a"]);
    }
}
