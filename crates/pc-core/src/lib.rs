pub mod archive;
pub mod attributes;
pub mod codepage;
pub mod command;
pub mod connections;
pub mod edit;
pub mod ftp;
pub mod html;
pub mod icons;
pub mod menu;
pub mod ops;
pub mod origin;
pub mod pdf;
pub mod properties;
pub mod remote;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod space;
pub mod theme;
pub mod vfs;
pub mod viewer;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::session::{SideState, TabState};
use crate::vfs::{Fs, Local, Remote};
use std::sync::Arc;

pub const PARENT: &str = "..";

#[derive(Clone, Debug)]
pub struct Entry {
    /// Not always UTF-8.
    pub name: OsString,
    /// Symbolic links to folders are folders.
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Dot files on Linux and macOS, the hidden attribute on Windows.
    pub hidden: bool,
    /// The system attribute; Windows only.
    pub system: bool,
    /// As the Attr column shows them: `rwxr-xr-x`, or `rahs` on Windows.
    pub attributes: String,
}

/// Which entries a panel lists besides the usual ones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filter {
    pub hidden: bool,
    pub system: bool,
}

impl Filter {
    fn shows(self, entry: &Entry) -> bool {
        (self.hidden || !entry.hidden) && (self.system || !entry.system)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Extension,
    Modified,
    Size,
    Attributes,
    /// As the folder lists them.
    Unsorted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Sort {
    /// Newest and largest first, names A to Z.
    pub fn by(key: SortKey) -> Self {
        Self {
            key,
            descending: matches!(key, SortKey::Modified | SortKey::Size),
        }
    }

    /// `..` first, then folders by name (as Total Commander by default), then files.
    fn compare(self, a: &Entry, b: &Entry) -> Ordering {
        let parent = b.is_parent().cmp(&a.is_parent());
        let dirs = parent.then(b.is_dir.cmp(&a.is_dir));
        if a.is_dir || b.is_dir {
            return dirs.then_with(|| by_name(a, b));
        }
        let files = match self.key {
            SortKey::Name => by_name(a, b),
            SortKey::Extension => extension(a).cmp(&extension(b)).then_with(|| by_name(a, b)),
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(|| by_name(a, b)),
            SortKey::Size => a.size.cmp(&b.size).then_with(|| by_name(a, b)),
            SortKey::Attributes => a.attributes.cmp(&b.attributes).then_with(|| by_name(a, b)),
            SortKey::Unsorted => Ordering::Equal,
        };
        dirs.then(if self.descending {
            files.reverse()
        } else {
            files
        })
    }
}

fn extension(entry: &Entry) -> String {
    Path::new(&entry.name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

impl Entry {
    pub fn is_parent(&self) -> bool {
        self.name == PARENT
    }

    pub fn display_name(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }

    /// For the Name and Ext columns; folders keep their name whole.
    pub fn name_and_extension(&self) -> (String, String) {
        let name = self.display_name();
        match name.rfind('.') {
            Some(dot) if dot > 0 && !self.is_dir => {
                (name[..dot].to_owned(), name[dot + 1..].to_owned())
            }
            _ => (name, String::new()),
        }
    }

    pub fn size_text(&self) -> String {
        if self.is_dir {
            return "<DIR>".into();
        }
        grouped(self.size)
    }

    pub fn modified_text(&self) -> String {
        self.modified
            .map(|time| local_time(time, "%Y-%m-%d %H:%M"))
            .unwrap_or_default()
    }
}

/// `1.5 MB`, for speeds and progress.
pub fn human_size(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        0..K => format!("{bytes} B"),
        K..1_048_576 => format!("{:.1} KB", bytes as f64 / K as f64),
        1_048_576..1_073_741_824 => format!("{:.1} MB", bytes as f64 / 1_048_576.),
        _ => format!("{:.2} GB", bytes as f64 / 1_073_741_824.),
    }
}

/// `time` in the local zone, as `format` says.
pub fn local_time(time: SystemTime, format: &str) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format(format)
        .to_string()
}

#[derive(Clone)]
pub struct Panel {
    /// The folder, or the archive `archive` browses; on `remote`'s server if any.
    path: PathBuf,
    archive: Option<Archive>,
    remote: Option<Arc<dyn Remote>>,
    entries: Vec<Entry>,
    cursor: usize,
    /// Names of the marked entries: what F5, F6 and F8 work on.
    marked: HashSet<OsString>,
    /// Folders' sizes Calculate Occupied Space found, until the panel moves.
    measured: HashMap<OsString, u64>,
    /// Moves from a locked tab open a new tab.
    locked: bool,
    /// The tab's own name.
    name: Option<String>,
    filter: Filter,
    sort: Sort,
}

#[derive(Clone)]
struct Archive {
    items: Vec<archive::Item>,
    dir: PathBuf,
    /// Where the archive is on the server; `path` is a copy taken from there.
    origin: Option<PathBuf>,
}

impl Panel {
    pub fn new(path: impl Into<PathBuf>, filter: Filter) -> io::Result<Self> {
        let mut panel = Self {
            path: path.into(),
            archive: None,
            remote: None,
            entries: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            measured: HashMap::new(),
            locked: false,
            name: None,
            filter,
            sort: Sort::default(),
        };
        panel.reload()?;
        Ok(panel)
    }

    /// Where `state` was, or the nearest folder above it that still exists.
    pub fn restore(state: &TabState, filter: Filter) -> io::Result<Self> {
        if state.url.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "a server's tab",
            ));
        }
        let exact = match &state.archive_dir {
            Some(dir) => Self::new(state.path.parent().unwrap_or(&state.path), filter).and_then(
                |mut panel| {
                    panel.enter_archive(state.path.clone(), None)?;
                    panel.show_archive_dir(dir.clone(), None);
                    Ok(panel)
                },
            ),
            None => Self::new(&state.path, filter),
        };
        let mut panel = exact.or_else(|err| {
            let above = state.path.ancestors().skip(1);
            above
                .into_iter()
                .find_map(|dir| Self::new(dir, filter).ok())
                .ok_or(err)
        })?;
        panel.locked = state.locked;
        panel.name = state.name.clone();
        Ok(panel)
    }

    pub fn state(&self) -> TabState {
        TabState {
            path: self.path.clone(),
            archive_dir: self.archive.as_ref().map(|archive| archive.dir.clone()),
            locked: self.locked,
            name: self.name.clone(),
            url: self.remote.as_ref().map(|remote| remote.url()),
        }
    }

    /// The path as the title bar shows it: `sftp://user@host/dir` on a server.
    pub fn location(&self) -> String {
        let path = self.path().display().to_string();
        match &self.remote {
            Some(remote) => format!("{}{}", remote.url(), path),
            None => path,
        }
    }

    pub fn remote(&self) -> Option<&Arc<dyn Remote>> {
        self.remote.as_ref()
    }

    /// What copies read from and write to here.
    pub fn fs(&self) -> Arc<dyn Fs> {
        match &self.remote {
            Some(remote) => remote.clone(),
            None => Arc::new(Local),
        }
    }

    /// Shows `path` on `remote`'s server.
    pub fn go_remote(&mut self, remote: Arc<dyn Remote>, path: PathBuf) -> io::Result<()> {
        let entries = remote.entries(&path)?;
        self.remote = Some(remote);
        self.archive = None;
        self.marked.clear();
        self.measured.clear();
        self.path = path;
        self.show(entries, None);
        Ok(())
    }

    /// Leaves the server for the local `path`.
    pub fn disconnect(&mut self, path: PathBuf) -> io::Result<()> {
        let entries = read_dir(&path)?;
        self.remote = None;
        self.archive = None;
        self.marked.clear();
        self.measured.clear();
        self.path = path;
        self.show(entries, None);
        Ok(())
    }

    pub fn path(&self) -> PathBuf {
        match &self.archive {
            Some(archive) if archive.dir.as_os_str().is_empty() => self.archive_path().to_owned(),
            Some(archive) => self.archive_path().join(&archive.dir),
            None => self.path.clone(),
        }
    }

    /// The archive browsed, where it is: on the server for one taken from there.
    fn archive_path(&self) -> &Path {
        let origin = self
            .archive
            .as_ref()
            .and_then(|archive| archive.origin.as_deref());
        origin.unwrap_or(&self.path)
    }

    /// The tab's own name, or its folder's.
    pub fn name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let path = self.path();
        match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => path.display().to_string(),
        }
    }

    /// As its tab shows it; locked tabs start with `*`.
    pub fn title(&self) -> String {
        match self.locked {
            true => format!("*{}", self.name()),
            false => self.name(),
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn move_cursor(&mut self, rows: isize) {
        self.set_cursor(self.cursor.saturating_add_signed(rows));
    }

    pub fn set_cursor(&mut self, ix: usize) {
        self.cursor = ix.min(self.entries.len().saturating_sub(1));
    }

    /// False on a file that is no archive.
    pub fn open(&mut self) -> io::Result<bool> {
        let Some(entry) = self.entries.get(self.cursor) else {
            return Ok(false);
        };
        let (name, is_dir) = (entry.name.clone(), entry.is_dir);
        if entry.is_parent() {
            self.up()?;
        } else if let Some(archive) = &self.archive {
            if !is_dir {
                return Ok(false);
            }
            self.show_archive_dir(archive.dir.join(name), None);
        } else if is_dir {
            self.go_to(self.path.join(name), None)?;
        } else if archive::is_archive(&name) {
            let path = self.path.join(name);
            match &self.remote {
                // shortcut: no progress while it downloads; show one if big archives make it feel stuck.
                Some(remote) => self.enter_archive(vfs::fetch(&**remote, &path)?, Some(path))?,
                None => self.enter_archive(path, None)?,
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// Cursor lands on the folder or archive it leaves.
    pub fn up(&mut self) -> io::Result<()> {
        if let Some(archive) = &self.archive
            && let Some(parent) = archive.dir.parent()
        {
            let left = archive.dir.file_name().map(OsString::from);
            self.show_archive_dir(parent.to_owned(), left);
            return Ok(());
        }
        let here = self.archive_path();
        let Some(parent) = here.parent() else {
            return Ok(());
        };
        let left = here.file_name().map(OsString::from);
        self.go_to(parent.to_owned(), left)
    }

    /// Reads the folder or archive again; the cursor stays on its entry.
    pub fn reload(&mut self) -> io::Result<()> {
        let select = self.entries.get(self.cursor).map(|e| e.name.clone());
        let Some(archive) = &self.archive else {
            return self.go_to(self.path.clone(), select);
        };
        let (dir, origin) = (archive.dir.clone(), archive.origin.clone());
        let items = archive::list(&self.path)?;
        self.archive = Some(Archive {
            items,
            dir: PathBuf::new(),
            origin,
        });
        self.show_archive_dir(dir, select);
        Ok(())
    }

    fn enter_archive(&mut self, path: PathBuf, origin: Option<PathBuf>) -> io::Result<()> {
        // shortcut: lists on the UI thread; move to the background if big archives freeze it.
        let items = archive::list(&path)?;
        self.marked.clear();
        self.measured.clear();
        self.path = path;
        self.archive = Some(Archive {
            items,
            dir: PathBuf::new(),
            origin,
        });
        self.show_archive_dir(PathBuf::new(), None);
        Ok(())
    }

    // Read first: a failed move leaves the panel as it was.
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// The cursor stays on its entry.
    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        let select = self.current_entry().map(|e| e.name.clone());
        let entries = std::mem::take(&mut self.entries);
        self.show(entries, select);
    }

    pub fn in_archive(&self) -> bool {
        self.archive.is_some()
    }

    /// Inside an archive, the file at the cursor.
    pub fn archive_member(&self) -> Option<archive::Member> {
        let archive = self.archive.as_ref()?;
        let entry = self.current_entry().filter(|entry| !entry.is_dir)?;
        let path = archive.dir.join(&entry.name);
        let item = archive.items.iter().find(|item| item.path == path)?;
        Some(archive::Member {
            archive: self.path.clone(),
            listed: item.listed.clone(),
            name: entry.name.clone(),
            inside: item.path.clone(),
        })
    }

    /// An archive taken from a server: the server, and where it is there.
    pub fn archive_origin(&self) -> Option<(Arc<dyn Remote>, PathBuf)> {
        let origin = self.archive.as_ref()?.origin.clone()?;
        Some((self.remote.clone()?, origin))
    }

    /// The file at the cursor, for viewers and editors.
    pub fn origin(&self) -> Option<origin::Origin> {
        let entry = self.current_entry().filter(|entry| !entry.is_dir)?;
        if self.archive.is_some() {
            let member = self.archive_member()?;
            return Some(origin::Origin::Archive(member, self.archive_origin()));
        }
        let path = self.folder().join(&entry.name);
        Some(match self.remote {
            Some(_) => origin::Origin::Server(self.fs(), path),
            None => origin::Origin::Local(path),
        })
    }

    /// The folder's files, and which is at the cursor: the viewer's Next and Previous.
    pub fn files_around(&self) -> (Vec<PathBuf>, usize) {
        let files = self.entries.iter().filter(|entry| !entry.is_dir);
        let files: Vec<PathBuf> = files.map(|entry| self.folder().join(&entry.name)).collect();
        let at = self
            .current_entry()
            .map(|entry| self.folder().join(&entry.name));
        let at = files.iter().position(|file| Some(file) == at.as_ref());
        (files, at.unwrap_or(0))
    }

    /// Inside an archive: the chosen entries taken out with their folders
    /// into a temporary folder, for F5 to copy from there. Blocks.
    pub fn extract_chosen(&self) -> io::Result<Vec<PathBuf>> {
        let Some(archive) = &self.archive else {
            return Err(io::Error::other("not in an archive"));
        };
        let paths: Vec<PathBuf> = self
            .chosen()
            .iter()
            .map(|e| archive.dir.join(&e.name))
            .collect();
        let listed = paths.iter().filter_map(|path| {
            let item = archive.items.iter().find(|item| item.path == *path)?;
            Some(item.listed.as_str())
        });
        let listed: Vec<&str> = listed.collect();
        let dir = archive::fresh_scratch()?;
        archive::extract(&self.path, &listed, &dir, true)?;
        Ok(paths.iter().map(|path| dir.join(path)).collect())
    }

    /// A file's size, or a folder's once measured.
    pub fn measured_size(&self, entry: &Entry) -> Option<u64> {
        match entry.is_dir {
            true => self.measured.get(&entry.name).copied(),
            false => Some(entry.size),
        }
    }

    /// The Size column: a measured folder's size in place of `<DIR>`.
    pub fn size_text(&self, entry: &Entry) -> String {
        match (entry.is_dir, self.measured.get(&entry.name)) {
            (true, Some(&bytes)) => grouped(bytes),
            _ => entry.size_text(),
        }
    }

    /// Calculate Occupied Space found `bytes` in the folder `name`.
    pub fn set_measured(&mut self, name: OsString, bytes: u64) {
        self.measured.insert(name, bytes);
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.marked.contains(&entry.name)
    }

    /// Insert and Space: the entry at the cursor, then the next one.
    pub fn toggle_mark(&mut self) {
        if let Some(entry) = self.current_entry().filter(|e| !e.is_parent()) {
            let name = entry.name.clone();
            if !self.marked.remove(&name) {
                self.marked.insert(name);
            }
        }
        self.move_cursor(1);
    }

    /// All but `..`, or none.
    pub fn mark_all(&mut self, marked: bool) {
        self.marked.clear();
        if marked {
            let names = self.entries.iter().filter(|e| !e.is_parent());
            self.marked.extend(names.map(|e| e.name.clone()));
        }
    }

    pub fn invert_marks(&mut self) {
        let names = self.entries.iter().filter(|e| !e.is_parent());
        let inverted = names.filter(|e| !self.marked.contains(&e.name));
        self.marked = inverted.map(|e| e.name.clone()).collect();
    }

    /// What an operation works on, as in Total Commander: the marked
    /// entries, or else the one at the cursor.
    pub fn chosen(&self) -> Vec<&Entry> {
        let marked: Vec<&Entry> = self.entries.iter().filter(|e| self.is_marked(e)).collect();
        match marked.is_empty() {
            true => self
                .current_entry()
                .filter(|e| !e.is_parent())
                .into_iter()
                .collect(),
            false => marked,
        }
    }

    /// `name` may be a path; the cursor goes to its first folder here.
    pub fn new_folder(&mut self, name: &str) -> io::Result<()> {
        self.on_disk()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "no name given"));
        }
        let target = self.path.join(name);
        let fs = self.fs();
        if fs.metadata(&target).is_ok() {
            let err = format!("{name} exists already");
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, err));
        }
        vfs::create_all(&*fs, &target)?;
        let first = Path::new(name)
            .components()
            .next()
            .map(|c| c.as_os_str().to_owned());
        self.go_to(self.path.clone(), first)
    }

    /// The chosen entries, to the trash or for good; stops at the first error.
    pub fn delete(&mut self, to_trash: bool) -> io::Result<()> {
        self.on_disk()?;
        let paths: Vec<PathBuf> = self
            .chosen()
            .iter()
            .map(|e| self.path.join(&e.name))
            .collect();
        let deleted = paths.iter().try_for_each(|path| {
            if let Some(remote) = &self.remote {
                // A server has no trash.
                vfs::remove_all(&**remote, path)
            } else if to_trash {
                trash::delete(path).map_err(io::Error::other)
            } else if fs::symlink_metadata(path)?.is_dir() {
                fs::remove_dir_all(path)
            } else {
                // A link goes, never what it points to.
                fs::remove_file(path)
            }
        });
        let cursor = self.cursor;
        self.go_to(self.path.clone(), None)?;
        self.set_cursor(cursor);
        deleted
    }

    // shortcut: files change only on disk; add archive updates (7-Zip `a`, `d`) when needed.
    fn on_disk(&self) -> io::Result<()> {
        match self.archive {
            Some(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "not inside archives yet",
            )),
            None => Ok(()),
        }
    }

    pub fn go_to_folder(&mut self, path: PathBuf) -> io::Result<()> {
        self.go_to(path, None)
    }

    /// The folder, local or on the server: inside an archive, the archive's.
    pub fn folder(&self) -> &Path {
        match self.archive {
            Some(_) => {
                let archive = self.archive_path();
                archive.parent().unwrap_or(archive)
            }
            None => &self.path,
        }
    }

    /// As Total Commander's status bar: marked / all.
    /// Measured folders count with their contents.
    pub fn status_text(&self) -> String {
        let listed = self.entries.iter().filter(|e| !e.is_parent());
        let (dirs, files): (Vec<&Entry>, Vec<&Entry>) = listed.partition(|e| e.is_dir);
        let marked = |list: &[&Entry]| list.iter().filter(|e| self.is_marked(e)).count();
        let all = || files.iter().chain(&dirs);
        let bytes = |list: &mut dyn Iterator<Item = &&Entry>| {
            list.filter_map(|e| self.measured_size(e)).sum::<u64>() / 1024
        };
        format!(
            "{} k / {} k in {} / {} file(s), {} / {} dir(s)",
            grouped(bytes(&mut all().filter(|e| self.is_marked(e)))),
            grouped(bytes(&mut all())),
            marked(&files),
            files.len(),
            marked(&dirs),
            dirs.len()
        )
    }

    fn go_to(&mut self, path: PathBuf, select: Option<OsString>) -> io::Result<()> {
        let entries = match &self.remote {
            Some(remote) => remote.entries(&path)?,
            None => read_dir(&path)?,
        };
        if path != self.path || self.archive.is_some() {
            self.marked.clear();
            self.measured.clear();
        }
        self.archive = None;
        self.path = path;
        self.show(entries, select);
        Ok(())
    }

    fn show_archive_dir(&mut self, dir: PathBuf, select: Option<OsString>) {
        let archive = self.archive.as_mut().expect("in an archive");
        let entries = archive::entries(&archive.items, &dir);
        if archive.dir != dir {
            self.marked.clear();
            self.measured.clear();
        }
        let archive = self.archive.as_mut().expect("in an archive");
        archive.dir = dir;
        self.show(entries, select);
    }

    fn show(&mut self, mut entries: Vec<Entry>, select: Option<OsString>) {
        entries.retain(|entry| self.filter.shows(entry));
        // Marks stay on what is still there.
        let names: HashSet<&OsString> = entries.iter().map(|e| &e.name).collect();
        self.marked.retain(|name| names.contains(name));
        entries.sort_by(|a, b| self.sort.compare(a, b));
        self.cursor = select
            .and_then(|name| entries.iter().position(|e| e.name == name))
            .unwrap_or(0);
        self.entries = entries;
    }

    /// Neither locked nor named.
    fn plain_copy(&self) -> Self {
        Self {
            locked: false,
            name: None,
            ..self.clone()
        }
    }
}

const CLOSED_TABS: usize = 10;

/// A side's folder tabs; there is always one.
pub struct Tabs {
    panels: Vec<Panel>,
    active: usize,
    /// Most recent first.
    closed: Vec<Panel>,
}

impl Tabs {
    pub fn new(panel: Panel) -> Self {
        Self {
            panels: vec![panel],
            active: 0,
            closed: Vec::new(),
        }
    }

    /// Tabs that cannot be opened any more are left out; with none, `fallback`.
    pub fn restore(state: &SideState, filter: Filter, fallback: impl FnOnce() -> Panel) -> Self {
        let mut panels: Vec<Panel> = state
            .tabs
            .iter()
            .filter_map(|tab| Panel::restore(tab, filter).ok())
            .collect();
        if panels.is_empty() {
            panels.push(fallback());
        }
        Self {
            active: state.active.min(panels.len() - 1),
            panels,
            closed: Vec::new(),
        }
    }

    pub fn state(&self) -> SideState {
        SideState {
            tabs: self.panels.iter().map(Panel::state).collect(),
            active: self.active,
        }
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn current(&self) -> &Panel {
        &self.panels[self.active]
    }

    pub fn current_mut(&mut self) -> &mut Panel {
        &mut self.panels[self.active]
    }

    pub fn closed(&self) -> &[Panel] {
        &self.closed
    }

    pub fn duplicate(&mut self) {
        let copy = self.current().plain_copy();
        self.add(copy);
    }

    /// A new last tab, which becomes current.
    pub fn add(&mut self, panel: Panel) {
        self.panels.push(panel);
        self.active = self.panels.len() - 1;
    }

    pub fn select(&mut self, ix: usize) {
        if ix < self.panels.len() {
            self.active = ix;
        }
    }

    /// The next (or with -1 the previous) tab, round from the end.
    pub fn step(&mut self, by: isize) {
        let count = self.panels.len() as isize;
        self.active = (self.active as isize + by).rem_euclid(count) as usize;
    }

    /// Moves in the current tab; from a locked tab the move opens a new tab.
    pub fn navigate(&mut self, go: impl FnOnce(&mut Panel) -> io::Result<()>) -> io::Result<()> {
        if !self.current().locked {
            return go(self.current_mut());
        }
        let mut moved = self.current().plain_copy();
        go(&mut moved)?;
        if moved.path() != self.current().path() {
            self.add(moved);
        }
        Ok(())
    }

    /// A move made on a copy of tab `ix` elsewhere, as for a server: the
    /// tab takes it, or a locked one gets a new tab beside it.
    pub fn navigated(&mut self, ix: usize, moved: Panel) {
        let Some(panel) = self.panels.get(ix) else {
            return;
        };
        if !panel.locked {
            self.panels[ix] = moved;
        } else if moved.path() != panel.path() || moved.location() != panel.location() {
            self.add(moved.plain_copy());
        }
    }

    /// Tabs showing `folder` read it again.
    pub fn reload_folder(&mut self, folder: &Path) {
        for panel in &mut self.panels {
            if panel.archive.is_none() && panel.remote.is_none() && panel.path == folder {
                // shortcut: a tab that cannot read its folder keeps its old list.
                let _ = panel.reload();
            }
        }
    }

    /// Every tab, closed ones too, lists again by `filter`.
    pub fn set_filter(&mut self, filter: Filter) {
        for panel in self.panels.iter_mut().chain(&mut self.closed) {
            panel.filter = filter;
            // shortcut: a tab whose folder is gone keeps its old list until it moves.
            let _ = panel.reload();
        }
    }

    pub fn toggle_lock(&mut self) {
        let panel = self.current_mut();
        panel.locked = !panel.locked;
    }

    /// An empty name gives the tab its folder's name again.
    pub fn rename(&mut self, ix: usize, name: &str) {
        if let Some(panel) = self.panels.get_mut(ix) {
            let name = name.trim();
            panel.name = (!name.is_empty()).then(|| name.to_owned());
        }
    }

    /// The last tab stays.
    pub fn close(&mut self) {
        if self.panels.len() > 1 {
            let panel = self.panels.remove(self.active);
            self.remember(panel);
            self.active = self.active.min(self.panels.len() - 1);
        }
    }

    /// All but the current and the locked tabs.
    pub fn close_all(&mut self) {
        self.close_where(|panel| !panel.locked);
    }

    /// Tabs on a folder that a kept tab shows too; current and locked tabs stay.
    pub fn close_duplicates(&mut self) {
        let mut shown: HashSet<PathBuf> = self
            .panels
            .iter()
            .enumerate()
            .filter(|(ix, panel)| *ix == self.active || panel.locked)
            .map(|(_, panel)| panel.path())
            .collect();
        self.close_where(|panel| !panel.locked && !shown.insert(panel.path()));
    }

    /// In tab order; never the current tab.
    fn close_where(&mut self, mut close: impl FnMut(&Panel) -> bool) {
        let mut closed = Vec::new();
        let active = self.active;
        for (ix, panel) in std::mem::take(&mut self.panels).into_iter().enumerate() {
            if ix != active && close(&panel) {
                closed.push(panel);
            } else {
                if ix == active {
                    self.active = self.panels.len();
                }
                self.panels.push(panel);
            }
        }
        for panel in closed {
            self.remember(panel);
        }
    }

    fn remember(&mut self, panel: Panel) {
        self.closed.insert(0, panel);
        self.closed.truncate(CLOSED_TABS);
    }

    /// A recently closed tab again, its folder read anew.
    pub fn reopen(&mut self, ix: usize) -> io::Result<()> {
        if ix < self.closed.len() {
            let mut panel = self.closed.remove(ix);
            panel.reload()?;
            self.add(panel);
        }
        Ok(())
    }
}

/// Unsorted, `..` first but in the root.
fn read_dir(path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = fs::read_dir(path)?
        // Skip unreadable entries, not the whole folder.
        .filter_map(Result::ok)
        .map(|item| {
            // Follow links; a broken link stays itself.
            let meta = fs::metadata(item.path()).or_else(|_| item.metadata()).ok();
            let name = item.file_name();
            let (hidden, system, attributes) = attributes(&name, meta.as_ref());
            Entry {
                name,
                is_dir: meta.as_ref().is_some_and(fs::Metadata::is_dir),
                size: meta.as_ref().map_or(0, fs::Metadata::len),
                modified: meta.and_then(|m| m.modified().ok()),
                hidden,
                system,
                attributes,
            }
        })
        .collect();
    if path.parent().is_some() {
        let modified = fs::metadata(path.join(PARENT))
            .and_then(|m| m.modified())
            .ok();
        entries.insert(0, parent_entry(modified));
    }
    Ok(entries)
}

/// Thousands apart: `1 234 567`.
pub fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut text = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(' ');
        }
        text.push(digit);
    }
    text
}

fn parent_entry(modified: Option<SystemTime>) -> Entry {
    Entry {
        name: PARENT.into(),
        is_dir: true,
        size: 0,
        modified,
        hidden: false,
        system: false,
        attributes: String::new(),
    }
}

/// Hidden, system, and the Attr column's text.
#[cfg(windows)]
fn attributes(_: &OsStr, meta: Option<&fs::Metadata>) -> (bool, bool, String) {
    use std::os::windows::fs::MetadataExt;
    const READ_ONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const ARCHIVE: u32 = 0x20;
    let bits = meta.map_or(0, MetadataExt::file_attributes);
    let flag = |bit, letter| if bits & bit != 0 { letter } else { '-' };
    let text = [
        flag(READ_ONLY, 'r'),
        flag(ARCHIVE, 'a'),
        flag(HIDDEN, 'h'),
        flag(SYSTEM, 's'),
    ];
    (
        bits & HIDDEN != 0,
        bits & SYSTEM != 0,
        text.iter().collect(),
    )
}

/// Hidden, system, and the Attr column's text.
#[cfg(unix)]
fn attributes(name: &OsStr, meta: Option<&fs::Metadata>) -> (bool, bool, String) {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.map_or(0, |meta| meta.permissions().mode());
    let text = (0..9)
        .map(|i| match mode & (0o400 >> i) {
            0 => '-',
            _ => b"rwxrwxrwx"[i] as char,
        })
        .collect();
    (dot_file(name), false, text)
}

pub(crate) fn dot_file(name: &OsStr) -> bool {
    cfg!(not(windows)) && name.as_encoded_bytes().first() == Some(&b'.')
}

fn by_name(a: &Entry, b: &Entry) -> Ordering {
    let key = |e: &Entry| e.name.to_string_lossy().to_lowercase();
    key(a).cmp(&key(b)).then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigates() {
        let root = std::env::temp_dir().join(format!("pc-core-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("b-dir/inner")).unwrap();
        fs::create_dir(root.join("A-dir")).unwrap();
        fs::write(root.join("a-file"), "12345").unwrap();

        let mut panel = Panel::new(&root, Filter::default()).unwrap();
        let names: Vec<_> = panel.entries().iter().map(Entry::display_name).collect();
        assert_eq!(names, ["..", "A-dir", "b-dir", "a-file"]);
        assert_eq!(panel.entries()[3].size_text(), "5");
        assert_eq!(
            panel.status_text(),
            "0 k / 0 k in 0 / 1 file(s), 0 / 2 dir(s)"
        );

        panel.set_cursor(99);
        assert_eq!(panel.cursor(), 3);
        assert!(!panel.open().unwrap(), "a file is not entered");

        panel.move_cursor(-1);
        assert!(panel.open().unwrap());
        assert_eq!(panel.path(), root.join("b-dir"));

        panel.set_cursor(0);
        assert!(panel.open().unwrap(), "`..` goes up");
        assert_eq!(panel.path(), root);
        assert_eq!(panel.cursor(), 2, "on the folder it left");

        fs::remove_dir_all(&root).unwrap();
        panel.move_cursor(-1);
        assert!(panel.open().is_err());
        assert_eq!(panel.path(), root, "stays where it was");
    }

    #[test]
    fn opens_tabs() {
        let mut tabs = Tabs::new(Panel::new("/", Filter::default()).unwrap());
        assert_eq!(tabs.current().title(), "/");
        tabs.current_mut().set_cursor(1);
        tabs.duplicate();
        assert_eq!((tabs.panels().len(), tabs.active()), (2, 1));
        assert_eq!(tabs.current().cursor(), 1, "a copy of the current tab");
        tabs.current_mut().open().unwrap();
        assert_ne!(tabs.current().path(), tabs.panels()[0].path());
        tabs.select(5);
        assert_eq!(tabs.active(), 1);
        tabs.select(0);
        assert_eq!(tabs.current().path(), Path::new("/"));
        tabs.step(-1);
        assert_eq!(tabs.active(), 1, "round from the first to the last");
        tabs.step(1);
        assert_eq!(tabs.active(), 0);
    }

    #[test]
    fn locks_and_closes_tabs() {
        let root = std::env::temp_dir().join(format!("pc-tabs-test-{}", std::process::id()));
        fs::create_dir_all(root.join("sub")).unwrap();
        let mut tabs = Tabs::new(Panel::new(&root, Filter::default()).unwrap());
        tabs.toggle_lock();
        assert!(tabs.current().title().starts_with("*pc-tabs-test-"));
        tabs.current_mut().set_cursor(1);
        tabs.navigate(|panel| panel.open().map(drop)).unwrap();
        assert_eq!(
            (tabs.panels().len(), tabs.active()),
            (2, 1),
            "moved in a new tab"
        );
        assert_eq!(tabs.panels()[0].path(), root, "the locked tab stayed");
        assert_eq!(tabs.current().path(), root.join("sub"));
        assert!(!tabs.current().is_locked());

        tabs.duplicate();
        tabs.duplicate();
        tabs.select(2);
        tabs.close_all();
        assert_eq!(
            (tabs.panels().len(), tabs.active()),
            (2, 1),
            "the locked and the current"
        );
        tabs.close();
        tabs.close();
        assert_eq!(tabs.panels().len(), 1, "the last stays");
        assert!(tabs.current().is_locked());

        assert_eq!(tabs.closed().len(), 3, "the closed tabs, most recent first");
        tabs.reopen(0).unwrap();
        assert_eq!((tabs.panels().len(), tabs.active()), (2, 1));
        tabs.duplicate();
        tabs.select(0);
        tabs.close_duplicates();
        assert_eq!(tabs.panels().len(), 2, "one of the two on the same folder");

        tabs.rename(0, " Work ");
        assert_eq!(tabs.panels()[0].title(), "*Work");
        let restored = Tabs::restore(&tabs.state(), Filter::default(), || unreachable!());
        assert_eq!(restored.panels()[0].title(), "*Work");
        assert_eq!(restored.panels()[1].path(), tabs.panels()[1].path());
        tabs.rename(0, "");
        assert!(tabs.panels()[0].title().starts_with("*pc-tabs-test-"));

        fs::remove_dir_all(root.join("sub")).unwrap();
        let gone = Tabs::restore(&tabs.state(), Filter::default(), || unreachable!());
        assert_eq!(gone.panels()[1].path(), root, "the nearest folder left");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(not(windows))]
    fn hides_dot_files() {
        let root = std::env::temp_dir().join(format!("pc-filter-test-{}", std::process::id()));
        fs::create_dir_all(root.join(".hidden-dir")).unwrap();
        fs::write(root.join("shown"), "").unwrap();
        let mut tabs = Tabs::new(Panel::new(&root, Filter::default()).unwrap());
        let names = |tabs: &Tabs| -> Vec<String> {
            tabs.current()
                .entries()
                .iter()
                .map(Entry::display_name)
                .collect()
        };
        assert_eq!(names(&tabs), ["..", "shown"]);
        tabs.set_filter(Filter {
            hidden: true,
            system: false,
        });
        assert_eq!(names(&tabs), ["..", ".hidden-dir", "shown"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sorts_creates_and_deletes() {
        let root = std::env::temp_dir().join(format!("pc-files-test-{}", std::process::id()));
        fs::create_dir_all(root.join("z-dir")).unwrap();
        fs::write(root.join("b.txt"), "12345").unwrap();
        fs::write(root.join("a.rs"), "1").unwrap();
        let mut panel = Panel::new(&root, Filter::default()).unwrap();
        let names = |panel: &Panel| -> Vec<String> {
            panel.entries().iter().map(Entry::display_name).collect()
        };
        assert_eq!(names(&panel), ["..", "z-dir", "a.rs", "b.txt"]);
        panel.set_cursor(2);
        panel.set_sort(Sort::by(SortKey::Size));
        assert_eq!(
            names(&panel),
            ["..", "z-dir", "b.txt", "a.rs"],
            "largest first"
        );
        assert_eq!(
            panel.current_entry().unwrap().display_name(),
            "a.rs",
            "cursor kept"
        );
        panel.set_sort(Sort {
            key: SortKey::Extension,
            descending: true,
        });
        assert_eq!(names(&panel), ["..", "z-dir", "b.txt", "a.rs"]);

        panel.new_folder("new/inner").unwrap();
        assert_eq!(panel.current_entry().unwrap().display_name(), "new");
        assert!(root.join("new/inner").is_dir());
        assert!(panel.new_folder("new").is_err(), "exists already");
        panel.delete(false).unwrap();
        assert!(!root.join("new").exists());
        assert_eq!(names(&panel), ["..", "z-dir", "b.txt", "a.rs"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn marks_what_operations_work_on() {
        let root = std::env::temp_dir().join(format!("pc-marks-test-{}", std::process::id()));
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("a.txt"), vec![0; 2048]).unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        let mut panel = Panel::new(&root, Filter::default()).unwrap();
        let chosen = |panel: &Panel| -> Vec<String> {
            panel.chosen().iter().map(|e| e.display_name()).collect()
        };
        assert!(chosen(&panel).is_empty(), "not ..");
        panel.toggle_mark();
        assert_eq!(panel.cursor(), 1, "not .. but the cursor moves on");
        panel.set_cursor(2);
        assert_eq!(chosen(&panel), ["a.txt"], "the cursor's, none marked");
        panel.toggle_mark();
        panel.toggle_mark();
        assert_eq!(chosen(&panel), ["a.txt", "b.txt"]);
        assert_eq!(
            panel.status_text(),
            "2 k / 2 k in 2 / 2 file(s), 0 / 1 dir(s)"
        );
        panel.reload().unwrap();
        assert_eq!(chosen(&panel).len(), 2, "kept on reload");
        panel.invert_marks();
        assert_eq!(chosen(&panel), ["dir"]);
        panel.mark_all(true);
        assert_eq!(chosen(&panel).len(), 3);
        panel.delete(false).unwrap();
        assert_eq!(panel.entries().len(), 1, "only .. left");
        fs::create_dir(root.join("again")).unwrap();
        panel.reload().unwrap();
        panel.mark_all(true);
        panel.set_cursor(1);
        panel.open().unwrap();
        assert!(chosen(&panel).is_empty(), "a new folder, no marks");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn splits_names_and_shows_attributes() {
        let entry = |name: &str, is_dir| Entry {
            name: name.into(),
            is_dir,
            size: 0,
            modified: None,
            hidden: false,
            system: false,
            attributes: String::new(),
        };
        let parts = |name, is_dir| entry(name, is_dir).name_and_extension();
        assert_eq!(
            parts("backup.tar.gz", false),
            ("backup.tar".into(), "gz".into())
        );
        assert_eq!(parts(".bashrc", false), (".bashrc".into(), String::new()));
        assert_eq!(
            parts("v1.2", true),
            ("v1.2".into(), String::new()),
            "folders whole"
        );
        assert_eq!(
            crate::archive::tests::attributes_of(" -rw-r--r--"),
            "rw-r--r--"
        );
        assert_eq!(crate::archive::tests::attributes_of("RA"), "ra--");
        #[cfg(unix)]
        {
            let root = std::env::temp_dir().join(format!("pc-attr-test-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let file = root.join("f");
            fs::write(&file, "").unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
            let panel = Panel::new(&root, Filter::default()).unwrap();
            assert_eq!(panel.entries()[1].attributes, "rw-r-----");
            fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn shows_sizes_for_people() {
        let sizes = [1023, 1536, 5 << 20, 3 << 30].map(human_size);
        assert_eq!(sizes, ["1023 B", "1.5 KB", "5.0 MB", "3.00 GB"]);
    }

    #[test]
    fn separates_thousands() {
        let entry = |size| Entry {
            name: "f".into(),
            is_dir: false,
            size,
            modified: None,
            hidden: false,
            system: false,
            attributes: String::new(),
        };
        assert_eq!(entry(1234567).size_text(), "1 234 567");
        assert_eq!(entry(123456).size_text(), "123 456");
        assert_eq!(entry(0).size_text(), "0");
    }
}
