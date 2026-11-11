# Patched third-party crates

Copied as is from Water Data Studio's `vendor/` (WDS = Water Data Studio), so
both apps share one set of GPUI Kit patches; keep them in step.

## gpui-component 0.7.3 (GPUI Kit)

`gpui-component/` is the published crate from crates.io with WDS's changes,
wired in through `[patch.crates-io]` in the workspace `Cargo.toml`. License:
Apache-2.0 (`gpui-component/LICENSE-APACHE`). Each modified file starts with
a notice, and every change is marked `WDS patch`.

| File | Change |
|---|---|
| `src/dialog/dialog.rs` | a dialog moves by dragging its title; `Dialog::center_over` opens it centred over part of the window (the work area a message is about); dialogs fade in where they open instead of sliding down; PowerCommander patch: the title is small text (`text_sm`, upstream `text_base`) and a long one ends in "…" instead of running under the close button |
| `src/highlighter/mod.rs`, `src/highlighter/input_adapter.rs` | `input_highlighter_factory` is public, so the app can wrap the editor's tree-sitter highlighter (WDS highlights `GO`-separated batches); a background parse asked for no folds leaves the fold candidates alone instead of clearing them, so the app can give its own |
| `src/input/popovers/completion_menu.rs` | the completion menu is as wide as `CompletionMenuOptions::max_width`; sized to its first measured item, it stayed too narrow for later items |
| `src/kbd.rs` | a key binding of several keystrokes (a chord, Ctrl+K Ctrl+0) shows them all, not only the first; PowerCommander patch: the numpad's `add`, `subtract`, `multiply`, `divide` show as Num +, Num -, Num *, Num / |
| `src/list/list.rs` | `ListState::confirm_on_double_click`: a click only selects, a double click or Enter confirms |
| `src/menu/app_menu_bar.rs`, `src/menu/mod.rs` | menu-bar menus may be as tall as the window minus 48 px (upstream: half the window, at most 450 px, then scrolling); PowerCommander patch: `AppMenuBar::built` makes a bar of a window's own menus (the viewer's), each built by a `MenuBuilder` as it opens, so it shows the window's state then; PowerCommander patch: an open menu's name is drawn in the theme's `primary` colour with `primary_foreground` text, as KDE's menu bar does |
| `src/menu/menu_item.rs` | PowerCommander patch: menu items are rounded, and the highlighted one is framed in the theme's `ring` colour, as KDE's Breeze menus |
| `src/menu/popup_menu.rs` | PowerCommander patch: every menu and submenu may be as tall as the window minus 16 px (upstream: menus of over 20 items scroll within half the window, at most 450 px); only a menu taller than that scrolls, by wheel and keys, and no menu shows a scrollbar; `scrollable()` has no effect |
| `src/tree.rs` | `Tree::context_menu` builders get the menu's `Context<PopupMenu>` instead of `Context<TreeState>`, so tree menus can have submenus; the row renderer's `selected` is not overridden, so the app can show several selected rows |
| `src/root.rs` | notifications paint above dialogs (deferred, priority 90) |
| `src/styled.rs` | PowerCommander patch: a popup's hairline ring is the theme's `border` colour (upstream: the foreground at 10%), as KDE frames menus |
| `src/tab/tab_bar.rs` | the segmented tab bar's indicator uses the theme's `tab_active` colour instead of the background, so the app can tint the active tab |
| `src/tab/tab.rs` | PowerCommander patch: the selected tab of the `Tab` variant has a 2 px line of the theme's `primary` colour on top, as KDE's Breeze tabs |
| `src/table/data_table.rs` | `DataTable::column_lines` draws lines between columns; shift and an arrow extends a range of cells; no focus ring, the selected cell shows where the keys go |
| `src/table/delegate.rs` | `TableDelegate::cell_context_menu`: a right-clicked cell's menu knows its column (by default its row's menu); `TableDelegate::cell_background`: a cell's own background, as a changed value's, filling the cell under the selection's |
| `src/table/state.rs` | a column sorts ascending first, then descending, then as it was (upstream: descending first); clearing the selection drops the right-clicked cell's outline, which otherwise stayed after an action of its menu; in cell selection, the row header shows row numbers, and the selected cell's row number and column header are highlighted; the selected cell gets a background instead of an overlay, which hid the text under an opaque `table_active`; a range of cells is selected by dragging or a shift click (`selected_range`, `set_selected_range`), and right-clicking outside it selects that cell; a right-clicked cell gets its row's context menu, whose actions go to the table; column lines; a cell takes the delegate's `cell_background`; the horizontal scrollbar starts after the row numbers, as its range is measured by its own width (upstream it took the row numbers' width too and stopped that much short of the last column) |

## gpui-base

`gpui-base/` is GPUI Kit's base crate, patched the same way.

| File | Change |
|---|---|
| `src/input/base/state.rs` | moving and deleting by words stops at a `.` between names, as code has them; `set_folded`, `is_folded` and `unfold_all`, so the app folds lines from the keyboard |
| `src/input/base/element.rs` | a multi-line input's placeholder wraps at the input's width instead of running past it |

## gpui-pre-linux 0.3.9 (GPUI's Linux platform)

`gpui-pre-linux/` is GPUI's Linux backend from crates.io, PowerCommander's
own patch (not from WDS), wired in the same way. License: Apache-2.0
(`gpui-pre-linux/LICENSE-APACHE`). `gpui-pre-linux.patch` holds the full
diff against upstream; every change is marked `PowerCommander patch`.

| File | Change |
|---|---|
| `src/linux/clipboard_files.rs` (new), `src/linux.rs` | files on the clipboard as file managers put them: `text/uri-list` with GNOME's `x-special/gnome-copied-files` and KDE's `application/x-kde-cutselection`; an item's `ExternalPaths` entry is the files, its string entry's metadata `cut` marks a cut |
| `src/linux/wayland/client.rs`, `src/linux/wayland/clipboard.rs` | an item with files offers those types too and sends each as asked; another program's URI list reads as files, cut or copied, before its text |
| `src/linux/x11/client.rs`, `src/linux/x11/clipboard.rs` | the same on X11: the types are stored beside the text, and a URI list is read before the text |

### Upgrading GPUI Kit

`gpui-component.patch` holds the full diff against upstream 0.7.3. For a new
version:

```sh
cp -r ~/.cargo/registry/src/*/gpui-component-<new>/ vendor/gpui-component-new
cd vendor/gpui-component-new && patch -p1 < ../gpui-component.patch
```

If it applies, replace `gpui-component/` with it and regenerate the patch;
if not, re-make the changes by hand. Drop each change once GPUI Kit covers it.
