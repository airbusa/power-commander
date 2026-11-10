//! Compare By Content: two marked files, or the cursor's file and its
//! namesake (else the cursor's file) in the other panel. Text side by side
//! in a window of its own, scrolling together; other files byte by byte.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::compare::{self, Compared, Kind, Row};
use pc_core::{Panel, grouped, vfs};

use crate::Commander;

impl Commander {
    pub(crate) fn compare_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = self.panels[self.active].tabs.current();
        let other = self.panels[1 - self.active].tabs.current();
        let Some((left, right)) = pair(this, other) else {
            let text = "Mark two files, or put the cursor on a file in each panel.";
            return window.push_notification(Notification::warning(text), cx);
        };
        if this.in_archive() || other.in_archive() {
            let text = "Files inside archives cannot be compared yet.";
            return window.push_notification(Notification::warning(text), cx);
        }
        // File names; with their folders when they are the same.
        let name = |path: &PathBuf, folder: bool| {
            let file = path.file_name().unwrap_or_default().display().to_string();
            let parent = path
                .parent()
                .and_then(|p| p.file_name())
                .unwrap_or_default();
            match folder {
                true => format!("{}/{file}", parent.display()),
                false => file,
            }
        };
        let same = left.1.file_name() == right.1.file_name();
        let names = (name(&left.1, same), name(&right.1, same));
        let bounds = window.window_bounds();
        cx.spawn_in(window, async move |_, cx| {
            let compared = cx
                .background_executor()
                .spawn(async move {
                    // A server's file is compared from a copy.
                    let local = |(fs, path): (std::sync::Arc<dyn vfs::Fs>, PathBuf)| match fs.id()
                        == "local"
                    {
                        true => Ok(path),
                        false => vfs::fetch(&*fs, &path),
                    };
                    compare::compare_files(&local(left)?, &local(right)?)
                })
                .await;
            let _ = cx.update(|window, cx| match compared {
                Err(err) => window.push_notification(Notification::error(err.to_string()), cx),
                Ok(Compared::Bytes { first: None, .. }) => {
                    window.push_notification(Notification::success("The files are identical."), cx)
                }
                Ok(Compared::Bytes {
                    first: Some(at),
                    differing,
                    sizes,
                }) => {
                    let text = format!(
                        "The files differ from byte {} on: {} byte(s) differ; sizes {} and {}.",
                        grouped(at),
                        grouped(differing),
                        grouped(sizes.0),
                        grouped(sizes.1)
                    );
                    window.push_notification(Notification::warning(text).autohide(false), cx)
                }
                Ok(Compared::Text(rows)) if compare::differences(&rows).is_empty() => {
                    let text = "The files have the same text.";
                    window.push_notification(Notification::success(text), cx)
                }
                Ok(Compared::Text(rows)) => {
                    if let Err(err) = open(rows, names, bounds, cx) {
                        window.push_notification(Notification::error(err), cx);
                    }
                }
            });
        })
        .detach();
    }
}

type Side = (std::sync::Arc<dyn vfs::Fs>, PathBuf);

/// The two files: two marked here, else the cursor's and the other panel's.
fn pair(this: &Panel, other: &Panel) -> Option<(Side, Side)> {
    let file = |panel: &Panel, name: &std::ffi::OsStr| (panel.fs(), panel.folder().join(name));
    let marked: Vec<_> = this.chosen().into_iter().filter(|e| !e.is_dir).collect();
    if let [a, b] = marked.as_slice() {
        return Some((file(this, &a.name), file(this, &b.name)));
    }
    let here = this
        .current_entry()
        .filter(|e| !e.is_dir && !e.is_parent())?;
    let namesake = other
        .entries()
        .iter()
        .find(|e| !e.is_dir && e.name == here.name);
    let there = namesake.or(other
        .current_entry()
        .filter(|e| !e.is_dir && !e.is_parent()))?;
    Some((file(this, &here.name), file(other, &there.name)))
}

fn open(
    rows: Vec<Row>,
    names: (String, String),
    bounds: WindowBounds,
    cx: &mut App,
) -> Result<(), String> {
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some(format!("Compare: {} — {}", names.0, names.1).into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let differences = compare::differences(&rows);
            CompareView {
                rows,
                differences,
                at: None,
                names,
                scroll: UniformListScrollHandle::new(),
                focus,
            }
        })
    })
    .map(drop)
    .map_err(|err| err.to_string())
}

struct CompareView {
    rows: Vec<Row>,
    /// Rows where a run of differences starts.
    differences: Vec<usize>,
    /// The difference last gone to.
    at: Option<usize>,
    names: (String, String),
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl CompareView {
    fn go(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.differences.len();
        if count == 0 {
            return;
        }
        let next = match (self.at, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(at), true) => (at + 1).min(count - 1),
            (Some(at), false) => at.saturating_sub(1),
        };
        self.at = Some(next);
        let row = self.differences[next].saturating_sub(3);
        self.scroll.scroll_to_item(row, ScrollStrategy::Top);
        cx.notify();
    }

    fn render_row(&self, ix: usize, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[ix];
        let tint = |color: Hsla| color.opacity(0.18);
        let (left_bg, right_bg) = match row.kind {
            Kind::Same => (None, None),
            Kind::Changed => (Some(tint(theme.warning)), Some(tint(theme.warning))),
            Kind::Removed => (Some(tint(theme.danger)), None),
            Kind::Added => (None, Some(tint(theme.success))),
        };
        let side = |line: &Option<(usize, String)>, bg: Option<Hsla>| {
            let (number, text) = match line {
                Some((number, text)) => (number.to_string(), text.replace('\t', "    ")),
                None => (String::new(), String::new()),
            };
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .when_some(bg, |el, bg| el.bg(bg))
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .pr_2()
                        .text_right()
                        .text_color(theme.muted_foreground)
                        .child(number),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(text),
                )
        };
        h_flex()
            .h(px(20.))
            .w_full()
            .child(side(&row.left, left_bg))
            .child(div().w(px(1.)).h_full().bg(theme.border))
            .child(side(&row.right, right_bg))
            .into_any_element()
    }
}

impl Render for CompareView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let count = self.differences.len();
        let place = match self.at {
            Some(at) => format!("Difference {} of {count}", at + 1),
            None => format!("{count} difference(s)"),
        };
        let list = uniform_list(
            "compare-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "n" | "f8" if !event.keystroke.modifiers.shift => this.go(true, cx),
                    "p" | "f8" => this.go(false, cx),
                    "escape" | "q" => window.remove_window(),
                    _ => {}
                }
            }))
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.names.0.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.names.1.clone()),
                    )
                    .child(place)
                    .child(
                        Button::new("compare-previous")
                            .small()
                            .outline()
                            .label("Previous (P)")
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.go(false, cx))),
                    )
                    .child(
                        Button::new("compare-next")
                            .small()
                            .primary()
                            .label("Next (N)")
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.go(true, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .child(list),
            )
    }
}
