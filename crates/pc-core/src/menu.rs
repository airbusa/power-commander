//! The menus, the button bar and the commands; their keys are the key map's.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    CopyToClipboard,
    CutToClipboard,
    PasteFromClipboard,
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
    /// Shows or hides the sidebar's tab.
    #[serde(alias = "Tree")]
    SidebarTree,
    SidebarFavourites,
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
    Shortcuts,
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
        command: Command,
    },
    Submenu {
        label: &'static str,
        items: &'static [Item],
    },
    /// System, Light and Dark.
    Appearance {
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

const fn cmd(label: &'static str, command: Command) -> Item {
    Item::Command { label, command }
}

const SEP: Item = Item::Separator;

use Command::*;

pub const MENUS: &[Menu] = &[
    Menu {
        label: "Files",
        items: &[
            cmd("Change Attributes...", ChangeAttributes),
            cmd("Pack...", Pack),
            cmd("Unpack Specific Files...", UnpackSpecificFiles),
            cmd("Test Archive(s)", TestArchives),
            cmd("Compare By Content...", CompareByContent),
            cmd("Associate With...", AssociateWith),
            cmd("Internal Associations (PowerCommander Only)...", InternalAssociations),
            cmd("Properties...", Properties),
            cmd("Edit With External Editor", EditExternally),
            cmd("Calculate Occupied Space...", CalculateOccupiedSpace),
            cmd("Multi-Rename Tool...", MultiRenameTool),
            SEP,
            cmd("Split File...", SplitFile),
            cmd("Combine Files...", CombineFiles),
            cmd("Create Checksum File(s) (CRC32, MD5, SHA1, SHA256, SHA512)...", CreateChecksumFiles),
            cmd("Verify Checksums (from checksum files)", VerifyChecksums),
            SEP,
            cmd("Quit", Quit),
        ],
    },
    Menu {
        label: "Mark",
        items: &[
            cmd("Select Group...", SelectGroup),
            cmd("Unselect Group...", UnselectGroup),
            cmd("Select All", SelectAll),
            cmd("Unselect All", UnselectAll),
            cmd("Invert Selection", InvertSelection),
            cmd("Select All With Same Extension", SelectAllWithSameExtension),
            SEP,
            cmd("Save Selection", SaveSelection),
            cmd("Restore Selection", RestoreSelection),
            cmd("Save Selection To File", SaveSelectionToFile),
            cmd("Load Selection From File", LoadSelectionFromFile),
            SEP,
            cmd("Copy Selected Names To Clipboard", CopyNamesToClipboard),
            cmd("Copy Names With Path To Clipboard", CopyNamesWithPathToClipboard),
            cmd("Copy To Clipboard With All Details", CopyWithAllDetails),
            cmd("Copy To Clipboard With Path+Details", CopyWithPathAndDetails),
            SEP,
            cmd("Compare Directories", CompareDirectories),
            cmd("Mark Newer, Hide Same Files", MarkNewerHideSame),
        ],
    },
    Menu {
        label: "Commands",
        items: &[
            cmd("Operation Queue...", OperationQueue),
            cmd("CD Tree...", CdTree),
            cmd("Search...", Search),
            cmd("Search in separate Process...", SearchInSeparateProcess),
            cmd("Volume Label...", VolumeLabel),
            cmd("System Information...", SystemInformation),
            cmd("Synchronize Dirs...", SynchronizeDirs),
            cmd("Directory Hotlist", DirectoryHotlist),
            cmd("Go Back", GoBack),
            SEP,
            cmd("Open command prompt window", OpenTerminal),
            SEP,
            cmd("Branch View (With Subdirs)", BranchView),
            cmd("Open Desktop Folder", OpenDesktopFolder),
            cmd("Background Transfer Manager...", BackgroundTransferManager),
            SEP,
            cmd("Source<->Target", SwapPanels),
            cmd("Target=Source", TargetEqualsSource),
        ],
    },
    Menu {
        label: "Show",
        items: &[
            cmd("Brief", Brief),
            cmd("Full", Full),
            cmd("Comments", Comments),
            Item::Submenu {
                label: "Custom Columns Mode",
                items: &[],
            },
            Item::Submenu {
                label: "Custom View Modes",
                items: &[],
            },
            Item::Submenu {
                label: "Sidebar",
                items: &[cmd("Folders", SidebarTree), cmd("Favourites", SidebarFavourites)],
            },
            cmd("Thumbnail View", ThumbnailView),
            cmd("Quick View Panel", QuickViewPanel),
            cmd("Vertical Arrangement", VerticalArrangement),
            cmd("Duplicate this tab", DuplicateTab),
            SEP,
            cmd("All Files", AllFiles),
            cmd("Programs", Programs),
            cmd("*.*", AllWithExtension),
            cmd("Custom...", CustomFilter),
            cmd("Only Selected Files", OnlySelectedFiles),
            SEP,
            cmd("Name", SortByName),
            cmd("Extension", SortByExtension),
            cmd("Time", SortByTime),
            cmd("Size", SortBySize),
            cmd("Unsorted", Unsorted),
            SEP,
            cmd("Reversed Order", ReversedOrder),
            SEP,
            cmd("Reread Source", RereadSource),
        ],
    },
    Menu {
        label: "Configuration",
        items: &[
            cmd("Settings...", Settings),
            cmd("Shortcuts...", Shortcuts),
            Item::Appearance { label: "Appearance" },
        ],
    },
];

/// The right-click menu of folder tabs.
pub const TAB_MENU: &[Item] = &[
    cmd("Duplicate this tab", DuplicateTab),
    cmd("Rename tab...", RenameTab),
    cmd("Lock this tab", LockTab),
    SEP,
    cmd("Copy tab to other panel", CopyTabToOtherPanel),
    SEP,
    cmd("Close this tab", CloseTab),
    cmd("Close all tabs", CloseAllTabs),
    cmd("Close duplicate tabs", CloseDuplicateTabs),
    Item::ClosedTabs {
        label: "Recently closed tabs",
    },
    cmd("Show list of all tabs", ListTabs),
];

/// The function key buttons under the panels.
pub const BUTTON_BAR: &[Item] = &[
    cmd("View", View),
    cmd("Edit", Edit),
    cmd("Copy", Copy),
    cmd("Move", Move),
    cmd("New Folder", NewFolder),
    cmd("Delete", Delete),
    cmd("Exit", Quit),
];

/// Commands on no menu or button.
pub const OTHER: &[Item] = &[
    cmd("Mark", ToggleMark),
    cmd("Delete permanently", DeletePermanently),
    cmd("Next tab", NextTab),
    cmd("Previous tab", PreviousTab),
    cmd("Copy to clipboard", CopyToClipboard),
    cmd("Cut to clipboard", CutToClipboard),
    cmd("Paste from clipboard", PasteFromClipboard),
    cmd("Connect to server", ConnectToServer),
    cmd("Disconnect", Disconnect),
];

/// Names for where the menu gives no context.
const FULL_NAMES: &[(Command, &str)] = &[
    (Brief, "Brief View"),
    (Full, "Full View"),
    (Comments, "Comments View"),
    (SidebarTree, "Sidebar: Folders"),
    (SidebarFavourites, "Sidebar: Favourites"),
    (AllFiles, "Show All Files"),
    (Programs, "Show Programs"),
    (AllWithExtension, "Show *.*"),
    (CustomFilter, "Custom Filter"),
    (OnlySelectedFiles, "Show Only Selected Files"),
    (SortByName, "Sort by Name"),
    (SortByExtension, "Sort by Extension"),
    (SortByTime, "Sort by Time"),
    (SortBySize, "Sort by Size"),
    (Unsorted, "Sort Unsorted"),
];

/// Every command once, under its menu's name, for the Shortcuts dialog.
pub fn groups() -> Vec<(&'static str, Vec<(&'static str, Command)>)> {
    fn walk(items: &[Item], seen: &mut Vec<Command>, out: &mut Vec<(&'static str, Command)>) {
        for item in items {
            match *item {
                Item::Command { label, command } if !seen.contains(&command) => {
                    seen.push(command);
                    let full = FULL_NAMES.iter().find(|(c, _)| *c == command);
                    out.push((full.map_or(label.trim_end_matches("..."), |(_, name)| name), command));
                }
                Item::Submenu { items, .. } => walk(items, seen, out),
                _ => {}
            }
        }
    }
    let mut seen = Vec::new();
    let named = MENUS.iter().map(|menu| (menu.label, menu.items));
    let others = [("Tabs", TAB_MENU), ("Button bar", BUTTON_BAR), ("Other", OTHER)];
    let mut groups = Vec::new();
    for (name, items) in named.chain(others) {
        let mut commands = Vec::new();
        walk(items, &mut seen, &mut commands);
        if !commands.is_empty() {
            groups.push((name, commands));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KeySettings, Preset};

    #[test]
    fn every_command_with_keys_is_listed() {
        let listed: Vec<Command> = groups().into_iter().flat_map(|(_, c)| c).map(|(_, c)| c).collect();
        for preset in Preset::ALL {
            let keys = KeySettings { preset, ..Default::default() };
            for (key, command) in keys.bindings() {
                assert!(listed.contains(&command), "{key}: {command:?} is on no list");
            }
        }
    }
}
