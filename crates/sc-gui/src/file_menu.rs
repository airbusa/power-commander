//! macOS: the files' right-click menu, as Finder's: what is right-clicked,
//! or what is marked if that is, opened, shown, renamed, copied.

use std::path::{Path, PathBuf};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use sc_core::finder;
use sc_core::tags::Tag;
use sc_core::menu::Command;

use crate::menus::Run;
use crate::{Commander, Open};

actions!(file_menu, [ShowPackageContents, RenameFile, Duplicate, MakeAlias, QuickLook, RevealInFinder, EditTags]);

/// Opens them with this application.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = file_menu, no_json)]
pub struct OpenWith(pub PathBuf);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = file_menu, no_json)]
pub struct ChangeTags {
    add: Vec<String>,
    remove: Vec<String>,
}

/// What the menu is for, as it opens.
struct Chosen {
    /// The cursor on `..`: the folder itself.
    folder: bool,
    /// On this Mac, not on a server nor in an archive.
    local: bool,
    in_archive: bool,
    /// The one entry's name.
    single: Option<String>,
    count: usize,
    /// The one entry is an application.
    app: bool,
    /// Whose applications Open With lists.
    first: Option<PathBuf>,
}

impl Chosen {
    /// “Name” or “3 Items”, as Finder's titles have it.
    fn what(&self) -> String {
        match &self.single {
            Some(name) => format!("“{name}”"),
            None => format!("{} Items", self.count),
        }
    }
}

impl Commander {
    /// Panel `ix`'s files; a right click on no row opens the folder's menu.
    pub(crate) fn with_file_menu(&self, ix: usize, files: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("files", ix))
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(files)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    let panel = this.panels[ix].tabs.current_mut();
                    panel.mark_all(false);
                    if panel.entries().first().is_some_and(|entry| entry.is_parent()) {
                        panel.set_cursor(0);
                    }
                    this.active = ix;
                    window.focus(&this.focus, cx);
                    this.show_file_menu(ix, window, cx);
                }),
            )
            .into_any_element()
    }

    /// AppKit's menu at the mouse; what is picked goes to the panels.
    pub(crate) fn show_file_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.chosen_paths().unwrap_or_default();
        let entries = build(self.chosen_for_menu(ix), &paths);
        let focus = self.focus.clone();
        cx.notify();
        // Its own event loop runs while it is open: outside GPUI's update.
        cx.spawn_in(window, async move |_, cx| {
            if let Some(action) = native::pop_up(&entries, &paths) {
                let _ = cx.update(|window, cx| focus.dispatch_action(&*action, window, cx));
            }
        })
        .detach();
    }

    /// Finder's Tags…: the tags all of them have, edited in a popover.
    fn edit_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) = self.chosen_paths() else {
            return;
        };
        let what = self.chosen_for_menu(self.active).what();
        let tags: Vec<Vec<String>> = paths.iter().map(|path| tags_of(path)).collect();
        let shared: Vec<String> =
            tags.first().into_iter().flatten().filter(|tag| tags.iter().all(|of| of.contains(tag))).cloned().collect();
        // Show All's: the folder's tags besides Finder's colours.
        let panel = self.panels[self.active].tabs.current();
        let mut known: Vec<Tag> = panel
            .entries()
            .iter()
            .filter(|entry| !entry.is_parent())
            .flat_map(|entry| sc_core::tags::read(&panel.folder().join(&entry.name)))
            .collect();
        known.sort_by_key(|tag| tag.name.to_lowercase());
        known.dedup_by(|a, b| a.name == b.name);
        let focus = self.focus.clone();
        cx.spawn_in(window, async move |_, cx| {
            let (done, chosen) = futures::channel::oneshot::channel();
            native::assign_tags(&what, &shared, favourites(), known, Box::new(move |tags| drop(done.send(tags))));
            let Ok(chosen) = chosen.await else {
                return;
            };
            let add = chosen.iter().filter(|tag| !shared.contains(tag)).cloned().collect();
            let remove = shared.iter().filter(|tag| !chosen.contains(tag)).cloned().collect();
            let _ = cx.update(|window, cx| focus.dispatch_action(&ChangeTags { add, remove }, window, cx));
        })
        .detach();
    }

    fn chosen_for_menu(&self, ix: usize) -> Chosen {
        let panel = self.panels[ix].tabs.current();
        let chosen = panel.chosen();
        let local = panel.remote().is_none() && !panel.in_archive();
        let single = match chosen.as_slice() {
            [entry] => Some(entry),
            _ => None,
        };
        Chosen {
            folder: chosen.is_empty(),
            local,
            in_archive: panel.in_archive(),
            // An application as Finder names it, without `.app`.
            single: single.map(|entry| match entry.is_app() {
                true => entry.name_and_extension().0,
                false => entry.display_name(),
            }),
            count: chosen.len(),
            app: single.is_some_and(|entry| entry.is_app()),
            first: chosen.first().map(|entry| panel.folder().join(&entry.name)),
        }
    }

    /// The active panel's chosen files, or its folder; on this Mac only.
    fn chosen_paths(&self) -> Option<Vec<PathBuf>> {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return None;
        }
        let paths: Vec<PathBuf> = panel.chosen().iter().map(|entry| panel.folder().join(&entry.name)).collect();
        Some(match paths.is_empty() {
            true => vec![panel.folder().to_owned()],
            false => paths,
        })
    }

    /// The menu's own commands, beside the panels' `Run`s.
    pub(crate) fn file_menu_actions(&self, panels: Div, cx: &mut Context<Self>) -> Div {
        panels
            .on_action(cx.listener(|this, _: &ShowPackageContents, window, cx| {
                let panel = this.panels[this.active].tabs.current();
                if let Some(entry) = panel.current_entry().filter(|entry| entry.is_app()) {
                    let path = panel.folder().join(&entry.name);
                    this.navigate(move |panel| panel.go_to_folder(path), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameFile, window, cx| this.rename_file(window, cx)))
            .on_action(cx.listener(|this, _: &Duplicate, window, cx| {
                this.in_background("Could not duplicate", |paths| finder::duplicate(&paths).map(drop), true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MakeAlias, window, cx| {
                this.in_background("Could not make the alias", |paths| finder::make_alias(&paths), true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickLook, window, cx| {
                this.in_background("Quick Look", |paths| finder::quick_look(&paths), false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, window, cx| {
                this.in_background("Could not show it in Finder", |paths| finder::reveal(&paths), false, window, cx)
            }))
            .on_action(cx.listener(|this, OpenWith(app): &OpenWith, window, cx| {
                let app = app.clone();
                this.in_background("Could not open", move |paths| finder::open_with(&app, &paths), false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &EditTags, window, cx| this.edit_tags(window, cx)))
            .on_action(cx.listener(|this, ChangeTags { add, remove }: &ChangeTags, window, cx| {
                let (add, remove) = (add.clone(), remove.clone());
                let change = move |paths: Vec<PathBuf>| change_tags(&paths, &add, &remove);
                this.in_background("Could not change the tags", change, true, window, cx)
            }))
    }

    /// `run` on the chosen files away from the window; `changes` the folder, which is read again then.
    fn in_background(
        &mut self,
        failed: &'static str,
        run: impl FnOnce(Vec<PathBuf>) -> std::io::Result<()> + Send + 'static,
        changes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(paths) = self.chosen_paths() else {
            return;
        };
        let folder = self.panels[self.active].tabs.current().folder().to_owned();
        let task = cx.background_executor().spawn(async move { run(paths) });
        cx.spawn_in(window, async move |this, cx| {
            let done = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(err) = done {
                    window.push_notification(Notification::error(format!("{failed}: {err}")), cx);
                }
                if changes {
                    this.refresh_folder(&folder, cx);
                }
            });
        })
        .detach();
    }
}

enum Entry {
    Item { title: String, icon: Option<Icon>, action: Box<dyn Action> },
    Submenu { title: String, entries: Vec<Entry> },
    /// The system's Share submenu for these files.
    Share(Vec<PathBuf>),
    /// Finder's colour tags, a row of dots.
    Tags(Vec<Dot>),
    Separator,
}

struct Dot {
    name: String,
    rgb: [u8; 3],
    /// Gray, drawn as a ring.
    ring: bool,
    have: Have,
    action: Box<dyn Action>,
}

/// How many of the files have the tag.
#[derive(Clone, Copy, PartialEq)]
enum Have {
    Not,
    Partly,
    All,
}

enum Icon {
    Symbol(&'static str),
    File(PathBuf),
}

fn item(title: impl Into<String>, action: impl Action) -> Entry {
    Entry::Item { title: title.into(), icon: None, action: Box::new(action) }
}

fn with_symbol(entry: Entry, symbol: &'static str) -> Entry {
    match entry {
        Entry::Item { title, action, .. } => Entry::Item { title, icon: Some(Icon::Symbol(symbol)), action },
        other => other,
    }
}

/// Finder's menu; AppKit adds the Services.
fn build(chosen: Chosen, paths: &[PathBuf]) -> Vec<Entry> {
    use Entry::Separator;
    if chosen.folder {
        let mut menu = vec![
            item("New Folder", Run(Command::NewFolder)),
            item("Paste", Run(Command::PasteFromClipboard)),
            Separator,
            item("Get Info", Run(Command::Properties)),
        ];
        if chosen.local {
            menu.push(item("Reveal in Finder", RevealInFinder));
        }
        return menu;
    }
    let what = chosen.what();
    let mut menu = vec![item("Open", Open)];
    if chosen.local {
        if let Some(first) = &chosen.first {
            menu.push(Entry::Submenu { title: "Open With".into(), entries: open_with(first) });
        }
        if chosen.app {
            menu.push(item("Show Package Contents", ShowPackageContents));
        }
    }
    let delete = if chosen.local { "Move to Trash" } else { "Delete" };
    menu.extend([Separator, item(delete, Run(Command::Delete)), Separator, item("Get Info", Run(Command::Properties))]);
    if chosen.single.is_some() && !chosen.in_archive {
        menu.push(item("Rename…", RenameFile));
    }
    menu.push(item(format!("Compress {what}…"), Run(Command::Pack)));
    if chosen.local {
        menu.extend([
            with_symbol(item("Duplicate", Duplicate), "plus.square.on.square"),
            item("Make Alias", MakeAlias),
            item(format!("Quick Look {what}"), QuickLook),
            item("Reveal in Finder", RevealInFinder),
        ]);
        if !paths.is_empty() {
            menu.push(Entry::Share(paths.to_vec()));
        }
    }
    menu.extend([
        Separator,
        with_symbol(item(format!("Copy {what}"), Run(Command::CopyToClipboard)), "doc.on.doc"),
        item("Copy Path to Clipboard", Run(Command::CopyNamesWithPathToClipboard)),
    ]);
    if chosen.local && !paths.is_empty() {
        menu.extend([Separator, item("Tags…", EditTags), Entry::Tags(dots(paths))]);
    }
    menu
}

/// Finder's colour tags in its menu's order, as the user named them.
fn favourites() -> Vec<Tag> {
    const LABELS: [(u8, &str); 7] =
        [(6, "Red"), (7, "Orange"), (5, "Yellow"), (2, "Green"), (4, "Blue"), (3, "Purple"), (1, "Grey")];
    let names = native::label_names();
    LABELS
        .iter()
        .map(|&(colour, english)| Tag {
            name: names.get(colour as usize).cloned().unwrap_or_else(|| english.to_owned()),
            colour,
        })
        .collect()
}

fn dots(paths: &[PathBuf]) -> Vec<Dot> {
    let tags: Vec<Vec<String>> = paths.iter().map(|path| tags_of(path)).collect();
    favourites()
        .into_iter()
        .map(|Tag { name, colour }| {
            let have = match tags.iter().filter(|tags| tags.contains(&name)).count() {
                0 => Have::Not,
                n if n == tags.len() => Have::All,
                _ => Have::Partly,
            };
            let change = match have {
                Have::All => ChangeTags { add: Vec::new(), remove: vec![name.clone()] },
                _ => ChangeTags { add: vec![name.clone()], remove: Vec::new() },
            };
            let rgb = sc_core::tags::rgb(colour).unwrap_or_default();
            Dot { name, rgb, ring: colour == 1, have, action: Box::new(change) }
        })
        .collect()
}

fn file_url(path: &Path) -> objc2::rc::Retained<objc2_foundation::NSURL> {
    use objc2_foundation::{NSString, NSURL};
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn tags_of(path: &Path) -> Vec<String> {
    sc_core::tags::read(path).into_iter().map(|tag| tag.name).collect()
}

fn change_tags(paths: &[PathBuf], add: &[String], remove: &[String]) -> std::io::Result<()> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSString, NSURLTagNamesKey};
    objc2::rc::autoreleasepool(|_| {
        for path in paths {
            let mut tags = tags_of(path);
            tags.retain(|tag| !remove.contains(tag));
            tags.extend(add.iter().filter(|tag| !tags.contains(tag)).cloned().collect::<Vec<_>>());
            let names: Vec<_> = tags.iter().map(|tag| NSString::from_str(tag)).collect();
            let names = NSArray::from_retained_slice(&names);
            let value: &AnyObject = &names;
            unsafe { file_url(path).setResourceValue_forKey_error(Some(value), NSURLTagNamesKey) }
                .map_err(|err| std::io::Error::other(err.localizedDescription().to_string()))?;
        }
        Ok(())
    })
}

/// The applications macOS opens `file` with: its default first.
fn open_with(file: &Path) -> Vec<Entry> {
    let (default, mut others) = applications_for(file);
    // One per app: updaters keep old copies; the one in /Applications first.
    others.sort_by_key(|app| !app.starts_with("/Applications"));
    let mut seen: Vec<String> = default.iter().filter_map(|app| bundle_id(app)).collect();
    others.retain(|app| match bundle_id(app) {
        Some(id) if seen.contains(&id) => false,
        Some(id) => {
            seen.push(id);
            true
        }
        None => Some(app) != default.as_ref(),
    });
    others.sort_by_key(|app| app_name(app).to_lowercase());
    let entry = |title: String, app: PathBuf| Entry::Item {
        title,
        icon: Some(Icon::File(app.clone())),
        action: Box::new(OpenWith(app)),
    };
    let mut menu = Vec::new();
    if let Some(app) = default {
        menu.push(entry(format!("{} (default)", app_name(&app)), app));
        if !others.is_empty() {
            menu.push(Entry::Separator);
        }
    }
    menu.extend(others.into_iter().map(|app| entry(app_name(&app), app)));
    menu
}

fn app_name(app: &Path) -> String {
    app.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

fn bundle_id(app: &Path) -> Option<String> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&app.to_string_lossy()))?;
    Some(bundle.bundleIdentifier()?.to_string())
}

/// Launch Services' default application for `file`, and all that open it.
fn applications_for(file: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;
    let workspace = NSWorkspace::sharedWorkspace();
    let url = file_url(file);
    let path = |url: &NSURL| url.path().map(|path| PathBuf::from(path.to_string()));
    let default = workspace.URLForApplicationToOpenURL(&url).and_then(|app| path(&app));
    let all = workspace.URLsForApplicationsToOpenURL(&url).iter().filter_map(|app| path(&app)).collect();
    (default, all)
}

/// AppKit's side: the menu, its pictures, sharing, tags, Services.
mod native {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::ptr::{self, NonNull};

    use block2::RcBlock;
    use gpui_kit::Action;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSBezierPath, NSColor, NSControl, NSControlTextEditingDelegate, NSEvent,
        NSEventMask, NSEventModifierFlags, NSEventType, NSFocusRingType, NSFont, NSImage, NSLineCapStyle, NSMenu, NSMenuItem, NSPasteboard, NSPopover,
        NSPopoverBehavior, NSPopoverDelegate, NSResponder, NSSharingServicePicker, NSTextAlignment, NSTextField,
        NSTextFieldDelegate, NSTokenField, NSTokenFieldDelegate, NSTrackingArea, NSTrackingAreaOptions, NSView,
        NSViewController, NSWorkspace,
    };
    use objc2_foundation::{NSArray, NSNotification, NSPoint, NSRange, NSRect, NSRectEdge, NSSize, NSString};
    use sc_core::tags::Tag;

    use super::{Dot, Entry, Have, Icon, file_url};

    thread_local! {
        static PICKED: Cell<Option<isize>> = const { Cell::new(None) };
        /// Where the menu was opened: Tags… points there.
        static ANCHOR: Cell<Option<NSPoint>> = const { Cell::new(None) };
        static OPEN: RefCell<Option<Retained<NSPopover>>> = const { RefCell::new(None) };
        static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCFileMenuTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(pick:))]
            fn pick(&self, item: &NSMenuItem) {
                PICKED.set(Some(item.tag()));
            }
        }
    );

    const FILE_TYPES: [&str; 2] = ["public.file-url", "NSFilenamesPboardType"];

    define_class!(
        /// Offers the files to Services, as Finder's selection.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCFileMenuRequestor"]
        #[ivars = Vec<PathBuf>]
        struct Requestor;

        impl Requestor {
            #[unsafe(method(validRequestorForSendType:returnType:))]
            fn valid_requestor(&self, send: Option<&NSString>, back: Option<&NSString>) -> *mut AnyObject {
                let files = send.is_some_and(|send| FILE_TYPES.contains(&&*send.to_string()));
                match files && back.is_none() && !self.ivars().is_empty() {
                    true => self as *const Self as *mut AnyObject,
                    false => ptr::null_mut(),
                }
            }

            #[unsafe(method(writeSelectionToPasteboard:types:))]
            fn write_selection(&self, pasteboard: &NSPasteboard, _types: &NSArray<NSString>) -> bool {
                pasteboard.clearContents();
                let urls: Vec<_> = self.ivars().iter().map(|path| ProtocolObject::from_retained(file_url(path))).collect();
                pasteboard.writeObjects(&NSArray::from_retained_slice(&urls))
            }
        }
    );

    struct Shown {
        name: String,
        rgb: [u8; 3],
        ring: bool,
        have: Have,
        pick: isize,
    }

    struct DotsIvars {
        dots: Vec<Shown>,
        hover: Cell<Option<usize>>,
    }

    // 14 pt dots, 24 pt apart, as ForkLift's.
    const FIRST: f64 = 30.5;
    const PITCH: f64 = 24.;
    const RADIUS: f64 = 7.;
    const MIDDLE: f64 = 12.;
    const HEIGHT: f64 = 24.;
    const WIDTH: f64 = 200.;

    type Lines = &'static [&'static [(f64, f64)]];
    const CHECK: Lines = &[&[(-3.2, 0.2), (-1., 2.6), (3.4, -2.6)]];
    const DASH: Lines = &[&[(-3., 0.), (3., 0.)]];
    const PLUS: Lines = &[&[(-3.5, 0.), (3.5, 0.)], &[(0., -3.5), (0., 3.5)]];
    const CROSS: Lines = &[&[(-3., -3.), (3., 3.)], &[(-3., 3.), (3., -3.)]];

    define_class!(
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCFileMenuDots"]
        #[ivars = DotsIvars]
        struct Dots;

        impl Dots {
            #[unsafe(method(isFlipped))]
            fn is_flipped(&self) -> bool {
                true
            }

            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty: NSRect) {
                let ivars = self.ivars();
                for (ix, dot) in ivars.dots.iter().enumerate() {
                    let hover = ivars.hover.get() == Some(ix);
                    let centre = NSPoint::new(FIRST + PITCH * ix as f64, MIDDLE);
                    let radius = if hover { RADIUS + 1.5 } else { RADIUS };
                    let [r, g, b] = dot.rgb.map(|c| c as f64 / 255.);
                    let colour = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.);
                    if dot.ring {
                        let ring = oval(centre, radius - 0.75);
                        ring.setLineWidth(1.5);
                        colour.setStroke();
                        ring.stroke();
                    } else {
                        colour.setFill();
                        oval(centre, radius).fill();
                    }
                    let lines = match (hover, dot.have) {
                        (true, Have::All) => CROSS,
                        (true, _) => PLUS,
                        (false, Have::All) => CHECK,
                        (false, Have::Partly) => DASH,
                        (false, Have::Not) => continue,
                    };
                    match dot.ring {
                        true => colour.setStroke(),
                        false => NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.6).setStroke(),
                    }
                    for line in lines {
                        let path = NSBezierPath::new();
                        path.setLineWidth(1.5);
                        path.setLineCapStyle(NSLineCapStyle::Round);
                        for (n, (x, y)) in line.iter().enumerate() {
                            let point = NSPoint::new(centre.x + x, centre.y + y);
                            match n {
                                0 => path.moveToPoint(point),
                                _ => path.lineToPoint(point),
                            }
                        }
                        path.stroke();
                    }
                }
            }

            #[unsafe(method(mouseMoved:))]
            fn mouse_moved(&self, event: &NSEvent) {
                self.hover(self.dot_at(event));
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                self.hover(None);
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _event: &NSEvent) {}

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, event: &NSEvent) {
                let Some(ix) = self.dot_at(event) else {
                    return;
                };
                PICKED.set(Some(self.ivars().dots[ix].pick));
                if let Some(menu) = self.enclosingMenuItem().and_then(|item| unsafe { item.menu() }) {
                    menu.cancelTracking();
                }
            }
        }
    );

    impl Dots {
        fn new(dots: Vec<Shown>, mtm: MainThreadMarker) -> Retained<Self> {
            let ivars = DotsIvars { dots, hover: Cell::new(None) };
            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, HEIGHT));
            let view: Retained<Self> = unsafe { msg_send![super(Self::alloc(mtm).set_ivars(ivars)), initWithFrame: frame] };
            view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            track(&view);
            view
        }

        fn dot_at(&self, event: &NSEvent) -> Option<usize> {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            let ix = ((at.x - FIRST) / PITCH).round();
            let near = (at.x - FIRST - PITCH * ix).hypot(at.y - MIDDLE) <= RADIUS + 4.;
            (near && ix >= 0. && (ix as usize) < self.ivars().dots.len()).then_some(ix as usize)
        }

        fn hover(&self, dot: Option<usize>) {
            let ivars = self.ivars();
            if ivars.hover.replace(dot) == dot {
                return;
            }
            let tip = dot.map(|ix| match &ivars.dots[ix] {
                Shown { name, have: Have::All, .. } => format!("Remove tag “{name}”"),
                Shown { name, .. } => format!("Add tag “{name}”"),
            });
            self.setToolTip(tip.map(|tip| NSString::from_str(&tip)).as_deref());
            self.setNeedsDisplay(true);
        }
    }

    /// Mouse moves over `view`, while a menu or popover is open too.
    fn track(view: &NSView) {
        let options = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let owner: &AnyObject = view;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), NSRect::ZERO, options, Some(owner), None)
        };
        view.addTrackingArea(&area);
    }

    fn oval(centre: NSPoint, radius: f64) -> Retained<NSBezierPath> {
        let corner = NSPoint::new(centre.x - radius, centre.y - radius);
        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(corner, NSSize::new(2. * radius, 2. * radius)))
    }

    /// The menu at the mouse until closed; what was picked.
    pub(super) fn pop_up(entries: &[Entry], paths: &[PathBuf]) -> Option<Box<dyn Action>> {
        let mtm = MainThreadMarker::new()?;
        let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
        let mut picks = Vec::new();
        let menu = menu(entries, &target, &mut picks, mtm);
        PICKED.set(None);
        ANCHOR.set(Some(NSEvent::mouseLocation()));
        match right_click(paths, mtm) {
            // A context menu: AppKit adds the Services for the files.
            Some((event, requestor)) => {
                NSMenu::popUpContextMenu_withEvent_forView(&menu, &event, &requestor);
                requestor.removeFromSuperview();
            }
            None => {
                menu.popUpMenuPositioningItem_atLocation_inView(None, NSEvent::mouseLocation(), None);
            }
        }
        let picked = PICKED.take()?;
        picks.into_iter().nth(picked as usize)
    }

    /// A right click at the mouse, on a view offering `paths` to Services.
    fn right_click(paths: &[PathBuf], mtm: MainThreadMarker) -> Option<(Retained<NSEvent>, Retained<Requestor>)> {
        let app = NSApplication::sharedApplication(mtm);
        // AppKit asks a requestor only for the types the app sends.
        let types = FILE_TYPES.map(NSString::from_str);
        app.registerServicesMenuSendTypes_returnTypes(&NSArray::from_retained_slice(&types), &NSArray::new());
        let window = app.keyWindow()?;
        let at = window.convertPointFromScreen(NSEvent::mouseLocation());
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::RightMouseDown,
            at,
            NSEventModifierFlags::empty(),
            0.,
            window.windowNumber(),
            None,
            0,
            1,
            1.,
        )?;
        let requestor = Requestor::alloc(mtm).set_ivars(paths.to_vec());
        let requestor: Retained<Requestor> = unsafe { msg_send![super(requestor), initWithFrame: NSRect::ZERO] };
        window.contentView()?.addSubview(&requestor);
        Some((event, requestor))
    }

    fn menu(entries: &[Entry], target: &Target, picks: &mut Vec<Box<dyn Action>>, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        for entry in entries {
            let item = match entry {
                Entry::Separator => NSMenuItem::separatorItem(mtm),
                Entry::Submenu { title, entries } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(title));
                    item.setSubmenu(Some(&self::menu(entries, target, picks, mtm)));
                    item
                }
                Entry::Share(paths) => {
                    let urls: Vec<Retained<AnyObject>> = paths.iter().map(|path| file_url(path).into()).collect();
                    let urls = NSArray::from_retained_slice(&urls);
                    let picker = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &urls) };
                    picker.standardShareMenuItem(mtm)
                }
                Entry::Tags(dots) => {
                    let shown = dots
                        .iter()
                        .map(|Dot { name, rgb, ring, have, action }| {
                            picks.push(action.boxed_clone());
                            Shown { name: name.clone(), rgb: *rgb, ring: *ring, have: *have, pick: picks.len() as isize - 1 }
                        })
                        .collect();
                    let item = NSMenuItem::new(mtm);
                    item.setView(Some(&Dots::new(shown, mtm)));
                    item
                }
                Entry::Item { title, icon, action } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(title));
                    unsafe {
                        item.setTarget(Some(target as &AnyObject));
                        item.setAction(Some(sel!(pick:)));
                    }
                    item.setTag(picks.len() as isize);
                    picks.push(action.boxed_clone());
                    if let Some(image) = icon.as_ref().and_then(image) {
                        item.setImage(Some(&image));
                    }
                    item
                }
            };
            menu.addItem(&item);
        }
        menu
    }

    fn image(icon: &Icon) -> Option<Retained<NSImage>> {
        let image = match icon {
            Icon::Symbol(name) => NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?,
            Icon::File(path) => {
                let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
                image.setSize(NSSize::new(16., 16.));
                image
            }
        };
        Some(image)
    }

    /// Finder's label names, as the user named them; by label number.
    pub(super) fn label_names() -> Vec<String> {
        NSWorkspace::sharedWorkspace().fileLabels().iter().map(|name| name.to_string()).collect()
    }

    enum Row {
        Tag(Tag),
        ShowAll,
    }

    type Done = Box<dyn FnOnce(Vec<String>)>;

    struct AssignIvars {
        field: Retained<TagsField>,
        favourites: Vec<Tag>,
        /// Show All's besides.
        known: Vec<Tag>,
        all: Cell<bool>,
        rows: RefCell<Vec<(Row, Retained<NSTextField>)>>,
        hover: Cell<Option<usize>>,
        done: Cell<Option<Done>>,
    }

    define_class!(
        #[unsafe(super(NSTokenField, NSTextField, NSControl, NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCTagsField"]
        struct TagsField;

        impl TagsField {
            #[unsafe(method(becomeFirstResponder))]
            fn become_first_responder(&self) -> Bool {
                let became: Bool = unsafe { msg_send![super(self), becomeFirstResponder] };
                // After the tokens, not all of them chosen.
                if let Some(editor) = self.currentEditor() {
                    editor.setSelectedRange(NSRange::new(editor.string().length(), 0));
                }
                became
            }
        }
    );

    const RETURN: u16 = 36;
    const ENTER: u16 = 76;
    const ESCAPE: u16 = 53;
    const POPOVER: f64 = 232.;
    const LIST: f64 = 68.;
    const ROW: f64 = 20.;

    define_class!(
        /// Finder's Tags… popover: the tags as tokens, the others to add below.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PCAssignTags"]
        #[ivars = AssignIvars]
        struct AssignTags;

        impl AssignTags {
            #[unsafe(method(isFlipped))]
            fn is_flipped(&self) -> bool {
                true
            }

            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty: NSRect) {
                NSColor::separatorColor().setFill();
                NSBezierPath::fillRect(NSRect::new(NSPoint::new(12., LIST - 7.), NSSize::new(POPOVER - 24., 1.)));
                let ivars = self.ivars();
                for (ix, (row, _)) in ivars.rows.borrow().iter().enumerate() {
                    let top = LIST + ROW * ix as f64;
                    if ivars.hover.get() == Some(ix) {
                        let rect = NSRect::new(NSPoint::new(6., top), NSSize::new(POPOVER - 12., ROW));
                        NSColor::selectedContentBackgroundColor().setFill();
                        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 5., 5.).fill();
                    }
                    let centre = NSPoint::new(19., top + ROW / 2.);
                    match row {
                        Row::Tag(tag) => match sc_core::tags::rgb(tag.colour) {
                            Some([r, g, b]) => {
                                NSColor::colorWithSRGBRed_green_blue_alpha(r as f64 / 255., g as f64 / 255., b as f64 / 255., 1.)
                                    .setFill();
                                oval(centre, 4.5).fill();
                            }
                            None => {
                                let ring = oval(centre, 4.);
                                ring.setLineWidth(1.);
                                NSColor::secondaryLabelColor().setStroke();
                                ring.stroke();
                            }
                        },
                        Row::ShowAll => {
                            let rect = NSRect::new(NSPoint::new(centre.x - 6., centre.y - 4.), NSSize::new(12., 8.));
                            let capsule = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 4., 4.);
                            capsule.setLineWidth(1.2);
                            NSColor::secondaryLabelColor().setStroke();
                            capsule.stroke();
                        }
                    }
                }
            }

            #[unsafe(method(mouseMoved:))]
            fn mouse_moved(&self, event: &NSEvent) {
                self.hover(self.row_at(event));
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                self.hover(None);
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _event: &NSEvent) {}

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, event: &NSEvent) {
                if let Some(ix) = self.row_at(event) {
                    self.pick(ix);
                }
            }
        }

        unsafe impl NSObjectProtocol for AssignTags {}

        unsafe impl NSPopoverDelegate for AssignTags {
            #[unsafe(method(popoverWillClose:))]
            fn popover_will_close(&self, _notification: &NSNotification) {
                // What is still being typed counts too.
                if let Some(window) = self.window() {
                    window.makeFirstResponder(None);
                }
            }

            #[unsafe(method(popoverDidClose:))]
            fn popover_did_close(&self, _notification: &NSNotification) {
                let _alive = self.retain();
                if let Some(monitor) = MONITOR.take() {
                    unsafe { NSEvent::removeMonitor(&monitor) };
                }
                if let Some(done) = self.ivars().done.take() {
                    done(self.tokens());
                }
                OPEN.take();
            }
        }

        unsafe impl NSControlTextEditingDelegate for AssignTags {
            #[unsafe(method(controlTextDidChange:))]
            fn control_text_did_change(&self, _notification: &NSNotification) {
                self.refresh();
            }
        }

        unsafe impl NSTextFieldDelegate for AssignTags {}

        unsafe impl NSTokenFieldDelegate for AssignTags {}
    );

    impl AssignTags {
        fn tokens(&self) -> Vec<String> {
            let Some(value) = self.ivars().field.objectValue() else {
                return Vec::new();
            };
            let tags: Vec<String> = match value.downcast::<NSArray>() {
                Ok(tags) => tags.iter().filter_map(|tag| tag.downcast::<NSString>().ok()).map(|tag| tag.to_string()).collect(),
                Err(value) => value.downcast::<NSString>().map(|text| text.to_string().split(',').map(str::to_owned).collect()).unwrap_or_default(),
            };
            let mut tokens: Vec<String> = Vec::new();
            for tag in tags.iter().map(|tag| tag.trim()).filter(|tag| !tag.is_empty()) {
                if !tokens.iter().any(|seen| seen == tag) {
                    tokens.push(tag.to_owned());
                }
            }
            tokens
        }

        fn set_tokens(&self, tags: &[String]) {
            let names: Vec<_> = tags.iter().map(|tag| NSString::from_str(tag)).collect();
            let names = NSArray::from_retained_slice(&names);
            let value: &AnyObject = &names;
            unsafe { self.ivars().field.setObjectValue(Some(value)) };
        }

        /// The tags it has not yet, a row each; the popover as tall as they need.
        fn refresh(&self) {
            let ivars = self.ivars();
            let assigned = self.tokens();
            let mut tags: Vec<&Tag> = ivars.favourites.iter().collect();
            if ivars.all.get() {
                tags.extend(ivars.known.iter().filter(|tag| !ivars.favourites.iter().any(|f| f.name == tag.name)));
            }
            let mut rows: Vec<Row> =
                tags.into_iter().filter(|tag| !assigned.contains(&tag.name)).cloned().map(Row::Tag).collect();
            if !ivars.all.get() {
                rows.push(Row::ShowAll);
            }
            for (_, label) in ivars.rows.borrow().iter() {
                label.removeFromSuperview();
            }
            let rows: Vec<_> = rows
                .into_iter()
                .enumerate()
                .map(|(ix, row)| {
                    let text = match &row {
                        Row::Tag(tag) => tag.name.clone(),
                        Row::ShowAll => "Show All…".to_owned(),
                    };
                    let label = NSTextField::labelWithString(&NSString::from_str(&text), self.mtm());
                    label.setFont(Some(&NSFont::systemFontOfSize(13.)));
                    label.setFrame(NSRect::new(NSPoint::new(30., LIST + ROW * ix as f64 + 2.), NSSize::new(POPOVER - 42., 16.)));
                    self.addSubview(&label);
                    (row, label)
                })
                .collect();
            let size = NSSize::new(POPOVER, LIST + ROW * rows.len() as f64 + 8.);
            *ivars.rows.borrow_mut() = rows;
            ivars.hover.set(None);
            self.setFrameSize(size);
            if let Some(popover) = OPEN.with_borrow(Clone::clone) {
                popover.setContentSize(size);
            }
            self.setNeedsDisplay(true);
        }

        fn row_at(&self, event: &NSEvent) -> Option<usize> {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            let ix = ((at.y - LIST) / ROW).floor();
            let inside = at.y >= LIST && (6. ..=POPOVER - 6.).contains(&at.x);
            (inside && (ix as usize) < self.ivars().rows.borrow().len()).then_some(ix as usize)
        }

        fn hover(&self, row: Option<usize>) {
            let ivars = self.ivars();
            if ivars.hover.replace(row) == row {
                return;
            }
            for (ix, (_, label)) in ivars.rows.borrow().iter().enumerate() {
                let colour = if Some(ix) == row { NSColor::whiteColor() } else { NSColor::labelColor() };
                label.setTextColor(Some(&colour));
            }
            self.setNeedsDisplay(true);
        }

        /// Return makes what is typed a token, or with nothing typed closes, as Esc does.
        fn key(&self, event: &NSEvent) -> bool {
            let field = &self.ivars().field;
            let Some(window) = field.window().filter(|window| window.isKeyWindow()) else {
                return false;
            };
            let typing = field.currentEditor().is_some_and(|editor| editor.string().to_string().chars().any(|c| c != '\u{fffc}'));
            match event.keyCode() {
                RETURN | ENTER if typing => {
                    window.makeFirstResponder(None);
                    self.set_tokens(&self.tokens());
                    window.makeFirstResponder(Some(field));
                    self.refresh();
                }
                RETURN | ENTER | ESCAPE => {
                    if let Some(popover) = OPEN.with_borrow(Clone::clone) {
                        popover.close();
                    }
                }
                _ => return false,
            }
            true
        }

        fn pick(&self, ix: usize) {
            let ivars = self.ivars();
            let tag = match &ivars.rows.borrow()[ix].0 {
                Row::Tag(tag) => Some(tag.name.clone()),
                Row::ShowAll => None,
            };
            match tag {
                Some(tag) => self.set_tokens(&[self.tokens(), vec![tag]].concat()),
                None => ivars.all.set(true),
            }
            self.refresh();
        }
    }

    /// Finder's Tags…: a popover where the menu was, `assigned` edited; `done` when it closes.
    pub(super) fn assign_tags(
        what: &str,
        assigned: &[String],
        favourites: Vec<Tag>,
        known: Vec<Tag>,
        done: Done,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(window) = NSApplication::sharedApplication(mtm).keyWindow() else {
            return;
        };
        let Some(content) = window.contentView() else {
            return;
        };
        let frame = NSRect::new(NSPoint::new(12., 30.), NSSize::new(POPOVER - 24., 22.));
        let field: Retained<TagsField> = unsafe { msg_send![TagsField::alloc(mtm), initWithFrame: frame] };
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        let ivars = AssignIvars {
            field: field.clone(),
            favourites,
            known,
            all: Cell::new(false),
            rows: RefCell::new(Vec::new()),
            hover: Cell::new(None),
            done: Cell::new(Some(done)),
        };
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(POPOVER, LIST));
        let view: Retained<AssignTags> =
            unsafe { msg_send![super(AssignTags::alloc(mtm).set_ivars(ivars)), initWithFrame: frame] };
        let title = NSTextField::labelWithString(&NSString::from_str(&format!("Assign tags to {what}")), mtm);
        title.setFont(Some(&NSFont::boldSystemFontOfSize(11.)));
        title.setAlignment(NSTextAlignment::Center);
        title.setFrame(NSRect::new(NSPoint::new(12., 9.), NSSize::new(POPOVER - 24., 16.)));
        view.addSubview(&title);
        view.set_tokens(assigned);
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*view))) };
        view.addSubview(&field);
        track(&view);
        let controller = NSViewController::new(mtm);
        controller.setView(&view);
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);
        popover.setContentViewController(Some(&controller));
        popover.setDelegate(Some(ProtocolObject::from_ref(&*view)));
        OPEN.set(Some(popover.clone()));
        let keys = view.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| match keys.key(unsafe { event.as_ref() }) {
            true => ptr::null_mut(),
            false => event.as_ptr(),
        });
        MONITOR.set(unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) });
        view.refresh();
        let anchor = ANCHOR.get().unwrap_or_else(NSEvent::mouseLocation);
        let at = content.convertPoint_fromView(window.convertPointFromScreen(anchor), None);
        let rect = NSRect::new(NSPoint::new(at.x - 1., at.y - 9.), NSSize::new(2., 18.));
        let below = if content.isFlipped() { NSRectEdge::MaxY } else { NSRectEdge::MinY };
        popover.showRelativeToRect_ofView_preferredEdge(rect, &content, below);
        if let Some(window) = field.window() {
            window.makeFirstResponder(Some(&field));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{change_tags, tags_of};

    #[test]
    fn tags_files() {
        let file = std::env::temp_dir().join(format!("pc-tags-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let files = [file.clone()];
        change_tags(&files, &["Red".into(), "Work".into()], &[]).unwrap();
        assert_eq!(tags_of(&file), ["Red", "Work"]);
        change_tags(&files, &["Work".into()], &["Red".into()]).unwrap();
        assert_eq!(tags_of(&file), ["Work"]);
        std::fs::remove_file(&file).unwrap();
    }
}
