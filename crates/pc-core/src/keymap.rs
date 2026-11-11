//! The commands' keys: a preset's, with the user's changes. Keys are in
//! GPUI's syntax (`ctrl-shift-f4`, a chord as `ctrl-x d`); `secondary` is
//! Cmd on macOS and Ctrl elsewhere; numpad keys are `add`, `subtract`,
//! `multiply`, `divide`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::menu::Command::{self, *};

/// Whose keys to start from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// Total Commander's, where no system takes them; keys on every keyboard.
    #[default]
    PowerCommander,
    TotalCommander,
    Krusader,
    MidnightCommander,
    /// ForkLift 4's, for macOS.
    ForkLift,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::PowerCommander,
        Preset::TotalCommander,
        Preset::Krusader,
        Preset::MidnightCommander,
        Preset::ForkLift,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::PowerCommander => "PowerCommander",
            Preset::TotalCommander => "Total Commander",
            Preset::Krusader => "Krusader",
            Preset::MidnightCommander => "Midnight Commander",
            Preset::ForkLift => "ForkLift",
        }
    }

    fn table(self) -> Table {
        match self {
            Preset::PowerCommander => POWER_COMMANDER,
            Preset::TotalCommander => TOTAL_COMMANDER,
            Preset::Krusader => KRUSADER,
            Preset::MidnightCommander => MIDNIGHT_COMMANDER,
            Preset::ForkLift => FORKLIFT,
        }
    }
}

/// A command's shortcut and alternate; "" for none.
type Table = &'static [(Command, &'static str, &'static str)];

const POWER_COMMANDER: Table = &[
    (View, "f3", ""),
    (Edit, "f4", ""),
    (Copy, "f5", ""),
    (Move, "f6", ""),
    (NewFolder, "f7", ""),
    (Delete, "f8", "delete"),
    (DeletePermanently, "shift-f8", "shift-delete"),
    (ToggleMark, "space", ""),
    (Quit, "secondary-q", "alt-f4"),
    (Pack, "alt-shift-p", ""),
    (UnpackSpecificFiles, "alt-f9", "alt-shift-u"),
    (TestArchives, "alt-shift-f9", "alt-shift-e"),
    (Properties, "alt-enter", "secondary-i"),
    (EditExternally, "ctrl-shift-f4", ""),
    (CalculateOccupiedSpace, "ctrl-l", ""),
    (MultiRenameTool, "ctrl-m", ""),
    (SelectGroup, "alt-shift-s", ""),
    (UnselectGroup, "alt-shift-r", ""),
    (SelectAll, "secondary-a", ""),
    (UnselectAll, "secondary-shift-a", ""),
    (InvertSelection, "alt-shift-i", ""),
    (SelectAllWithSameExtension, "alt-shift-x", ""),
    (RestoreSelection, "alt-shift-z", ""),
    (CompareDirectories, "shift-f2", ""),
    (CopyToClipboard, "secondary-c", ""),
    (CutToClipboard, "secondary-x", ""),
    (PasteFromClipboard, "secondary-v", ""),
    (CdTree, "alt-shift-f10", ""),
    (Search, "secondary-f", ""),
    (SearchInSeparateProcess, "secondary-shift-f", ""),
    (DirectoryHotlist, "secondary-d", ""),
    (GoBack, "alt-left", ""),
    (BranchView, "ctrl-b", ""),
    (SwapPanels, "ctrl-u", ""),
    (ConnectToServer, "secondary-k", ""),
    (Disconnect, "secondary-shift-k", ""),
    (Brief, "alt-shift-b", ""),
    (Full, "alt-shift-d", ""),
    (Comments, "ctrl-shift-f2", ""),
    (ThumbnailView, "ctrl-shift-f1", ""),
    (QuickViewPanel, "alt-shift-q", ""),
    (DuplicateTab, "secondary-t", ""),
    (AllFiles, "ctrl-shift-f10", ""),
    (Programs, "ctrl-shift-f11", ""),
    (CustomFilter, "ctrl-shift-f12", ""),
    (SortByName, "secondary-1", ""),
    (SortByExtension, "secondary-2", ""),
    (SortByTime, "secondary-3", ""),
    (SortBySize, "secondary-4", ""),
    (Unsorted, "secondary-5", ""),
    (RereadSource, "secondary-r", ""),
    (Settings, "secondary-,", ""),
    (CloseTab, "secondary-w", ""),
    (CloseAllTabs, "secondary-shift-w", ""),
    (ListTabs, "secondary-shift-l", ""),
    (NextTab, "ctrl-tab", "alt-shift-right"),
    (PreviousTab, "ctrl-shift-tab", "alt-shift-left"),
];

const TOTAL_COMMANDER: Table = &[
    (View, "f3", ""),
    (Edit, "f4", ""),
    (Copy, "f5", ""),
    (Move, "f6", ""),
    (NewFolder, "f7", ""),
    (Delete, "f8", "delete"),
    (DeletePermanently, "shift-f8", "shift-delete"),
    (ToggleMark, "insert", "space"),
    (Quit, "alt-f4", ""),
    (Pack, "alt-f5", ""),
    (UnpackSpecificFiles, "alt-f9", ""),
    (TestArchives, "alt-shift-f9", ""),
    (Properties, "alt-enter", ""),
    (EditExternally, "ctrl-shift-f4", ""),
    (CalculateOccupiedSpace, "ctrl-l", ""),
    (MultiRenameTool, "ctrl-m", ""),
    (SelectGroup, "add", ""),
    (UnselectGroup, "subtract", ""),
    (SelectAll, "ctrl-add", "ctrl-a"),
    (UnselectAll, "ctrl-subtract", ""),
    (InvertSelection, "multiply", ""),
    (SelectAllWithSameExtension, "alt-add", ""),
    (RestoreSelection, "divide", ""),
    (CompareDirectories, "shift-f2", ""),
    (CopyToClipboard, "ctrl-c", ""),
    (CutToClipboard, "ctrl-x", ""),
    (PasteFromClipboard, "ctrl-v", ""),
    (CdTree, "alt-f10", ""),
    (Search, "alt-f7", ""),
    (SearchInSeparateProcess, "alt-shift-f7", ""),
    (DirectoryHotlist, "ctrl-d", ""),
    (GoBack, "alt-left", ""),
    (BranchView, "ctrl-b", ""),
    (SwapPanels, "ctrl-u", ""),
    (ConnectToServer, "ctrl-f", ""),
    (Disconnect, "ctrl-shift-f", ""),
    (Brief, "ctrl-f1", ""),
    (Full, "ctrl-f2", ""),
    (Comments, "ctrl-shift-f2", ""),
    (Tree, "ctrl-f8", ""),
    (ThumbnailView, "ctrl-shift-f1", ""),
    (QuickViewPanel, "ctrl-q", ""),
    (DuplicateTab, "ctrl-t", ""),
    (AllFiles, "ctrl-f10", ""),
    (Programs, "ctrl-f11", ""),
    (CustomFilter, "ctrl-f12", ""),
    (SortByName, "ctrl-f3", ""),
    (SortByExtension, "ctrl-f4", ""),
    (SortByTime, "ctrl-f5", ""),
    (SortBySize, "ctrl-f6", ""),
    (Unsorted, "ctrl-f7", ""),
    (RereadSource, "ctrl-r", "f2"),
    (CloseTab, "ctrl-w", ""),
    (CloseAllTabs, "ctrl-shift-w", ""),
    (ListTabs, "ctrl-shift-a", ""),
    (NextTab, "ctrl-tab", "alt-shift-right"),
    (PreviousTab, "ctrl-shift-tab", "alt-shift-left"),
];

/// Krusader's own defaults (its kractions, listpanelactions, viewactions
/// and tabactions) and KDE's standard keys.
const KRUSADER: Table = &[
    (View, "f3", ""),
    (Edit, "f4", ""),
    (Copy, "f5", ""),
    (Move, "f6", ""),
    (NewFolder, "f7", ""),
    (Delete, "f8", "delete"),
    (DeletePermanently, "shift-delete", "ctrl-delete"),
    (OpenTerminal, "f9", ""),
    (Quit, "ctrl-q", "f10"),
    (ToggleMark, "insert", "space"),
    (MultiRenameTool, "shift-f2", ""),
    (Pack, "alt-shift-p", ""),
    (UnpackSpecificFiles, "alt-shift-u", ""),
    (TestArchives, "alt-shift-e", ""),
    (Properties, "alt-enter", ""),
    (CompareDirectories, "alt-shift-c", ""),
    (SplitFile, "ctrl-p", ""),
    (Search, "ctrl-s", ""),
    (SynchronizeDirs, "ctrl-y", ""),
    (SwapPanels, "ctrl-u", ""),
    (DirectoryHotlist, "ctrl-d", ""),
    (GoBack, "alt-left", ""),
    (RereadSource, "ctrl-r", ""),
    (ConnectToServer, "ctrl-n", ""),
    (Disconnect, "ctrl-shift-d", ""),
    (SelectGroup, "ctrl-add", ""),
    (UnselectGroup, "ctrl-subtract", ""),
    (SelectAll, "alt-add", "ctrl-a"),
    (UnselectAll, "alt-subtract", ""),
    (InvertSelection, "alt-multiply", ""),
    (AllFiles, "shift-f10", ""),
    (CustomFilter, "shift-f12", ""),
    (Brief, "alt-shift-b", ""),
    (Full, "alt-shift-d", ""),
    (QuickViewPanel, "alt-down", ""),
    (VerticalArrangement, "ctrl-alt-r", ""),
    (SortByName, "ctrl-1", ""),
    (SortByExtension, "ctrl-2", ""),
    (SortBySize, "ctrl-3", ""),
    (SortByTime, "ctrl-4", ""),
    (CopyToClipboard, "ctrl-c", ""),
    (CutToClipboard, "ctrl-x", ""),
    (PasteFromClipboard, "ctrl-v", ""),
    (Settings, "ctrl-shift-,", ""),
    (DuplicateTab, "ctrl-t", ""),
    (CloseTab, "ctrl-w", ""),
    (NextTab, "ctrl-pagedown", ""),
    (PreviousTab, "ctrl-pageup", ""),
];

/// mc.default.keymap's file manager keys; `ctrl-x` starts its chords.
const MIDNIGHT_COMMANDER: Table = &[
    (View, "f3", ""),
    (Edit, "f4", ""),
    (Copy, "f5", ""),
    (Move, "f6", ""),
    (NewFolder, "f7", ""),
    (Delete, "f8", ""),
    (Quit, "f10", ""),
    (ToggleMark, "insert", "ctrl-t"),
    (Search, "alt-?", ""),
    (DirectoryHotlist, "ctrl-\\", ""),
    (RereadSource, "ctrl-r", ""),
    (CalculateOccupiedSpace, "ctrl-space", ""),
    (SwapPanels, "ctrl-u", ""),
    (OpenTerminal, "ctrl-o", ""),
    (SelectGroup, "add", "alt-+"),
    (UnselectGroup, "subtract", "alt--"),
    (InvertSelection, "multiply", "alt-*"),
    (CompareDirectories, "ctrl-x d", ""),
    (ChangeAttributes, "ctrl-x c", ""),
    (QuickViewPanel, "ctrl-x q", ""),
    (GoBack, "alt-y", ""),
    (TargetEqualsSource, "alt-o", ""),
];

/// ForkLift 4's menus and macOS's own keys.
const FORKLIFT: Table = &[
    (View, "f3", "space"),
    (Edit, "f4", ""),
    (Copy, "f5", ""),
    (Move, "f6", ""),
    (NewFolder, "f7", ""),
    (Delete, "f8", "secondary-backspace"),
    (DeletePermanently, "secondary-alt-backspace", ""),
    (Pack, "alt-f5", ""),
    (UnpackSpecificFiles, "secondary-shift-x", ""),
    (SynchronizeDirs, "secondary-alt-shift-s", ""),
    (Quit, "secondary-q", ""),
    (Settings, "secondary-,", ""),
    (Search, "alt-f7", ""),
    (Properties, "secondary-i", ""),
    (OpenTerminal, "secondary-shift-r", ""),
    (CalculateOccupiedSpace, "secondary-shift-s", ""),
    (ConnectToServer, "secondary-k", ""),
    (Disconnect, "secondary-shift-e", ""),
    (DirectoryHotlist, "secondary-alt-f", ""),
    (OpenDesktopFolder, "secondary-shift-d", ""),
    (RereadSource, "f2", ""),
    (GoBack, "secondary-[", ""),
    (SelectAll, "secondary-a", ""),
    (UnselectAll, "secondary-alt-a", ""),
    (SelectGroup, "secondary-s", ""),
    (CopyToClipboard, "secondary-c", ""),
    (CutToClipboard, "secondary-x", ""),
    (PasteFromClipboard, "secondary-v", ""),
    (Full, "secondary-1", ""),
    (Brief, "secondary-2", ""),
    (ThumbnailView, "secondary-3", ""),
    (QuickViewPanel, "secondary-shift-p", ""),
    (TargetEqualsSource, "secondary-shift-c", ""),
    (OperationQueue, "secondary-shift-u", ""),
    (DuplicateTab, "secondary-t", ""),
    (CloseTab, "secondary-w", ""),
    (ListTabs, "secondary-shift-\\", ""),
    (NextTab, "ctrl-tab", ""),
    (PreviousTab, "ctrl-shift-tab", ""),
];

/// The user's choice: a preset, and the commands' keys changed from it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeySettings {
    pub preset: Preset,
    pub changed: BTreeMap<Command, [String; 2]>,
}

impl KeySettings {
    /// The preset's keys of a command.
    fn preset_keys(&self, command: Command) -> [String; 2] {
        let row = self.preset.table().iter().find(|(c, ..)| *c == command);
        row.map_or_else(Default::default, |(_, main, alternate)| {
            [main.to_string(), alternate.to_string()]
        })
    }

    /// A command's shortcut and alternate; "" for none.
    pub fn keys_of(&self, command: Command) -> [String; 2] {
        match self.changed.get(&command) {
            Some(keys) => keys.clone(),
            None => self.preset_keys(command),
        }
    }

    pub fn is_changed(&self, command: Command) -> bool {
        self.changed.contains_key(&command)
    }

    pub fn reset(&mut self, command: Command) {
        self.changed.remove(&command);
    }

    /// Every key and its command, each command's alternate before its shortcut.
    pub fn bindings(&self) -> Vec<(String, Command)> {
        let preset = self.preset.table().iter().map(|(c, ..)| *c);
        let changed = self.changed.keys().filter(|c| !self.preset.table().iter().any(|(p, ..)| p == *c));
        let commands: Vec<Command> = preset.chain(changed.copied()).collect();
        let mut out = Vec::new();
        for command in commands {
            let [main, alternate] = self.keys_of(command);
            for key in [alternate, main] {
                if !key.is_empty() {
                    out.push((key, command));
                }
            }
        }
        out
    }

    /// The command `key` runs, as this platform reads it.
    pub fn command_of(&self, key: &str) -> Option<Command> {
        let key = normalize(key, cfg!(target_os = "macos"));
        let bindings = self.bindings();
        let found = bindings.iter().find(|(bound, _)| normalize(bound, cfg!(target_os = "macos")) == key);
        found.map(|(_, command)| *command)
    }

    /// Puts `key` in a command's slot (0 shortcut, 1 alternate), taking it
    /// from the command that had it, which this returns.
    pub fn set(&mut self, command: Command, slot: usize, key: String) -> Option<Command> {
        let mut taken = None;
        if !key.is_empty() {
            let mac = cfg!(target_os = "macos");
            let wanted = normalize(&key, mac);
            for (bound, other) in self.bindings() {
                if normalize(&bound, mac) != wanted {
                    continue;
                }
                let mut keys = self.keys_of(other);
                keys.iter_mut().filter(|k| **k == bound).for_each(String::clear);
                self.put(other, keys);
                taken = (other != command).then_some(other);
            }
        }
        let mut keys = self.keys_of(command);
        keys[slot] = key;
        self.put(command, keys);
        taken
    }

    /// Changed, unless the preset has these keys.
    fn put(&mut self, command: Command, keys: [String; 2]) {
        match keys == self.preset_keys(command) {
            true => self.changed.remove(&command),
            false => self.changed.insert(command, keys),
        };
    }
}

/// A key the same way however written: modifiers as `ctrl-alt-shift-cmd-`,
/// `secondary` as this platform's (`mac`). The terminal names keys so too.
pub fn normalize(keys: &str, mac: bool) -> String {
    let strokes = keys.split_whitespace().map(|stroke| {
        let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
        let mut rest = stroke;
        while let Some((modifier, after)) = rest.split_once('-') {
            if after.is_empty() {
                break;
            }
            match modifier {
                "ctrl" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "cmd" | "super" | "win" => cmd = true,
                "secondary" if mac => cmd = true,
                "secondary" => ctrl = true,
                _ => break,
            }
            rest = after;
        }
        let modifiers = [(ctrl, "ctrl-"), (alt, "alt-"), (shift, "shift-"), (cmd, "cmd-")];
        let prefix: String = modifiers.iter().filter(|(on, _)| *on).map(|(_, name)| *name).collect();
        prefix + rest
    });
    strokes.collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Desktop {
    Windows,
    Mac,
    /// Every Linux desktop.
    Linux,
    Kde,
    Gnome,
}

/// Keys a system takes before programs see them, or gives a meaning every
/// program keeps; `allows` is the command that may have it all the same.
const RESERVED: &[(&str, Desktop, &str, Option<Command>)] = &[
    ("alt-f4", Desktop::Windows, "Windows: close the window", Some(Quit)),
    ("alt-tab", Desktop::Windows, "Windows: switch windows", None),
    ("alt-space", Desktop::Windows, "Windows: the window menu", None),
    ("alt-escape", Desktop::Windows, "Windows: switch windows", None),
    ("ctrl-escape", Desktop::Windows, "Windows: the Start menu", None),
    ("ctrl-shift-escape", Desktop::Windows, "Windows: Task Manager", None),
    ("ctrl-alt-delete", Desktop::Windows, "Windows: the security screen", None),
    ("cmd-q", Desktop::Mac, "macOS: quit", Some(Quit)),
    ("cmd-w", Desktop::Mac, "macOS: close the window or tab", Some(CloseTab)),
    ("cmd-shift-w", Desktop::Mac, "macOS: close the window", Some(CloseAllTabs)),
    ("cmd-h", Desktop::Mac, "macOS: hide the program", None),
    ("cmd-m", Desktop::Mac, "macOS: minimize", None),
    ("cmd-tab", Desktop::Mac, "macOS: switch programs", None),
    ("cmd-space", Desktop::Mac, "macOS: Spotlight", None),
    ("cmd-`", Desktop::Mac, "macOS: switch windows", None),
    ("cmd-shift-q", Desktop::Mac, "macOS: log out", None),
    ("ctrl-cmd-q", Desktop::Mac, "macOS: lock the screen", None),
    ("alt-cmd-escape", Desktop::Mac, "macOS: force quit", None),
    ("cmd-shift-3", Desktop::Mac, "macOS: screenshot", None),
    ("cmd-shift-4", Desktop::Mac, "macOS: screenshot", None),
    ("cmd-shift-5", Desktop::Mac, "macOS: screenshot", None),
    ("ctrl-space", Desktop::Mac, "macOS: the input source", None),
    ("ctrl-up", Desktop::Mac, "macOS: Mission Control", None),
    ("ctrl-down", Desktop::Mac, "macOS: the program's windows", None),
    ("ctrl-left", Desktop::Mac, "macOS: the space to the left", None),
    ("ctrl-right", Desktop::Mac, "macOS: the space to the right", None),
    ("ctrl-f1", Desktop::Mac, "macOS: keyboard access", None),
    ("ctrl-f2", Desktop::Mac, "macOS: the menu bar", None),
    ("ctrl-f3", Desktop::Mac, "macOS: the Dock", None),
    ("ctrl-f4", Desktop::Mac, "macOS: the next window", None),
    ("ctrl-f5", Desktop::Mac, "macOS: the toolbar", None),
    ("ctrl-f6", Desktop::Mac, "macOS: the floating window", None),
    ("ctrl-f7", Desktop::Mac, "macOS: keyboard navigation", None),
    ("ctrl-f8", Desktop::Mac, "macOS: the status menus", None),
    ("f11", Desktop::Mac, "macOS: show the desktop", None),
    ("ctrl-f1", Desktop::Kde, "KDE: switch to desktop 1", None),
    ("ctrl-f2", Desktop::Kde, "KDE: switch to desktop 2", None),
    ("ctrl-f3", Desktop::Kde, "KDE: switch to desktop 3", None),
    ("ctrl-f4", Desktop::Kde, "KDE: switch to desktop 4", None),
    ("ctrl-f7", Desktop::Kde, "KDE: Present Windows (window class)", None),
    ("ctrl-f9", Desktop::Kde, "KDE: Present Windows", None),
    ("ctrl-f10", Desktop::Kde, "KDE: Present Windows (all desktops)", None),
    ("ctrl-f12", Desktop::Kde, "KDE: show the desktop", None),
    ("alt-f1", Desktop::Kde, "KDE: the application launcher", None),
    ("alt-f2", Desktop::Kde, "KDE: KRunner", None),
    ("alt-space", Desktop::Kde, "KDE: KRunner", None),
    ("alt-f3", Desktop::Kde, "KDE: the window menu", None),
    ("alt-f4", Desktop::Kde, "KDE: close the window", Some(Quit)),
    ("alt-tab", Desktop::Kde, "KDE: switch windows", None),
    ("alt-shift-tab", Desktop::Kde, "KDE: switch windows", None),
    ("alt-shift-f12", Desktop::Kde, "KDE: suspend compositing", None),
    ("ctrl-escape", Desktop::Kde, "KDE: System Monitor", None),
    ("ctrl-alt-delete", Desktop::Kde, "KDE: log out", None),
    ("ctrl-alt-l", Desktop::Kde, "KDE: lock the screen", None),
    ("alt-f2", Desktop::Gnome, "GNOME: run a command", None),
    ("alt-f4", Desktop::Gnome, "GNOME: close the window", Some(Quit)),
    ("alt-f5", Desktop::Gnome, "GNOME: restore the window", None),
    ("alt-f6", Desktop::Gnome, "GNOME: switch the program's windows", None),
    ("alt-shift-f6", Desktop::Gnome, "GNOME: switch the program's windows", None),
    ("alt-f7", Desktop::Gnome, "GNOME: move the window", None),
    ("alt-f8", Desktop::Gnome, "GNOME: resize the window", None),
    ("alt-f10", Desktop::Gnome, "GNOME: maximize the window", None),
    ("alt-space", Desktop::Gnome, "GNOME: the window menu", None),
    ("alt-tab", Desktop::Gnome, "GNOME: switch programs", None),
    ("alt-escape", Desktop::Gnome, "GNOME: switch windows", None),
    ("alt-shift-escape", Desktop::Gnome, "GNOME: switch windows", None),
    ("ctrl-alt-tab", Desktop::Gnome, "GNOME: switch system controls", None),
    ("ctrl-alt-escape", Desktop::Gnome, "GNOME: switch system controls", None),
    ("ctrl-alt-up", Desktop::Gnome, "GNOME: the workspace above", None),
    ("ctrl-alt-down", Desktop::Gnome, "GNOME: the workspace below", None),
    ("ctrl-alt-left", Desktop::Gnome, "GNOME: the workspace to the left", None),
    ("ctrl-alt-right", Desktop::Gnome, "GNOME: the workspace to the right", None),
];

/// Who takes `key` from `command` on `desktop`, if anyone.
fn taken_on(key: &str, command: Command, desktop: Desktop) -> Option<&'static str> {
    let mac = desktop == Desktop::Mac;
    let key = normalize(key, mac);
    let applies = |on: Desktop| {
        on == desktop || (on == Desktop::Linux && matches!(desktop, Desktop::Kde | Desktop::Gnome))
    };
    // Every Linux desktop switches to a text console with these.
    if matches!(desktop, Desktop::Kde | Desktop::Gnome | Desktop::Linux)
        && key.strip_prefix("ctrl-alt-f").is_some_and(|n| n.parse::<u8>().is_ok())
    {
        return Some("Linux: switch to a text console");
    }
    RESERVED
        .iter()
        .find(|(reserved, on, _, allows)| {
            applies(*on) && normalize(reserved, mac) == key && *allows != Some(command)
        })
        .map(|(_, _, by, _)| *by)
}

fn this_desktop() -> Desktop {
    if cfg!(windows) {
        return Desktop::Windows;
    }
    if cfg!(target_os = "macos") {
        return Desktop::Mac;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let named = |name: &str| desktop.split(':').any(|part| part.eq_ignore_ascii_case(name));
    match () {
        _ if named("KDE") => Desktop::Kde,
        _ if named("GNOME") => Desktop::Gnome,
        _ => Desktop::Linux,
    }
}

/// Who on this computer takes `key` before it reaches `command`.
pub fn taken_by(key: &str, command: Command) -> Option<&'static str> {
    taken_on(key, command, this_desktop())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_keys_one_way() {
        assert_eq!(normalize("shift-ctrl-f4", false), "ctrl-shift-f4");
        assert_eq!(normalize("secondary-shift-a", false), "ctrl-shift-a");
        assert_eq!(normalize("secondary-shift-a", true), "shift-cmd-a");
        assert_eq!(normalize("ctrl-alt-secondary-1", false), "ctrl-alt-1");
        assert_eq!(normalize("ctrl--", false), "ctrl--");
        assert_eq!(normalize("alt-+", false), "alt-+");
        assert_eq!(normalize("ctrl-x d", false), "ctrl-x d");
    }

    #[test]
    fn no_preset_gives_a_key_to_two_commands() {
        for preset in Preset::ALL {
            for mac in [false, true] {
                let mut keys: Vec<String> = preset.table().iter().flat_map(|(_, a, b)| [*a, *b]).filter(|k| !k.is_empty()).map(|k| normalize(k, mac)).collect();
                let count = keys.len();
                keys.sort();
                keys.dedup();
                assert_eq!(keys.len(), count, "{} gives a key twice", preset.name());
            }
        }
    }

    /// Its keys reach the program on every system, and every keyboard has them.
    #[test]
    fn power_commander_keys_work_everywhere() {
        let desktops = [Desktop::Windows, Desktop::Mac, Desktop::Kde, Desktop::Gnome];
        let numpad = ["add", "subtract", "multiply", "divide", "insert"];
        for (command, main, alternate) in POWER_COMMANDER {
            for key in [main, alternate].into_iter().filter(|k| !k.is_empty()) {
                for desktop in desktops {
                    let taken = taken_on(key, *command, desktop);
                    assert_eq!(taken, None, "{key} of {command:?} on {desktop:?}");
                }
                let last = key.rsplit('-').next().unwrap_or(key);
                assert!(!numpad.contains(&last), "{key}: not on every keyboard");
            }
        }
    }

    #[test]
    fn changes_keys_and_takes_them_from_others() {
        let mut keys = KeySettings::default();
        assert_eq!(keys.command_of("ctrl-1"), Some(SortByName));
        // Ctrl+1 moves to Size; Name has none then.
        assert_eq!(keys.set(SortBySize, 0, "ctrl-1".into()), Some(SortByName));
        assert_eq!(keys.keys_of(SortByName), [String::new(), String::new()]);
        assert_eq!(keys.command_of("ctrl-1"), Some(SortBySize));
        // Back as the preset had it: no longer a change.
        keys.set(SortByName, 0, "secondary-1".into());
        keys.set(SortBySize, 0, "secondary-4".into());
        assert!(keys.changed.is_empty(), "{:?}", keys.changed);
        assert_eq!(taken_on("ctrl-f3", SortByName, Desktop::Kde), Some("KDE: switch to desktop 3"));
        assert_eq!(taken_on("alt-f4", Quit, Desktop::Kde), None);
        assert!(taken_on("ctrl-alt-f2", View, Desktop::Gnome).is_some());
    }
}
