//! Properties (Alt+Enter): the cursor's entry, or on `..` the folder; a
//! folder's contents are counted while the dialog is open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::properties::{self, Kind, Properties};
use sc_core::space::{self, Space};
use sc_core::{grouped, human_size};

use crate::Commander;

impl Commander {
    /// This computer's files in the system's own dialog, as other file
    /// managers show them; ours for a server's, or when that fails.
    pub(crate) fn show_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_none() && !panel.in_archive() {
            let folder = panel.folder().to_owned();
            let paths: Vec<std::path::PathBuf> = match panel.chosen().as_slice() {
                [] => vec![folder],
                chosen => chosen.iter().map(|e| folder.join(&e.name)).collect(),
            };
            cx.spawn_in(window, async move |this, cx| {
                let shown = cx
                    .background_executor()
                    .spawn(async move { properties::show_in_system(&paths) })
                    .await;
                if shown.is_err() {
                    let _ = this.update_in(cx, |this, window, cx| this.own_properties(window, cx));
                }
            })
            .detach();
            return;
        }
        self.own_properties(window, cx);
    }

    fn own_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.in_archive() {
            let text = "The archive's listing shows what it knows of its files.";
            return window.push_notification(Notification::warning(text), cx);
        }
        let path = match panel.current_entry() {
            Some(entry) if !entry.is_parent() => panel.folder().join(&entry.name),
            _ => panel.folder().to_owned(),
        };
        let (fs, local) = (panel.fs(), panel.remote().is_none());
        let location = match panel.remote() {
            Some(remote) => format!(
                "{}{}",
                remote.url(),
                path.parent().unwrap_or(&path).display()
            ),
            None => path.parent().unwrap_or(&path).display().to_string(),
        };
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.display().to_string(),
        );
        let commander = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let reading = (fs.clone(), path.clone());
            let read = cx
                .background_executor()
                .spawn(async move { properties::of(&*reading.0, local, &reading.1) })
                .await;
            let _ = cx.update(|window, cx| {
                let properties = match read {
                    Ok(properties) => properties,
                    Err(err) => {
                        let text = format!("{}: {err}", path.display());
                        return window.push_notification(Notification::error(text), cx);
                    }
                };
                // A folder's contents, counted while the dialog is open.
                let contents = properties.is_dir.then(|| {
                    let counted = Arc::new(Mutex::new((Space::default(), false)));
                    let stop = Arc::new(AtomicBool::new(false));
                    let (counting, stopping) = (counted.clone(), stop.clone());
                    std::thread::spawn(move || {
                        let mut space = Space::default();
                        let mut seen = |space: &Space| counting.lock().unwrap().0 = *space;
                        let _ = space::measure(&*fs, &path, &mut space, &stopping, &mut seen);
                        // The folder itself is not in its contents.
                        space.dirs = space.dirs.saturating_sub(1);
                        *counting.lock().unwrap() = (space, true);
                    });
                    (counted, stop)
                });
                let view = cx.new(|cx| {
                    if contents.is_some() {
                        cx.spawn(async move |this, cx| {
                            loop {
                                cx.background_executor()
                                    .timer(Duration::from_millis(150))
                                    .await;
                                let done = this.update(cx, |this: &mut PropertiesDialog, cx| {
                                    cx.notify();
                                    this.contents
                                        .as_ref()
                                        .is_none_or(|(c, _)| c.lock().unwrap().1)
                                });
                                if done.unwrap_or(true) {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    PropertiesDialog {
                        name,
                        location,
                        properties,
                        contents,
                        commander,
                    }
                });
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog
                        .title("Properties")
                        .width(px(520.))
                        .child(view.clone())
                });
            });
        })
        .detach();
    }
}

type Counted = Arc<Mutex<(Space, bool)>>;

struct PropertiesDialog {
    name: String,
    location: String,
    properties: Properties,
    contents: Option<(Counted, Arc<AtomicBool>)>,
    commander: WeakEntity<Commander>,
}

impl Drop for PropertiesDialog {
    fn drop(&mut self) {
        if let Some((_, stop)) = &self.contents {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

fn time(time: Option<SystemTime>) -> Option<String> {
    time.map(sc_core::dates::date_time_seconds)
}

impl Render for PropertiesDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = &self.properties;
        let line = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .items_start()
                .child(div().w(px(100.)).flex_none().child(label))
                .child(div().flex_1().min_w_0().child(value))
        };
        let kind = match &p.kind {
            Kind::File => "File".to_string(),
            Kind::Folder => "Folder".into(),
            Kind::Link(target) if target.is_empty() => "Link".into(),
            Kind::Link(target) => format!("Link to {target}"),
        };
        let size = match &self.contents {
            Some((counted, _)) => {
                let (space, done) = *counted.lock().unwrap();
                let more = if done { "" } else { " (counting…)" };
                format!(
                    "{} file(s), {} folder(s), {} bytes ({}){more}",
                    grouped(space.files),
                    grouped(space.dirs),
                    grouped(space.bytes),
                    human_size(space.bytes)
                )
            }
            None => format!("{} bytes ({})", grouped(p.size), human_size(p.size)),
        };
        let permissions = p.mode.map(|mode| {
            let text: String = sc_core::attributes::BITS
                .iter()
                .map(|(bit, letter)| if mode & bit != 0 { *letter } else { '-' })
                .collect();
            format!("{text} ({:03o})", mode & 0o777)
        });
        let owner = match (&p.owner, &p.group) {
            (Some(owner), Some(group)) => Some(format!("{owner} / {group}")),
            _ => None,
        };
        let commander = self.commander.clone();
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(line("Name:", self.name.clone()))
            .child(line("Location:", self.location.clone()))
            .child(line("Type:", kind))
            .child(line(
                if self.contents.is_some() {
                    "Contents:"
                } else {
                    "Size:"
                },
                size,
            ))
            .children(time(p.modified).map(|t| line("Modified:", t)))
            .children(time(p.accessed).map(|t| line("Accessed:", t)))
            .children(time(p.created).map(|t| line("Created:", t)))
            .children(permissions.map(|t| line("Permissions:", t)))
            .children(owner.map(|t| line("Owner / group:", t)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when(p.mode.is_some(), |el| {
                        el.child(
                            Button::new("properties-attributes")
                                .outline()
                                .label("Change attributes...")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = commander.update(cx, |commander, cx| {
                                        commander.change_attributes(window, cx)
                                    });
                                }),
                        )
                    })
                    .child(
                        Button::new("properties-close")
                            .primary()
                            .label("Close")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
