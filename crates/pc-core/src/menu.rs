//! Keys in GPUI syntax; numpad keys are `add`, `subtract`, `multiply`, `divide`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    // Files
    ChangeAttributes,
    Pack,
    UnpackSpecificFiles,
    TestArchives,
    CompareByContent,
    AssociateWith,
    InternalAssociations,
    Properties,
    CalculateOccupiedSpace,
    MultiRenameTool,
    SplitFile,
    CombineFiles,
    CreateChecksumFiles,
    VerifyChecksums,
    Quit,
    // Mark
    SelectGroup,
    UnselectGroup,
    SelectAll,
    UnselectAll,
    InvertSelection,
    SelectAllWithSameExtension,
    SaveSelection,
    RestoreSelection,
    SaveSelectionToFile,
    LoadSelectionFromFile,
    CopyNamesToClipboard,
    CopyNamesWithPathToClipboard,
    CopyWithAllDetails,
    CopyWithPathAndDetails,
    CompareDirectories,
    MarkNewerHideSame,
    // Commands
    CdTree,
    Search,
    SearchInSeparateProcess,
    VolumeLabel,
    SystemInformation,
    SynchronizeDirs,
    DirectoryHotlist,
    GoBack,
    OpenTerminal,
    BranchView,
    OpenDesktopFolder,
    BackgroundTransferManager,
    SwapPanels,
    TargetEqualsSource,
    // Show
    Brief,
    Full,
    Comments,
    Tree,
    ThumbnailView,
    QuickViewPanel,
    VerticalArrangement,
    DuplicateTab,
    AllFiles,
    Programs,
    AllWithExtension,
    CustomFilter,
    OnlySelectedFiles,
    SortByName,
    SortByExtension,
    SortByTime,
    SortBySize,
    Unsorted,
    ReversedOrder,
    RereadSource,
    // Configuration
    Settings,
    // Button bar
    View,
    Edit,
    EditExternally,
    ToggleMark,
    OperationQueue,
    ConnectToServer,
    Disconnect,
    Copy,
    Move,
    NewFolder,
    Delete,
    DeletePermanently,
    // Tab menu
    RenameTab,
    LockTab,
    CopyTabToOtherPanel,
    CloseTab,
    CloseAllTabs,
    CloseDuplicateTabs,
    ListTabs,
    NextTab,
    PreviousTab,
}

pub enum Item {
    Command {
        label: &'static str,
        keys: &'static str,
        command: Command,
    },
    Submenu {
        label: &'static str,
        items: &'static [Item],
    },
    Themes {
        label: &'static str,
    },
    /// A submenu of the side's recently closed tabs.
    ClosedTabs {
        label: &'static str,
    },
    Separator,
}

pub struct Menu {
    pub label: &'static str,
    pub items: &'static [Item],
}

const fn cmd(label: &'static str, keys: &'static str, command: Command) -> Item {
    Item::Command {
        label,
        keys,
        command,
    }
}

const SEP: Item = Item::Separator;

use Command::*;

pub const MENUS: &[Menu] = &[
    Menu {
        label: "Files",
        items: &[
            cmd("Change Attributes...", "", ChangeAttributes),
            cmd("Pack...", "alt-f5", Pack),
            cmd("Unpack Specific Files...", "alt-f9", UnpackSpecificFiles),
            cmd("Test Archive(s)", "alt-shift-f9", TestArchives),
            cmd("Compare By Content...", "", CompareByContent),
            cmd("Associate With...", "", AssociateWith),
            cmd(
                "Internal Associations (PowerCommander Only)...",
                "",
                InternalAssociations,
            ),
            cmd("Properties...", "alt-enter", Properties),
            cmd("Edit With External Editor", "ctrl-shift-f4", EditExternally),
            cmd(
                "Calculate Occupied Space...",
                "ctrl-l",
                CalculateOccupiedSpace,
            ),
            cmd("Multi-Rename Tool...", "ctrl-m", MultiRenameTool),
            SEP,
            cmd("Split File...", "", SplitFile),
            cmd("Combine Files...", "", CombineFiles),
            cmd(
                "Create Checksum File(s) (CRC32, MD5, SHA1, SHA256, SHA512)...",
                "",
                CreateChecksumFiles,
            ),
            cmd(
                "Verify Checksums (from checksum files)",
                "",
                VerifyChecksums,
            ),
            SEP,
            cmd("Quit", "alt-f4", Quit),
        ],
    },
    Menu {
        label: "Mark",
        items: &[
            cmd("Select Group...", "add", SelectGroup),
            cmd("Unselect Group...", "subtract", UnselectGroup),
            cmd("Select All", "ctrl-add", SelectAll),
            cmd("Unselect All", "ctrl-subtract", UnselectAll),
            cmd("Invert Selection", "multiply", InvertSelection),
            cmd(
                "Select All With Same Extension",
                "alt-add",
                SelectAllWithSameExtension,
            ),
            SEP,
            cmd("Save Selection", "", SaveSelection),
            cmd("Restore Selection", "divide", RestoreSelection),
            cmd("Save Selection To File", "", SaveSelectionToFile),
            cmd("Load Selection From File", "", LoadSelectionFromFile),
            SEP,
            cmd("Copy Selected Names To Clipboard", "", CopyNamesToClipboard),
            cmd(
                "Copy Names With Path To Clipboard",
                "",
                CopyNamesWithPathToClipboard,
            ),
            cmd("Copy To Clipboard With All Details", "", CopyWithAllDetails),
            cmd(
                "Copy To Clipboard With Path+Details",
                "",
                CopyWithPathAndDetails,
            ),
            SEP,
            cmd("Compare Directories", "shift-f2", CompareDirectories),
            cmd("Mark Newer, Hide Same Files", "", MarkNewerHideSame),
        ],
    },
    Menu {
        label: "Commands",
        items: &[
            cmd("Operation Queue...", "", OperationQueue),
            cmd("CD Tree...", "alt-f10", CdTree),
            cmd("Search...", "alt-f7", Search),
            cmd(
                "Search in separate Process...",
                "alt-shift-f7",
                SearchInSeparateProcess,
            ),
            cmd("Volume Label...", "", VolumeLabel),
            cmd("System Information...", "", SystemInformation),
            cmd("Synchronize Dirs...", "", SynchronizeDirs),
            cmd("Directory Hotlist", "ctrl-d", DirectoryHotlist),
            cmd("Go Back", "alt-left", GoBack),
            SEP,
            cmd("Open command prompt window", "", OpenTerminal),
            SEP,
            cmd("Branch View (With Subdirs)", "ctrl-b", BranchView),
            cmd("Open Desktop Folder", "", OpenDesktopFolder),
            cmd(
                "Background Transfer Manager...",
                "",
                BackgroundTransferManager,
            ),
            SEP,
            cmd("Source<->Target", "ctrl-u", SwapPanels),
            cmd("Target=Source", "", TargetEqualsSource),
        ],
    },
    Menu {
        label: "Net",
        items: &[
            cmd("Connect to Server...", "ctrl-f", ConnectToServer),
            cmd("Disconnect", "ctrl-shift-f", Disconnect),
        ],
    },
    Menu {
        label: "Show",
        items: &[
            cmd("Brief", "ctrl-f1", Brief),
            cmd("Full", "ctrl-f2", Full),
            cmd("Comments", "ctrl-shift-f2", Comments),
            Item::Submenu {
                label: "Custom Columns Mode",
                items: &[],
            },
            Item::Submenu {
                label: "Custom View Modes",
                items: &[],
            },
            cmd("Tree", "ctrl-f8", Tree),
            Item::Submenu {
                label: "Separate Tree",
                items: &[],
            },
            cmd("Thumbnail View", "ctrl-shift-f1", ThumbnailView),
            cmd("Quick View Panel", "ctrl-q", QuickViewPanel),
            cmd("Vertical Arrangement", "", VerticalArrangement),
            cmd("Duplicate this tab", "ctrl-t", DuplicateTab),
            SEP,
            cmd("All Files", "ctrl-f10", AllFiles),
            cmd("Programs", "ctrl-f11", Programs),
            cmd("*.*", "", AllWithExtension),
            cmd("Custom...", "ctrl-f12", CustomFilter),
            cmd("Only Selected Files", "", OnlySelectedFiles),
            SEP,
            cmd("Name", "ctrl-f3", SortByName),
            cmd("Extension", "ctrl-f4", SortByExtension),
            cmd("Time", "ctrl-f5", SortByTime),
            cmd("Size", "ctrl-f6", SortBySize),
            cmd("Unsorted", "ctrl-f7", Unsorted),
            SEP,
            cmd("Reversed Order", "", ReversedOrder),
            SEP,
            cmd("Reread Source", "ctrl-r", RereadSource),
        ],
    },
    Menu {
        label: "Configuration",
        items: &[
            cmd("Settings...", "", Settings),
            Item::Themes { label: "Theme" },
        ],
    },
];

/// The right-click menu of folder tabs.
pub const TAB_MENU: &[Item] = &[
    cmd("Duplicate this tab", "ctrl-t", DuplicateTab),
    cmd("Rename tab...", "", RenameTab),
    cmd("Lock this tab", "", LockTab),
    SEP,
    cmd("Copy tab to other panel", "", CopyTabToOtherPanel),
    SEP,
    cmd("Close this tab", "ctrl-w", CloseTab),
    cmd("Close all tabs", "ctrl-shift-w", CloseAllTabs),
    cmd("Close duplicate tabs", "", CloseDuplicateTabs),
    Item::ClosedTabs {
        label: "Recently closed tabs",
    },
    cmd("Show list of all tabs", "ctrl-shift-a", ListTabs),
];

/// The function key buttons under the panels.
pub const BUTTON_BAR: &[Item] = &[
    cmd("View", "f3", View),
    cmd("Edit", "f4", Edit),
    cmd("Copy", "f5", Copy),
    cmd("Move", "f6", Move),
    cmd("New Folder", "f7", NewFolder),
    cmd("Delete", "f8", Delete),
    cmd("Exit", "alt-f4", Quit),
];

/// Keys of commands on no menu or button.
pub const OTHER_KEYS: &[Item] = &[
    cmd("Mark", "insert", ToggleMark),
    cmd("Mark", "space", ToggleMark),
    cmd("Select All", "ctrl-a", SelectAll),
    cmd("Delete", "delete", Delete),
    cmd("Delete permanently", "shift-f8", DeletePermanently),
    cmd("Delete permanently", "shift-delete", DeletePermanently),
    cmd("Next tab", "ctrl-tab", NextTab),
    cmd("Previous tab", "ctrl-shift-tab", PreviousTab),
];

pub fn key_bindings() -> Vec<(&'static str, Command)> {
    fn walk(items: &[Item], out: &mut Vec<(&'static str, Command)>) {
        for item in items {
            match *item {
                Item::Command { keys, command, .. }
                    if !keys.is_empty() && !out.contains(&(keys, command)) =>
                {
                    out.push((keys, command))
                }
                Item::Submenu { items, .. } => walk(items, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for items in MENUS
        .iter()
        .map(|menu| menu.items)
        .chain([TAB_MENU, BUTTON_BAR, OTHER_KEYS])
    {
        walk(items, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_runs_two_commands() {
        let mut all: Vec<_> = key_bindings().into_iter().map(|(keys, _)| keys).collect();
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count, "a key is given to two commands");
    }
}
