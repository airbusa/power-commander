//! Pack (Alt+F5), Unpack Specific Files (Alt+F9) and Test Archives
//! (Alt+Shift+F9): their dialogs; the work runs in the queue.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::ops::Queue;
use pc_core::packing::{self, Format, Pack, Unpack};
use pc_core::{Panel, archive, command};

use crate::Commander;

const LEVELS: [(u8, &str); 5] = [
    (0, "Store"),
    (1, "Fastest"),
    (5, "Normal"),
    (7, "Maximum"),
    (9, "Ultra"),
];

impl Commander {
    /// The other panel's folder, where packing and unpacking go; this one's
    /// if that one is on a server or in an archive.
    pub(crate) fn local_target(&self) -> PathBuf {
        let other = self.panels[1 - self.active].tabs.current();
        let this = self.panels[self.active].tabs.current();
        match other.remote().is_none() && !other.in_archive() {
            true => other.folder().to_owned(),
            false => this.folder().to_owned(),
        }
    }

    /// The chosen entries on this computer, outside archives; else why not.
    pub(crate) fn local_chosen(&self, what: &str) -> Result<(&Panel, Vec<OsString>), String> {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return Err(format!(
                "{what} works on this computer's files, outside archives."
            ));
        }
        let names: Vec<OsString> = panel.chosen().iter().map(|e| e.name.clone()).collect();
        Ok((panel, names))
    }

    pub(crate) fn pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Packing") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        if names.is_empty() {
            return;
        }
        let folder = panel.folder().to_owned();
        let base = match names.as_slice() {
            [one] => one.to_string_lossy().into_owned(),
            _ => folder
                .file_name()
                .map_or("archive".into(), |name| name.to_string_lossy().into_owned()),
        };
        let format = Format::Zip;
        let target = self.local_target().join(format.rename(&base));
        let label = format!("Pack {} item(s) into the archive:", names.len());
        let queue = self.transfers.queue.clone();
        let target =
            cx.new(|cx| InputState::new(window, cx).default_value(target.display().to_string()));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let volume = cx.new(|cx| InputState::new(window, cx).default_value("100"));
        let view = cx.new(|_| PackDialog {
            label,
            target: target.clone(),
            format,
            level: 5,
            move_files: false,
            separate: false,
            split: false,
            volume,
            encrypt: false,
            password,
            folder,
            names,
            queue,
        });
        open(view, "Pack files", window, cx, |this, window, cx| {
            this.finish(window, cx)
        });
        focus_all(&target, window, cx);
    }

    pub(crate) fn unpack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Unpacking") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let archives: Vec<PathBuf> = names
            .iter()
            .filter(|name| archive::is_archive(name))
            .map(|name| panel.folder().join(name))
            .collect();
        if archives.is_empty() {
            let text = "Choose the archives to unpack.";
            return window.push_notification(Notification::warning(text), cx);
        }
        let label = format!("Unpack {} archive(s) to:", archives.len());
        let target = self.local_target().display().to_string();
        let target = cx.new(|cx| InputState::new(window, cx).default_value(target));
        let mask = cx.new(|cx| InputState::new(window, cx).default_value("*"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let queue = self.transfers.queue.clone();
        let base = panel.folder().to_owned();
        let view = cx.new(|_| UnpackDialog {
            label,
            target: target.clone(),
            mask,
            with_paths: true,
            overwrite: false,
            separate: archives.len() > 1,
            password,
            archives,
            base,
            queue,
        });
        open(view, "Unpack files", window, cx, |this, window, cx| {
            this.finish(window, cx)
        });
        focus_all(&target, window, cx);
    }

    pub(crate) fn test_archives(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("Testing") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let archives: Vec<PathBuf> = names
            .iter()
            .filter(|name| archive::is_archive(name))
            .map(|name| panel.folder().join(name))
            .collect();
        if archives.is_empty() {
            let text = "Choose the archives to test.";
            return window.push_notification(Notification::warning(text), cx);
        }
        // shortcut: no password to give; an encrypted archive reports that it needs one.
        self.transfers
            .queue
            .push_task(packing::test(archives, None));
    }
}

/// A dialog of `view`; OK runs `finish`, which keeps it open by returning false.
pub(crate) fn open<V: Render>(
    view: Entity<V>,
    title: &'static str,
    window: &mut Window,
    cx: &mut App,
    finish: fn(&mut V, &mut Window, &mut Context<V>) -> bool,
) {
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        dialog
            .title(title)
            .width(px(600.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| ok.update(cx, |this, cx| finish(this, window, cx)))
    });
}

/// After the menu that opened the dialog has given the focus back to the panel.
pub(crate) fn focus_all(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let input = input.clone();
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    });
}

pub(crate) fn buttons<V: 'static>(
    id: &'static str,
    cx: &mut Context<V>,
    finish: fn(&mut V, &mut Window, &mut Context<V>) -> bool,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .justify_end()
        .child(
            Button::new((id, 0usize))
                .primary()
                .label("OK")
                .min_w(px(80.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if finish(this, window, cx) {
                        window.close_dialog(cx);
                    }
                })),
        )
        .child(
            Button::new((id, 1usize))
                .outline()
                .label("Cancel")
                .min_w(px(80.))
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
}

pub(crate) fn check<V: 'static>(
    id: &'static str,
    label: &'static str,
    on: bool,
    cx: &mut Context<V>,
    set: fn(&mut V, bool),
) -> Checkbox {
    Checkbox::new(id)
        .label(label)
        .checked(on)
        .on_click(cx.listener(move |this, on: &bool, _, cx| {
            set(this, *on);
            cx.notify();
        }))
}

struct PackDialog {
    label: String,
    target: Entity<InputState>,
    format: Format,
    level: u8,
    move_files: bool,
    separate: bool,
    split: bool,
    volume: Entity<InputState>,
    encrypt: bool,
    password: Entity<InputState>,
    folder: PathBuf,
    names: Vec<OsString>,
    queue: Queue,
}

impl PackDialog {
    /// The target's name follows the format.
    fn set_format(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        self.format = format;
        let text = self.target.read(cx).value().to_string();
        let path = Path::new(&text);
        let renamed = match path.file_name() {
            Some(name) => path.with_file_name(format.rename(&name.to_string_lossy())),
            None => path.to_owned(),
        };
        let renamed = renamed.display().to_string();
        self.target
            .update(cx, |input, cx| input.set_value(renamed, window, cx));
        cx.notify();
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.target.read(cx).value().to_string();
        let archive = command::resolve(text.trim(), &self.folder);
        let password = self.password.read(cx).value().to_string();
        let encrypt = self.encrypt && self.format.encrypts();
        let fail = |err: &str, window: &mut Window, cx: &mut Context<Self>| {
            window.push_notification(Notification::error(err.to_string()), cx);
            false
        };
        if encrypt && password.is_empty() {
            return fail("Type the password, or leave Encrypt off.", window, cx);
        }
        let volume_mb = match self.split && !self.format.compressed_tar() {
            false => None,
            true => match self.volume.read(cx).value().trim().parse::<u64>() {
                Ok(mb) if mb > 0 => Some(mb),
                _ => return fail("The volume size is a number of MB.", window, cx),
            },
        };
        self.queue.push_task(packing::pack(Pack {
            archive,
            format: self.format,
            level: self.level,
            folder: self.folder.clone(),
            names: self.names.clone(),
            password: encrypt.then_some(password),
            volume_mb,
            move_files: self.move_files,
            separate: self.separate,
        }));
        true
    }
}

impl Render for PackDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let format = Format::ALL.iter().position(|f| *f == self.format);
        let level = LEVELS.iter().position(|(level, _)| *level == self.level);
        let tar = self.format.compressed_tar();
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
                    .child(div().w(px(100.)).child("Format:"))
                    .child(
                        RadioGroup::horizontal("pack-format")
                            .children(Format::ALL.map(Format::label))
                            .selected_index(format)
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.set_format(Format::ALL[*ix], window, cx)
                            })),
                    ),
            )
            .when(self.format != Format::Tar, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(100.)).flex_none().child("Compression:"))
                        .child(
                            RadioGroup::horizontal("pack-level")
                                .children(LEVELS.map(|(_, label)| label))
                                .selected_index(level)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.level = LEVELS[*ix].0;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(check(
                "pack-move",
                "Move to archive (delete the packed files)",
                self.move_files,
                cx,
                |this, on| this.move_files = on,
            ))
            .child(check(
                "pack-separate",
                "One archive for each selected file or folder",
                self.separate,
                cx,
                |this, on| this.separate = on,
            ))
            .when(!tar, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(check(
                            "pack-split",
                            "Split into volumes of",
                            self.split,
                            cx,
                            |this, on| this.split = on,
                        ))
                        .child(div().w(px(80.)).child(Input::new(&self.volume).small()))
                        .child("MB"),
                )
            })
            .when(self.format.encrypts(), |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(check(
                            "pack-encrypt",
                            "Encrypt with the password",
                            self.encrypt,
                            cx,
                            |this, on| this.encrypt = on,
                        ))
                        .child(div().flex_1().child(Input::new(&self.password).small())),
                )
            })
            .child(buttons("pack", cx, PackDialog::finish))
    }
}

struct UnpackDialog {
    label: String,
    target: Entity<InputState>,
    mask: Entity<InputState>,
    with_paths: bool,
    overwrite: bool,
    separate: bool,
    password: Entity<InputState>,
    archives: Vec<PathBuf>,
    /// Relative targets start here.
    base: PathBuf,
    queue: Queue,
}

impl UnpackDialog {
    fn finish(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.target.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        self.queue.push_task(packing::unpack(Unpack {
            archives: self.archives.clone(),
            target: command::resolve(text.trim(), &self.base),
            mask: self.mask.read(cx).value().to_string(),
            with_paths: self.with_paths,
            overwrite: self.overwrite,
            separate: self.separate,
            password: (!password.is_empty()).then_some(password),
        }));
        true
    }
}

impl Render for UnpackDialog {
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
                    .child(div().w(px(110.)).child("Only the files:"))
                    .child(div().flex_1().child(Input::new(&self.mask).small())),
            )
            .child(check(
                "unpack-paths",
                "Keep the folders stored in the archive",
                self.with_paths,
                cx,
                |this, on| this.with_paths = on,
            ))
            .child(check(
                "unpack-overwrite",
                "Overwrite existing files (else they are kept)",
                self.overwrite,
                cx,
                |this, on| this.overwrite = on,
            ))
            .child(check(
                "unpack-separate",
                "Each archive into a folder of its name",
                self.separate,
                cx,
                |this, on| this.separate = on,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(110.)).child("Password:"))
                    .child(div().flex_1().child(Input::new(&self.password).small())),
            )
            .child(buttons("unpack", cx, UnpackDialog::finish))
    }
}
