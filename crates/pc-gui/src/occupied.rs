//! Calculate Occupied Space (Ctrl+L): totals counted in the background and
//! shown as they grow; measured folders then show their size in the panel.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::space::{self, Space};
use pc_core::{grouped, human_size};

use crate::Commander;

/// What the counting thread found so far: the totals, each chosen
/// folder's size, and whether it is done.
#[derive(Default)]
struct Found {
    space: Space,
    folders: Vec<(OsString, u64)>,
    done: bool,
    error: Option<String>,
}

impl Commander {
    pub(crate) fn calculate_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        if panel.in_archive() {
            let text = "Inside archives the Size column has the sizes already.";
            return window.push_notification(Notification::warning(text), cx);
        }
        // On `..`, the whole folder.
        let mut chosen = panel.chosen();
        if chosen.is_empty() {
            chosen = panel.entries().iter().filter(|e| !e.is_parent()).collect();
        }
        let items: Vec<(OsString, bool)> = chosen
            .into_iter()
            .map(|entry| (entry.name.clone(), entry.is_dir))
            .collect();
        let (fs, folder) = (panel.fs(), panel.folder().to_owned());
        let found = Arc::new(Mutex::new(Found::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (counting, stopping) = (found.clone(), stop.clone());
        let walked = folder.clone();
        std::thread::spawn(move || {
            let mut space = Space::default();
            for (name, is_dir) in items {
                let before = space.bytes;
                let mut seen = |space: &Space| counting.lock().unwrap().space = *space;
                let path = walked.join(&name);
                match space::measure(&*fs, &path, &mut space, &stopping, &mut seen) {
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => return,
                    Err(err) => {
                        counting.lock().unwrap().error = Some(format!("{}: {err}", path.display()));
                    }
                    Ok(()) if is_dir => {
                        let mut found = counting.lock().unwrap();
                        found.folders.push((name, space.bytes - before));
                    }
                    Ok(()) => {}
                }
            }
            let mut found = counting.lock().unwrap();
            (found.space, found.done) = (space, true);
        });

        let commander = cx.entity().downgrade();
        let view = cx.new(|cx| {
            // Shows the totals as they grow; once done, the folders get their sizes.
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let done =
                        this.update(cx, |_, cx| cx.notify()).is_err() || found_done(&this, cx);
                    if done {
                        break;
                    }
                }
                let _ = this.update(cx, |this: &mut SpaceDialog, cx| {
                    let folders = std::mem::take(&mut this.found.lock().unwrap().folders);
                    let _ = commander.update(cx, |commander, cx| {
                        let panel = commander.panels[side].tabs.current_mut();
                        if panel.folder() == folder {
                            for (name, bytes) in folders {
                                panel.set_measured(name, bytes);
                            }
                        }
                        cx.notify();
                    });
                    cx.notify();
                });
            })
            .detach();
            SpaceDialog { found, stop }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Calculate occupied space")
                .width(px(420.))
                .child(view.clone())
        });
    }
}

fn found_done(this: &WeakEntity<SpaceDialog>, cx: &mut AsyncApp) -> bool {
    this.read_with(cx, |this, _| this.found.lock().unwrap().done)
        .unwrap_or(true)
}

struct SpaceDialog {
    found: Arc<Mutex<Found>>,
    stop: Arc<AtomicBool>,
}

/// Closing the dialog stops the counting.
impl Drop for SpaceDialog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Render for SpaceDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let found = self.found.lock().unwrap();
        let Space {
            files,
            dirs,
            bytes,
            unreadable,
        } = found.space;
        let line = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .child(div().w(px(110.)).child(label))
                .child(value)
        };
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(line("Files:", grouped(files)))
            .child(line("Folders:", grouped(dirs)))
            .child(line(
                "Total size:",
                format!("{} bytes ({})", grouped(bytes), human_size(bytes)),
            ))
            .when(unreadable > 0, |el| {
                el.child(format!("{unreadable} item(s) could not be read."))
            })
            .children(found.error.clone())
            .child(match found.done {
                true => "Done.",
                false => "Counting…",
            })
            .child(
                h_flex().justify_end().child(
                    Button::new("space-close")
                        .primary()
                        .label("Close")
                        .min_w(px(80.))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    }
}
