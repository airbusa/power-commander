//! F5 and F6: their dialog, the queue's progress strip and manager, and
//! the questions a running job asks.

use std::collections::HashSet;
use std::path::{MAIN_SEPARATOR, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress as Bar;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{ActiveTheme, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::Panel;
use pc_core::ops::{Answer, JobView, Kind, Operation, Overwrite, Question, Queue, Status, Target};
use pc_core::vfs::{Fs, Local, Meta};

use crate::Commander;

const OVERWRITE: [(Overwrite, &str); 5] = [
    (Overwrite::Ask, "Ask"),
    (Overwrite::All, "Overwrite all"),
    (Overwrite::Skip, "Skip all"),
    (Overwrite::Older, "Overwrite older"),
    (Overwrite::Rename, "Rename the copies"),
];

/// The queue, and what the main window last made of it.
#[derive(Default)]
pub struct Transfers {
    pub queue: Queue,
    version: u64,
    /// Finished jobs whose panels were read again.
    seen: HashSet<u64>,
    /// The job whose question is on screen.
    asked: Option<u64>,
    /// The running job, when it started, and its bytes then: for the speed.
    started: Option<(u64, Instant)>,
}

impl Commander {
    /// The chosen entries into the other panel's folder, by way of the queue.
    pub(crate) fn transfer(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let source = self.panels[side].tabs.current();
        let other = self.panels[1 - side].tabs.current();
        if other.in_archive() || source.in_archive() && kind == Kind::Move {
            let text =
                "Copying into archives and moving out of them are not possible yet; F5 copies out.";
            return window.push_notification(Notification::warning(text), cx);
        }
        let chosen = source.chosen();
        let folder = source.folder().to_owned();
        let sources: Vec<PathBuf> = chosen.iter().map(|e| folder.join(&e.name)).collect();
        let (what, target) = match chosen.as_slice() {
            [] => return,
            [one] if !one.is_dir => (
                format!("\"{}\"", one.display_name()),
                other.folder().join(&one.name).display().to_string(),
            ),
            [one] => (
                format!("\"{}\"", one.display_name()),
                folder_text(other.folder()),
            ),
            many => (format!("{} items", many.len()), folder_text(other.folder())),
        };
        let verb = match kind {
            Kind::Copy => "Copy",
            Kind::Move => "Move",
        };
        // Out of an archive: taken out first, then copied from there.
        let archived = source.in_archive().then(|| source.clone());
        let (from, to): (Arc<dyn Fs>, _) = match archived {
            Some(_) => (Arc::new(Local), other.fs()),
            None => (source.fs(), other.fs()),
        };
        let server = other.remote().map(|remote| remote.url());
        let label = match &server {
            Some(url) => format!("{verb} {what} to {url}:"),
            None => format!("{verb} {what} to:"),
        };
        let commander = cx.entity().downgrade();
        open_dialog(
            verb,
            label,
            target,
            window,
            cx,
            move |text, overwrite, window, cx| {
                let _ = commander.update(cx, |this, cx| {
                    // The server's own address typed: its path.
                    let text = server
                        .as_deref()
                        .and_then(|url| text.strip_prefix(url))
                        .unwrap_or(&text);
                    let target = Target::parse(text, &folder, sources.len(), &*to);
                    let (from, to) = (from.clone(), to.clone());
                    let queue = this.transfers.queue.clone();
                    let push = move |sources| {
                        queue.push(Operation {
                            kind,
                            from,
                            sources,
                            to,
                            target,
                            overwrite,
                        })
                    };
                    match archived.clone() {
                        None => drop(push(sources.clone())),
                        Some(panel) => {
                            cx.spawn_in(window, async move |_, cx| {
                                let taken = cx
                                    .background_executor()
                                    .spawn(async move { panel.extract_chosen() })
                                    .await;
                                match taken {
                                    Ok(sources) => drop(push(sources)),
                                    Err(err) => {
                                        let text = format!("Could not take them out: {err}");
                                        let note = Notification::error(text).autohide(false);
                                        let _ = cx.update(|window, cx| {
                                            window.push_notification(note, cx)
                                        });
                                    }
                                }
                            })
                            .detach();
                        }
                    }
                    // Queued: the marks have done their work.
                    this.panels[side].tabs.current_mut().mark_all(false);
                    cx.notify();
                });
            },
        );
    }

    /// Checks the queue several times a second while the window lives.
    pub(crate) fn watch_queue(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let watched = this.update_in(cx, |this, window, cx| this.queue_changed(window, cx));
                if watched.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn queue_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transfers = &mut self.transfers;
        let version = transfers.queue.version();
        if version == transfers.version {
            return;
        }
        transfers.version = version;
        let jobs = transfers.queue.jobs();
        let running = jobs
            .iter()
            .find(|job| job.status == Status::Running)
            .map(|job| job.id);
        if transfers.started.map(|(id, _)| id) != running {
            transfers.started = running.map(|id| (id, Instant::now()));
        }
        let finished: Vec<u64> = jobs
            .iter()
            .filter(|job| job.status.finished())
            .map(|job| job.id)
            .collect();
        let newly = finished.iter().any(|id| !transfers.seen.contains(id));
        transfers.seen = finished.into_iter().collect();
        if newly {
            for ix in 0..self.panels.len() {
                // shortcut: a panel whose folder is gone keeps its list until it moves.
                let remote = self.panels[ix].tabs.current().remote().is_some();
                self.navigate_on(ix, remote, Panel::reload, window, cx);
            }
        }
        if self.transfers.asked.is_some_and(|id| {
            !jobs
                .iter()
                .any(|job| job.id == id && job.question.is_some())
        }) {
            self.transfers.asked = None;
        }
        if self.transfers.asked.is_none()
            && let Some(job) = jobs.iter().find(|job| job.question.is_some())
        {
            self.transfers.asked = Some(job.id);
            ask(&self.transfers.queue, job, window, cx);
        }
        cx.notify();
    }

    /// Above the button bar while jobs run or wait.
    pub(crate) fn render_transfers(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let jobs = self.transfers.queue.jobs();
        let waiting = jobs
            .iter()
            .filter(|job| job.status == Status::Waiting)
            .count();
        let job = jobs
            .iter()
            .find(|job| job.status == Status::Running)?
            .clone();
        let theme = cx.theme();
        let p = &job.progress;
        let percent = match p.total_bytes {
            0 => 0.,
            total => p.bytes as f32 * 100. / total as f32,
        };
        let state = match p.scanning {
            true => format!("counting: {} file(s)", p.total_files),
            false => format!("{} / {} file(s), {percent:.0}%", p.files, p.total_files),
        };
        let speed = self.transfers.started.and_then(|(_, at)| {
            let seconds = at.elapsed().as_secs_f64();
            (seconds > 1. && !p.scanning).then(|| {
                format!(
                    "{}/s",
                    pc_core::human_size((p.bytes as f64 / seconds) as u64)
                )
            })
        });
        let queue = self.transfers.queue.clone();
        Some(
            h_flex()
                .px_2()
                .py_1()
                .gap_3()
                .border_t_1()
                .border_color(theme.border)
                .text_sm()
                .child(
                    div()
                        .flex_none()
                        .max_w(px(380.))
                        .truncate()
                        .child(job.title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(80.))
                        .child(Bar::new("transfer-progress").value(percent)),
                )
                .child(div().flex_none().child(state))
                .children(speed)
                .child(
                    Button::new("transfer-queue")
                        .ghost()
                        .small()
                        .label(match waiting {
                            0 => "Queue...".to_string(),
                            n => format!("Queue ({n} waiting)..."),
                        })
                        .on_click(move |_, window, cx| open_queue(queue.clone(), window, cx)),
                ),
        )
    }
}

fn folder_text(folder: &std::path::Path) -> String {
    let mut text = folder.display().to_string();
    if !text.ends_with(MAIN_SEPARATOR) {
        text.push(MAIN_SEPARATOR);
    }
    text
}

type Done = Rc<dyn Fn(String, Overwrite, &mut Window, &mut App)>;

/// "Copy 3 items to:", the target, and what to do with files there already.
fn open_dialog(
    title: &'static str,
    label: String,
    target: String,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(String, Overwrite, &mut Window, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(target));
    let view = cx.new(|_| TransferDialog {
        label,
        input: input.clone(),
        overwrite: Overwrite::Ask,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        dialog
            .title(title)
            .width(px(560.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| {
                ok.update(cx, |this, cx| this.finish(window, cx));
                true
            })
    });
    input.update(cx, |input, cx| {
        input.focus(window, cx);
        input.select_all(window, cx);
    });
}

struct TransferDialog {
    label: String,
    input: Entity<InputState>,
    overwrite: Overwrite,
    done: Done,
}

impl TransferDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.input.read(cx).value().to_string();
        (self.done)(target, self.overwrite, window, cx);
    }
}

impl Render for TransferDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = OVERWRITE
            .iter()
            .position(|(mode, _)| *mode == self.overwrite);
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label.clone())
                    .child(Input::new(&self.input)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child("Where the target has a file of the same name:")
                    .child(
                        RadioGroup::vertical("transfer-overwrite")
                            .children(OVERWRITE.map(|(_, label)| label))
                            .selected_index(selected)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.overwrite = OVERWRITE[*ix].0;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("transfer-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("transfer-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

/// A running job's question; it waits until a button answers.
fn ask(queue: &Queue, job: &JobView, window: &mut Window, cx: &mut App) {
    let Some(question) = job.question.clone() else {
        return;
    };
    let (title, text, answers): (&str, String, &[(Answer, &str)]) = match &question {
        Question::Exists {
            target,
            source,
            existing,
        } => (
            "File exists",
            format!(
                "{}\n\nThere already: {}\nThe new one: {}",
                target.display(),
                describe(existing),
                describe(source)
            ),
            &[
                (Answer::Overwrite, "Overwrite"),
                (Answer::OverwriteAll, "Overwrite all"),
                (Answer::Skip, "Skip"),
                (Answer::SkipAll, "Skip all"),
                (Answer::Rename, "Rename"),
                (Answer::Cancel, "Cancel"),
            ],
        ),
        Question::Failed { path, error } => (
            "Error",
            format!("{}\n\n{error}", path.display()),
            &[
                (Answer::Retry, "Retry"),
                (Answer::Skip, "Skip"),
                (Answer::SkipAll, "Skip all"),
                (Answer::Cancel, "Cancel"),
            ],
        ),
    };
    let (queue, id) = (queue.clone(), job.id);
    let job_title = job.title.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let buttons = answers.iter().map(|&(answer, label)| {
            let queue = queue.clone();
            let button = Button::new(label)
                .label(label)
                .on_click(move |_, window, cx| {
                    queue.answer(id, answer);
                    window.close_dialog(cx);
                });
            match answer {
                Answer::Overwrite | Answer::Retry => button.primary(),
                Answer::Cancel => button.with_variant(ButtonVariant::Danger),
                _ => button.outline(),
            }
        });
        dialog
            .title(title)
            .width(px(560.))
            .keyboard(false)
            .close_button(false)
            .overlay_closable(false)
            .child(
                v_flex()
                    .gap_3()
                    .text_sm()
                    .child(div().text_xs().child(job_title.clone()))
                    .child(div().whitespace_normal().child(text.clone()))
                    .child(h_flex().gap_2().flex_wrap().justify_end().children(buttons)),
            )
    });
}

fn describe(meta: &Meta) -> String {
    let time = meta
        .modified
        .map(|time| pc_core::local_time(time, "%Y-%m-%d %H:%M:%S"));
    format!(
        "{} bytes, {}",
        pc_core::grouped(meta.size),
        time.unwrap_or_default()
    )
}

/// The jobs: the running one's progress; the waiting ones reordered or cancelled.
pub fn open_queue(queue: Queue, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        // Follows the queue while open.
        let poll = cx.spawn(async move |this: WeakEntity<QueueDialog>, cx| {
            let mut version = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let changed = this.update(cx, |this, cx| {
                    let now = this.queue.version();
                    if now != version {
                        version = now;
                        cx.notify();
                    }
                });
                if changed.is_err() {
                    break;
                }
            }
        });
        QueueDialog { queue, _poll: poll }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Operation queue")
            .width(px(640.))
            .child(view.clone())
    });
}

struct QueueDialog {
    queue: Queue,
    _poll: Task<()>,
}

impl Render for QueueDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let jobs = self.queue.jobs();
        let rows = jobs.iter().map(|job| {
            let (id, queue) = (job.id, self.queue.clone());
            let p = &job.progress;
            let state = match &job.status {
                Status::Waiting => "Waiting".to_string(),
                Status::Running if job.question.is_some() => "Asking".into(),
                Status::Running if p.scanning => "Counting".into(),
                Status::Running => match p.total_bytes {
                    0 => "Running".into(),
                    total => format!("{:.0}%", p.bytes as f64 * 100. / total as f64),
                },
                Status::Done if p.skipped > 0 => format!("Done, {} skipped", p.skipped),
                Status::Done => "Done".into(),
                Status::Cancelled => "Cancelled".into(),
            };
            let button = |icon: IconName, name: &'static str| {
                Button::new((name, id)).ghost().xsmall().icon(icon)
            };
            let waiting = job.status == Status::Waiting;
            let (up, down) = (queue.clone(), queue.clone());
            h_flex()
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().truncate().child(job.title.clone()))
                .child(
                    div()
                        .w(px(120.))
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(state),
                )
                .child(
                    h_flex()
                        .w(px(84.))
                        .flex_none()
                        .justify_end()
                        .when(waiting, |el| {
                            el.child(
                                button(IconName::ArrowUp, "queue-up")
                                    .on_click(move |_, _, _| up.reorder(id, -1)),
                            )
                            .child(
                                button(IconName::ArrowDown, "queue-down")
                                    .on_click(move |_, _, _| down.reorder(id, 1)),
                            )
                        })
                        .when(!job.status.finished(), |el| {
                            el.child(
                                button(IconName::Close, "queue-cancel")
                                    .on_click(move |_, _, _| queue.cancel(id)),
                            )
                        }),
                )
        });
        let queue = self.queue.clone();
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .id("queue-jobs")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .when(jobs.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child("No operations."),
                        )
                    })
                    .children(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("queue-clear")
                            .outline()
                            .label("Clear finished")
                            .on_click(move |_, _, _| queue.clear_finished()),
                    )
                    .child(
                        Button::new("queue-close")
                            .primary()
                            .label("Close")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
