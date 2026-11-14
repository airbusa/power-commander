//! macOS native menu implementation (AppKit `NSMenu` via objc2).

use std::{cell::Cell, sync::Arc};

use gpui::{Action, App, AssetSource, Pixels, Point, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSData, NSPoint, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{NativeMenuItem, resolve_icon_image};

/// Side length (in points) menu item images are scaled to. AppKit does not resize the image to fit
/// the row, so a large file would otherwise overflow it.
const MENU_IMAGE_SIZE: f64 = 16.0;

/// Ivars for [`MenuTarget`]: the tag of the selected item, or `-1` if none.
struct MenuTargetIvars {
    selected: Cell<isize>,
}

define_class!(
    // A throwaway Objective-C object that receives the menu item action and
    // records which item (by tag) was clicked.
    #[unsafe(super(NSObject))]
    #[name = "GPUIComponentNativeMenuTarget"]
    #[ivars = MenuTargetIvars]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(menuItemClicked:))]
        fn menu_item_clicked(&self, sender: &NSMenuItem) {
            self.ivars().selected.set(sender.tag());
        }
    }
);

impl MenuTarget {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(MenuTargetIvars {
            selected: Cell::new(-1),
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Show a native popup menu and dispatch the selected item's action.
///
/// The AppKit tracking loop is run from a foreground task so that GPUI is not
/// borrowed while the menu is open.
pub(super) fn show(
    items: Vec<NativeMenuItem>,
    asset_source: Arc<dyn AssetSource>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(view_ptr) = ns_view_ptr(window) else {
        return;
    };
    // Inherent `Window::window_handle` (GPUI's `AnyWindowHandle`), not the
    // `raw_window_handle::HasWindowHandle` trait method in scope below.
    let handle = Window::window_handle(window);

    cx.spawn(async move |cx| {
        let action = run_menu(view_ptr, &items, asset_source.as_ref(), position);
        let _ = cx.update(move |app| {
            let _ = handle.update(app, move |_, window, app| {
                if let Some(action) = action {
                    window.dispatch_action(action, app);
                }
                // Wake GPUI after the AppKit tracking loop returns so the window
                // resumes painting and re-registers its mouse handlers. Without
                // this, a dismissed menu (especially when nothing is selected)
                // leaves the window idle and unresponsive to a second
                // right-click.
                window.refresh();
            });
        });
    })
    .detach();
}

/// Build the menu (recursively, including submenus), show it, and return the
/// selected item's action.
fn run_menu(
    view_ptr: usize,
    items: &[NativeMenuItem],
    asset_source: &dyn AssetSource,
    position: Point<Pixels>,
) -> Option<Box<dyn Action>> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: `view_ptr` came from the window's AppKit handle, and the window
    // outlives this synchronous call.
    let view: &NSView = unsafe { &*(view_ptr as *const NSView) };

    let target = MenuTarget::new();
    let mut actions: Vec<&Box<dyn Action>> = Vec::new();
    let ns_menu = build_menu(items, asset_source, &target, mtm, &mut actions);

    // `position` is window-relative, logical pixels, origin top-left (GPUI).
    // AppKit view coordinates have their origin at the bottom-left, so flip y.
    let height = view.bounds().size.height;
    let location = NSPoint::new(
        f32::from(position.x) as f64,
        height - f32::from(position.y) as f64,
    );
    ns_menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(view));

    let tag = target.ivars().selected.get();
    if tag >= 0 {
        actions.get(tag as usize).map(|action| action.boxed_clone())
    } else {
        None
    }
}

/// Recursively build an `NSMenu`. Each actionable leaf item is given a tag equal
/// to its index in `actions`, so the selected tag maps back to its action.
fn build_menu<'a>(
    items: &'a [NativeMenuItem],
    asset_source: &dyn AssetSource,
    target: &MenuTarget,
    mtm: MainThreadMarker,
    actions: &mut Vec<&'a Box<dyn Action>>,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    // Items are configured explicitly, so disable AppKit's automatic enabling.
    menu.setAutoenablesItems(false);

    for item in items {
        match item {
            NativeMenuItem::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            NativeMenuItem::Item {
                label,
                disabled,
                checked,
                icon,
                action,
            } => {
                let ns_item = NSMenuItem::new(mtm);
                unsafe {
                    ns_item.setTitle(&NSString::from_str(label));
                    ns_item.setEnabled(!*disabled);
                    if let Some(icon) = icon {
                        if let Some(ns_image) = ns_image_for_icon(icon, asset_source) {
                            ns_image.setSize(NSSize::new(MENU_IMAGE_SIZE, MENU_IMAGE_SIZE));
                            ns_image.setTemplate(true);
                            ns_item.setImage(Some(&ns_image));
                        }
                    }

                    if *checked {
                        // `NSControlStateValueOn`
                        ns_item.setState(1);
                    }
                    if let Some(action) = action {
                        if !*disabled {
                            ns_item.setTag(actions.len() as isize);
                            actions.push(action);
                            ns_item.setTarget(Some(target as &AnyObject));
                            ns_item.setAction(Some(sel!(menuItemClicked:)));
                        }
                    }
                }
                menu.addItem(&ns_item);
            }
            NativeMenuItem::Submenu {
                label,
                disabled,
                items,
            } => {
                let ns_item = NSMenuItem::new(mtm);
                let submenu = build_menu(items, asset_source, target, mtm, actions);
                ns_item.setTitle(&NSString::from_str(label));
                ns_item.setEnabled(!*disabled);
                ns_item.setSubmenu(Some(&submenu));
                menu.addItem(&ns_item);
            }
        }
    }

    menu
}

fn ns_image_for_icon(
    icon: &crate::Icon,
    asset_source: &dyn AssetSource,
) -> Option<Retained<NSImage>> {
    let image = resolve_icon_image(icon, asset_source)?;
    if image.bytes.is_empty() {
        return None;
    }

    let data = unsafe {
        NSData::dataWithBytes_length(image.bytes.as_ptr().cast(), image.bytes.len() as _)
    };
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// SparkCommander patch: an entry of a drawn menu shown by AppKit, see
/// `PopupMenu::show_native`.
pub(crate) enum NativeEntry {
    Separator,
    Header(gpui::SharedString),
    Item {
        label: gpui::SharedString,
        disabled: bool,
        checked: bool,
        icon: Option<crate::Icon>,
        /// Its single-keystroke shortcut, shown as AppKit's key equivalent.
        key: Option<gpui::Keystroke>,
    },
    Submenu {
        label: gpui::SharedString,
        disabled: bool,
        items: Vec<NativeEntry>,
    },
}

/// SparkCommander patch: shows `entries` as an `NSMenu` at `position`; `picked`
/// gets the path to the item chosen, its index in each menu down to it.
pub(crate) fn pop_up_entries(
    entries: Vec<NativeEntry>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
    picked: impl FnOnce(Vec<usize>, &mut Window, &mut App) + 'static,
) {
    let Some(view_ptr) = ns_view_ptr(window) else {
        return;
    };
    let handle = Window::window_handle(window);
    let asset_source = cx.asset_source().clone();
    cx.spawn(async move |cx| {
        let path = run_entries(view_ptr, &entries, asset_source.as_ref(), position);
        let _ = cx.update(move |app| {
            let _ = handle.update(app, move |_, window, app| {
                if let Some(path) = path {
                    picked(path, window, app);
                }
                window.refresh();
            });
        });
    })
    .detach();
}

fn run_entries(
    view_ptr: usize,
    entries: &[NativeEntry],
    asset_source: &dyn AssetSource,
    position: Point<Pixels>,
) -> Option<Vec<usize>> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: as in `run_menu`.
    let view: &NSView = unsafe { &*(view_ptr as *const NSView) };
    let target = MenuTarget::new();
    let mut paths = Vec::new();
    let menu = build_entries(entries, &mut Vec::new(), asset_source, &target, mtm, &mut paths);
    let height = view.bounds().size.height;
    let location = NSPoint::new(f32::from(position.x) as f64, height - f32::from(position.y) as f64);
    menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(view));
    let tag = target.ivars().selected.get();
    (tag >= 0).then(|| paths.get(tag as usize).cloned()).flatten()
}

fn build_entries(
    entries: &[NativeEntry],
    at: &mut Vec<usize>,
    asset_source: &dyn AssetSource,
    target: &MenuTarget,
    mtm: MainThreadMarker,
    paths: &mut Vec<Vec<usize>>,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for (ix, entry) in entries.iter().enumerate() {
        at.push(ix);
        match entry {
            NativeEntry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            NativeEntry::Header(label) => {
                menu.addItem(&NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(label), mtm))
            }
            NativeEntry::Item { label, disabled, checked, icon, key } => {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(label));
                item.setEnabled(!*disabled);
                if *checked {
                    item.setState(1);
                }
                if let Some(image) = icon.as_ref().and_then(|icon| ns_image_for_icon(icon, asset_source)) {
                    image.setSize(NSSize::new(MENU_IMAGE_SIZE, MENU_IMAGE_SIZE));
                    image.setTemplate(true);
                    item.setImage(Some(&image));
                }
                if let Some((key, modifiers)) = key.as_ref().and_then(key_equivalent) {
                    item.setKeyEquivalent(&NSString::from_str(&key));
                    item.setKeyEquivalentModifierMask(modifiers);
                }
                if !*disabled {
                    unsafe {
                        item.setTag(paths.len() as isize);
                        item.setTarget(Some(target as &AnyObject));
                        item.setAction(Some(sel!(menuItemClicked:)));
                    }
                    paths.push(at.clone());
                }
                menu.addItem(&item);
            }
            NativeEntry::Submenu { label, disabled, items } => {
                let item = NSMenuItem::new(mtm);
                let submenu = build_entries(items, at, asset_source, target, mtm, paths);
                item.setTitle(&NSString::from_str(label));
                item.setEnabled(!*disabled);
                item.setSubmenu(Some(&submenu));
                menu.addItem(&item);
            }
        }
        at.pop();
    }
    menu
}

/// A keystroke as AppKit's key equivalent: its key, then its modifiers.
fn key_equivalent(stroke: &gpui::Keystroke) -> Option<(String, objc2_app_kit::NSEventModifierFlags)> {
    use objc2_app_kit::NSEventModifierFlags as Flags;
    let function = |offset: u32| char::from_u32(0xF700 + offset).map(String::from);
    let key = match stroke.key.as_str() {
        "up" => function(0x00)?,
        "down" => function(0x01)?,
        "left" => function(0x02)?,
        "right" => function(0x03)?,
        "insert" => function(0x27)?,
        "delete" => function(0x28)?,
        "home" => function(0x29)?,
        "end" => function(0x2B)?,
        "pageup" => function(0x2C)?,
        "pagedown" => function(0x2D)?,
        "enter" => "\r".into(),
        "tab" => "\t".into(),
        "space" => " ".into(),
        "escape" => "\u{1b}".into(),
        "backspace" => "\u{8}".into(),
        key if key.len() > 1 && key.starts_with('f') => function(0x04 + key[1..].parse::<u32>().ok()?.checked_sub(1)?)?,
        key if key.chars().count() == 1 => key.to_lowercase(),
        _ => return None,
    };
    let modifiers = &stroke.modifiers;
    let mut flags = Flags::empty();
    for (on, flag) in [
        (modifiers.platform, Flags::Command),
        (modifiers.shift, Flags::Shift),
        (modifiers.alt, Flags::Option),
        (modifiers.control, Flags::Control),
    ] {
        if on {
            flags |= flag;
        }
    }
    Some((key, flags))
}

/// SparkCommander patch: a pop-up button's menu as AppKit shows it, the chosen
/// row over the button and the menu at least as wide; `chosen` gets the row picked.
pub(crate) fn pop_up_list(
    labels: Vec<gpui::SharedString>,
    selected: Option<usize>,
    bounds: gpui::Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
    chosen: impl FnOnce(usize, &mut Window, &mut App) + 'static,
) {
    let Some(view_ptr) = ns_view_ptr(window) else {
        return;
    };
    let handle = Window::window_handle(window);
    cx.spawn(async move |cx| {
        let row = run_list(view_ptr, &labels, selected, bounds);
        let _ = cx.update(move |app| {
            let _ = handle.update(app, move |_, window, app| {
                if let Some(row) = row {
                    chosen(row, window, app);
                }
                window.refresh();
            });
        });
    })
    .detach();
}

fn run_list(
    view_ptr: usize,
    labels: &[gpui::SharedString],
    selected: Option<usize>,
    bounds: gpui::Bounds<Pixels>,
) -> Option<usize> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: as in `run_menu`.
    let view: &NSView = unsafe { &*(view_ptr as *const NSView) };
    let target = MenuTarget::new();
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setMinimumWidth(f32::from(bounds.size.width) as f64 + LIST_INSET);
    for (row, label) in labels.iter().enumerate() {
        let item = NSMenuItem::new(mtm);
        unsafe {
            item.setTitle(&NSString::from_str(label));
            item.setTag(row as isize);
            item.setTarget(Some(&target as &AnyObject));
            item.setAction(Some(sel!(menuItemClicked:)));
        }
        if Some(row) == selected {
            item.setState(1);
        }
        menu.addItem(&item);
    }
    let positioning = selected.and_then(|row| menu.itemAtIndex(row as isize));
    let height = view.bounds().size.height;
    let location = NSPoint::new(
        f32::from(bounds.origin.x) as f64 - LIST_INSET,
        height - f32::from(bounds.origin.y) as f64 + LIST_RAISE,
    );
    menu.popUpMenuPositioningItem_atLocation_inView(positioning.as_deref(), location, Some(view));
    let tag = target.ivars().selected.get();
    (tag >= 0).then_some(tag as usize)
}

/// How far left of and above the button the menu starts, so the chosen title lies on the button's.
const LIST_INSET: f64 = 13.;
const LIST_RAISE: f64 = 4.;

/// Extract the AppKit `NSView` pointer from the window's raw handle.
fn ns_view_ptr(window: &Window) -> Option<usize> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.ns_view.as_ptr() as usize)
}
