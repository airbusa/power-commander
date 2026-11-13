//! Create Checksum Files, Verify Checksums, Split File, Combine Files and
//! Create Link: their dialogs; the work runs in the queue, links here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use pc_core::checksum::{self, Create, Method};
use pc_core::ops::Queue;
use pc_core::links::{self, Kind};
use pc_core::{command, split};

use crate::Commander;
use crate::pack_dialog::{buttons, check, focus_all, open};

/// Part sizes to pick from.
const SIZES: [&str; 5] = ["10 MB", "100 MB", "700 MB", "1 GB", "4095 MB"];

impl Commander {
    pub(crate) fn create_checksums(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Creating checksums") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        if names.is_empty() {
            return;
        }
        let folder = panel.folder().to_owned();
        let method = Method::Md5;
        let base = match names.as_slice() {
            [one] => one.to_string_lossy().into_owned(),
            _ => folder.file_name().map_or("checksums".into(), |name| {
                name.to_string_lossy().into_owned()
            }),
        };
        let file = folder.join(format!("{base}.{}", method.extension()));
        let file =
            cx.new(|cx| InputState::new(window, cx).default_value(file.display().to_string()));
        let queue = self.transfers.queue.clone();
        let view = cx.new(|_| ChecksumDialog {
            file: file.clone(),
            method,
            separate: false,
            folder,
            names,
            queue,
        });
        open(
            view,
            "Create checksum files",
            window,
            cx,
            ChecksumDialog::finish,
        );
        focus_all(&file, window, cx);
    }

    /// Ctrl+Shift+F5: links to the chosen in the other panel's folder.
    pub(crate) fn create_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Making links") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        if names.is_empty() {
            return;
        }
        let chosen = panel.chosen();
        let (folders, files) = (chosen.iter().any(|e| e.is_dir), chosen.iter().any(|e| !e.is_dir));
        let folder = panel.folder().to_owned();
        let into = self.local_target();
        let (label, at) = match names.as_slice() {
            [one] => (format!("Link to \"{}\" as:", one.to_string_lossy()), into.join(one)),
            many => (format!("Links to the {} items, in the folder:", many.len()), into),
        };
        let at = cx.new(|cx| InputState::new(window, cx).default_value(at.display().to_string()));
        let (commander, side) = (cx.entity().downgrade(), self.active);
        let view = cx.new(|_| LinkDialog {
            label,
            at: at.clone(),
            kind: Kind::Symbolic,
            folders,
            files,
            folder,
            names,
            commander,
            side,
        });
        open(view, "Create link", window, cx, LinkDialog::finish);
        focus_all(&at, window, cx);
    }

    pub(crate) fn verify_checksums(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Verifying checksums") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let files: Vec<PathBuf> = names
            .iter()
            .map(|name| panel.folder().join(name))
            .filter(|path| Method::of(path).is_some())
            .collect();
        if files.is_empty() {
            let text = "Choose checksum files: .sfv, .md5, .sha1, .sha256 or .sha512.";
            return window.push_notification(Notification::warning(text), cx);
        }
        self.transfers.queue.push_task(checksum::verify(files));
    }

    pub(crate) fn split_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, _) = match self.local_chosen("Splitting") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let Some(entry) = panel.current_entry().filter(|e| !e.is_dir) else {
            let text = "Put the cursor on the file to split.";
            return window.push_notification(Notification::warning(text), cx);
        };
        let file = panel.folder().join(&entry.name);
        let label = format!(
            "Split \"{}\" into parts in the folder:",
            entry.display_name()
        );
        let target = self.local_target().display().to_string();
        let target = cx.new(|cx| InputState::new(window, cx).default_value(target));
        let size = cx.new(|cx| InputState::new(window, cx).default_value(SIZES[1]));
        let base = panel.folder().to_owned();
        let queue = self.transfers.queue.clone();
        let view = cx.new(|_| SplitDialog {
            label,
            target: target.clone(),
            size,
            file,
            base,
            queue,
        });
        open(view, "Split file", window, cx, SplitDialog::finish);
        focus_all(&target, window, cx);
    }

    pub(crate) fn combine_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, _) = match self.local_chosen("Combining") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let parts = panel
            .current_entry()
            .and_then(|entry| split::parts_of(&panel.folder().join(&entry.name)));
        let Some(parts) = parts else {
            let text = "Put the cursor on a part (name.001) or on its .crc file.";
            return window.push_notification(Notification::warning(text), cx);
        };
        let target = self.local_target().join(&parts.name);
        let base = panel.folder().to_owned();
        let queue = self.transfers.queue.clone();
        let parts = std::cell::RefCell::new(Some(parts));
        crate::name_dialog::open(
            "Combine files",
            "Combine the parts into the file:",
            target.display().to_string(),
            window,
            cx,
            move |text, _, _| {
                if let Some(parts) = parts.borrow_mut().take() {
                    let target = command::resolve(text.trim(), &base);
                    queue.push_task(split::combine(parts, target));
                }
            },
        );
    }
}

struct LinkDialog {
    label: String,
    at: Entity<InputState>,
    kind: Kind,
    folders: bool,
    files: bool,
    folder: PathBuf,
    names: Vec<OsString>,
    commander: WeakEntity<Commander>,
    side: usize,
}

impl LinkDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.kind.fits(self.folders, self.files) {
            return false;
        }
        let at = command::resolve(self.at.read(cx).value().trim(), &self.folder);
        let items: Vec<(PathBuf, PathBuf)> = match self.names.as_slice() {
            // Into a folder named, as ln's.
            [one] if at.is_dir() && at != self.folder.join(one) => vec![(self.folder.join(one), at.join(one))],
            [one] => vec![(self.folder.join(one), at.clone())],
            many => many.iter().map(|name| (self.folder.join(name), at.join(name))).collect(),
        };
        let kind = self.kind;
        let made = links::create(kind, &items);
        let _ = self.commander.update(cx, |this, cx| {
            let into = match items.as_slice() {
                [(_, link)] => link.parent().unwrap_or(&at).to_owned(),
                _ => at,
            };
            match made {
                Err((err, rest)) if crate::denied(&err) && kind != Kind::Shortcut => {
                    let action = pc_core::elevate::Action::Link { kind, items: rest };
                    this.offer_admin(format!("Making the {}", kind.label().to_lowercase()), action, window, cx);
                }
                Err((err, _)) => window.push_notification(Notification::error(format!("Could not make the link: {err}")), cx),
                // The marks have done their work.
                Ok(()) => this.panels[self.side].tabs.current_mut().mark_all(false),
            }
            this.refresh_folder(&into, cx);
        });
        true
    }
}

impl Render for LinkDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds = Kind::ALL.iter().map(|kind| Radio::new(kind.label()).label(kind.label()).disabled(!kind.fits(self.folders, self.files)));
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(v_flex().gap_1().child(self.label.clone()).child(Input::new(&self.at)))
            .child(
                RadioGroup::horizontal("link-kind")
                    .children(kinds)
                    .selected_index(Kind::ALL.iter().position(|kind| *kind == self.kind))
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.kind = Kind::ALL[*ix];
                        cx.notify();
                    })),
            )
            .child(buttons("link", cx, LinkDialog::finish))
    }
}

struct ChecksumDialog {
    file: Entity<InputState>,
    method: Method,
    separate: bool,
    folder: PathBuf,
    names: Vec<OsString>,
    queue: Queue,
}

impl ChecksumDialog {
    /// The file's extension follows the method.
    fn set_method(&mut self, method: Method, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.file.read(cx).value().to_string();
        let path = Path::new(&text);
        let renamed = match Method::of(path) {
            Some(_) => path.with_extension(method.extension()),
            None => PathBuf::from(format!("{text}.{}", method.extension())),
        };
        let renamed = renamed.display().to_string();
        self.file
            .update(cx, |input, cx| input.set_value(renamed, window, cx));
        self.method = method;
        cx.notify();
    }

    fn finish(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.file.read(cx).value().to_string();
        self.queue.push_task(checksum::create(Create {
            folder: self.folder.clone(),
            names: self.names.clone(),
            method: self.method,
            file: command::resolve(text.trim(), &self.folder),
            separate: self.separate,
        }));
        true
    }
}

impl Render for ChecksumDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let method = Method::ALL.iter().position(|m| *m == self.method);
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child("Save the checksum file as:")
                    .child(Input::new(&self.file)),
            )
            .child(
                RadioGroup::horizontal("checksum-method")
                    .children(Method::ALL.map(Method::label))
                    .selected_index(method)
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        this.set_method(Method::ALL[*ix], window, cx)
                    })),
            )
            .child(check(
                "checksum-separate",
                "A checksum file beside each file instead",
                self.separate,
                cx,
                |this, on| this.separate = on,
            ))
            .child(buttons("checksum", cx, ChecksumDialog::finish))
    }
}

struct SplitDialog {
    label: String,
    target: Entity<InputState>,
    size: Entity<InputState>,
    file: PathBuf,
    base: PathBuf,
    queue: Queue,
}

impl SplitDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let size = self.size.read(cx).value().to_string();
        let Some(bytes) = split::parse_size(&size) else {
            let text = format!("\"{size}\" is no size: 100 MB, 1.5 GB, 650m");
            window.push_notification(Notification::error(text), cx);
            return false;
        };
        let text = self.target.read(cx).value().to_string();
        let dir = command::resolve(text.trim(), &self.base);
        self.queue
            .push_task(split::split(self.file.clone(), dir, bytes));
        true
    }
}

impl Render for SplitDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label.clone())
                    .child(Input::new(&self.target)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(80.)).child("Part size:"))
                    .child(div().w(px(110.)).child(Input::new(&self.size).small()))
                    .children(SIZES.iter().enumerate().map(|(ix, size)| {
                        Button::new(("split-size", ix))
                            .ghost()
                            .small()
                            .label(*size)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.size
                                    .update(cx, |input, cx| input.set_value(*size, window, cx));
                            }))
                    })),
            )
            .child(buttons("split", cx, SplitDialog::finish))
    }
}
