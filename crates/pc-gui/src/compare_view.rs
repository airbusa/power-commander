//! Compare By Content in a window of its own, as Total Commander's: the
//! two files' paths, text side by side scrolling together or bytes in hex,
//! next and previous difference, and the current row's two lines below.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Position};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::compare::{self, Compared, Kind, Options, Visible};
use pc_core::grouped;
use pc_core::origin::Origin;

use crate::Commander;

/// Rows shown around each difference with "Only differences".
const CONTEXT: usize = 3;
const ROW_HEIGHT: f32 = 20.;

impl Commander {
    pub(crate) fn compare_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = self.panels[self.active].tabs.current();
        let other = self.panels[1 - self.active].tabs.current();
        let Some(files) = compare::pair(this, other) else {
            let text = "Put the cursor on a file in each panel, or mark two files.";
            return window.push_notification(Notification::warning(text), cx);
        };
        // The active panel's file on its own side.
        let files = match self.active {
            0 => files,
            _ => {
                let [a, b] = files;
                [b, a]
            }
        };
        if let Err(err) = open(files.map(Arc::new), window.window_bounds(), cx) {
            window.push_notification(Notification::error(err), cx);
        }
    }
}

fn open(files: [Arc<Origin>; 2], bounds: WindowBounds, cx: &mut App) -> Result<(), String> {
    let name = |origin: &Origin| {
        let name = origin.name();
        let file = Path::new(&name).file_name().map(|n| n.display().to_string());
        file.unwrap_or(name)
    };
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some(format!("Compare: {} — {}", name(&files[0]), name(&files[1])).into()),
            ..Default::default()
        }),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        let inputs = files.each_ref().map(|origin| {
            let path = origin.name();
            // The cursor at the end shows the file's name in a long path.
            let end = Position::new(0, path.chars().count() as u32);
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).default_value(path);
                input.set_cursor_position(end, window, cx);
                input
            })
        });
        let view = cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            CompareView {
                files,
                inputs,
                options: Options::default(),
                only_differences: false,
                state: State::Comparing,
                generation: 0,
                runs: Vec::new(),
                visible: Visible::default(),
                current: None,
                at: None,
                scroll: UniformListScrollHandle::new(),
                focus,
            }
        });
        view.update(cx, |view, cx| view.compare(cx));
        view
    })
    .map(drop)
    .map_err(|err| err.to_string())
}

enum State {
    Comparing,
    Failed(String),
    Done(Compared),
}

struct CompareView {
    files: [Arc<Origin>; 2],
    inputs: [Entity<InputState>; 2],
    options: Options,
    only_differences: bool,
    state: State,
    /// Counts comparisons, so an earlier one's late result is dropped.
    generation: usize,
    runs: Vec<Range<usize>>,
    visible: Visible,
    /// The row shown below, whole.
    current: Option<usize>,
    /// The difference last gone to.
    at: Option<usize>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl CompareView {
    /// Compares the files the paths name: as opened, or typed or chosen since.
    fn compare(&mut self, cx: &mut Context<Self>) {
        for side in 0..2 {
            let typed = self.inputs[side].read(cx).value().trim().to_string();
            if typed != self.files[side].name() {
                self.files[side] = Arc::new(Origin::Local(PathBuf::from(typed)));
            }
        }
        self.generation += 1;
        let (generation, options) = (self.generation, self.options);
        let files = self.files.clone();
        self.state = State::Comparing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let compared = cx
                .background_executor()
                .spawn(async move {
                    let local = |origin: &Origin| {
                        let named = |err: std::io::Error| format!("{}: {err}", origin.name());
                        origin.view().map_err(named)
                    };
                    let (left, right) = (local(&files[0])?, local(&files[1])?);
                    compare::compare_files(&left, &right, options).map_err(|err| err.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.finish(compared, cx);
                }
            });
        })
        .detach();
    }

    fn finish(&mut self, compared: Result<Compared, String>, cx: &mut Context<Self>) {
        self.state = match compared {
            Ok(compared) => {
                self.runs = compared.runs();
                State::Done(compared)
            }
            Err(err) => {
                self.runs.clear();
                State::Failed(err)
            }
        };
        (self.current, self.at) = (None, None);
        self.show_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn rows(&self) -> usize {
        match &self.state {
            State::Done(compared) => compared.rows(),
            _ => 0,
        }
    }

    fn show_rows(&mut self) {
        let context = self.only_differences.then_some(CONTEXT);
        self.visible = Visible::new(self.rows(), &self.runs, context);
    }

    fn go(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.runs.len();
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
        let row = self.runs[next].start;
        self.current = Some(row);
        if let Some(ix) = self.visible.index_of(row) {
            self.scroll
                .scroll_to_item(ix.saturating_sub(3), ScrollStrategy::Top);
        }
        cx.notify();
    }

    /// The current row moves by `by` shown rows, over gaps.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.visible.len();
        if count == 0 {
            return;
        }
        let from = self.current.and_then(|row| self.visible.index_of(row));
        let mut ix = match from {
            Some(ix) => ix.saturating_add_signed(by).min(count - 1),
            None => 0,
        };
        while self.visible.row(ix).is_none() {
            ix = (ix + 1).min(count - 1);
        }
        self.current = self.visible.row(ix);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn set_options(&mut self, set: impl FnOnce(&mut Options), cx: &mut Context<Self>) {
        set(&mut self.options);
        self.compare(cx);
    }

    fn browse(&mut self, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Compare".into()),
        });
        let input = self.inputs[side].clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let path = paths[0].display().to_string();
            let _ = this.update_in(cx, |this, window, cx| {
                input.update(cx, |input, cx| input.set_value(path, window, cx));
                this.compare(cx);
            });
        })
        .detach();
    }

    /// The row's two sides: text, the parts that differ, and the tint.
    fn sides(&self, row: usize, cx: &App) -> [Side; 2] {
        let theme = cx.theme();
        let tint = |color: Hsla| Some(color.opacity(0.18));
        match &self.state {
            State::Done(Compared::Text(rows)) => {
                let row = &rows[row];
                let text = |line: &Option<(usize, String)>| match line {
                    Some((number, text)) => (number.to_string(), text.replace('\t', "    ")),
                    None => (String::new(), String::new()),
                };
                let (left, right) = (text(&row.left), text(&row.right));
                let parts = match row.kind {
                    Kind::Changed => compare::changed_parts(&left.1, &right.1),
                    _ => [Vec::new(), Vec::new()],
                };
                let (left_bg, right_bg) = match row.kind {
                    Kind::Same => (None, None),
                    Kind::Changed => (tint(theme.warning), tint(theme.warning)),
                    Kind::Removed => (tint(theme.danger), None),
                    Kind::Added => (None, tint(theme.success)),
                };
                let [left_parts, right_parts] = parts;
                [
                    Side::new(left, left_parts, left_bg),
                    Side::new(right, right_parts, right_bg),
                ]
            }
            State::Done(Compared::Bytes(bytes)) => {
                let run = self.runs.partition_point(|run| run.end <= row);
                let differs = self.runs.get(run).is_some_and(|run| run.contains(&row));
                let bg = differs.then(|| theme.danger.opacity(0.12));
                bytes
                    .row(row)
                    .map(|(text, parts)| Side::new((String::new(), text), parts, bg))
            }
            _ => [Side::default(), Side::default()],
        }
    }

    fn render_side(&self, side: Side, current: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let mark = HighlightStyle {
            background_color: Some(theme.danger.opacity(0.35)),
            ..Default::default()
        };
        let text = StyledText::new(side.text)
            .with_highlights(side.parts.into_iter().map(|range| (range, mark)));
        let numbered = !side.number.is_empty();
        h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .when_some(side.bg, |el, bg| el.bg(bg))
            .when(numbered, |el| {
                el.child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .pr_2()
                        .text_right()
                        .text_color(theme.muted_foreground)
                        .when(current, |el| {
                            el.bg(theme.accent).text_color(theme.accent_foreground)
                        })
                        .child(side.number),
                )
            })
            .when(!numbered && current, |el| {
                el.child(div().w(px(4.)).h_full().flex_none().bg(theme.accent))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(text),
            )
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(row) = self.visible.row(ix) else {
            return div()
                .h(px(ROW_HEIGHT))
                .w_full()
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .text_center()
                .child("⋯")
                .into_any_element();
        };
        let current = self.current == Some(row);
        let [left, right] = self.sides(row, cx);
        let theme = cx.theme();
        h_flex()
            .id(("compare-row", ix))
            .h(px(ROW_HEIGHT))
            .w_full()
            .child(self.render_side(left, current, cx))
            .child(div().w(px(1.)).h_full().bg(theme.border))
            .child(self.render_side(right, current, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.current = Some(row);
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    fn status(&self) -> String {
        let count = self.runs.len();
        let place = match self.at {
            Some(at) => format!("Difference {} of {count}", at + 1),
            None => format!("{count} difference(s)"),
        };
        match &self.state {
            State::Comparing => "Comparing…".into(),
            State::Failed(err) => err.clone(),
            State::Done(Compared::Text(_)) if count == 0 => "No differences.".into(),
            State::Done(Compared::Text(_)) => place,
            State::Done(Compared::Bytes(bytes)) => match bytes.first {
                None => format!("The files are identical: {} bytes.", grouped(bytes.sizes.0)),
                Some(first) => format!(
                    "{place}: {} byte(s) differ, the first at {}; sizes {} and {}.",
                    grouped(bytes.differing),
                    grouped(first),
                    grouped(bytes.sizes.0),
                    grouped(bytes.sizes.1)
                ),
            },
        }
    }
}

#[derive(Default)]
struct Side {
    number: String,
    text: String,
    parts: Vec<Range<usize>>,
    bg: Option<Hsla>,
}

impl Side {
    fn new((number, text): (String, String), parts: Vec<Range<usize>>, bg: Option<Hsla>) -> Self {
        Side {
            number,
            text,
            parts,
            bg,
        }
    }
}

impl Render for CompareView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let none = self.runs.is_empty();
        let path = |side: usize, cx: &mut Context<Self>| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().flex_1().child(Input::new(&self.inputs[side]).small()))
                .child(
                    Button::new(("compare-browse", side))
                        .small()
                        .outline()
                        .label(">>")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.browse(side, window, cx)
                        })),
                )
        };
        let check = |id: &'static str, label: &'static str, on: bool| {
            Checkbox::new(id).label(label).checked(on)
        };
        let list = uniform_list(
            "compare-rows",
            self.visible.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1();
        let below = self.current.map(|row| self.sides(row, cx));
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let keys = &event.keystroke;
                if keys.key == "escape" {
                    return window.remove_window();
                }
                if !this.focus.is_focused(window) {
                    return;
                }
                match keys.key.as_str() {
                    "n" | "f8" if !keys.modifiers.shift => this.go(true, cx),
                    "p" | "f8" => this.go(false, cx),
                    "up" => this.step(-1, cx),
                    "down" => this.step(1, cx),
                    "pageup" => this.step(-30, cx),
                    "pagedown" => this.step(30, cx),
                    "home" => this.step(isize::MIN, cx),
                    "end" => this.step(isize::MAX, cx),
                    "q" => window.remove_window(),
                    _ => {}
                }
            }))
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .child(path(0, cx))
                    .child(path(1, cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_2()
                    .pb_2()
                    .flex_wrap()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("compare-again")
                            .small()
                            .primary()
                            .label("Compare")
                            .on_click(cx.listener(|this, _, _, cx| this.compare(cx))),
                    )
                    .child(
                        Button::new("compare-next")
                            .small()
                            .outline()
                            .label("Next difference (N)")
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.go(true, cx))),
                    )
                    .child(
                        Button::new("compare-previous")
                            .small()
                            .outline()
                            .label("Previous difference (P)")
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.go(false, cx))),
                    )
                    .child(
                        check("compare-case", "Ignore case", self.options.ignore_case).on_click(
                            cx.listener(|this, on: &bool, _, cx| {
                                this.set_options(|o| o.ignore_case = *on, cx)
                            }),
                        ),
                    )
                    .child(
                        check(
                            "compare-spaces",
                            "Ignore repeated spaces",
                            self.options.ignore_spaces,
                        )
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.set_options(|o| o.ignore_spaces = *on, cx)
                        })),
                    )
                    .child(
                        check("compare-binary", "Binary", self.options.binary).on_click(
                            cx.listener(|this, on: &bool, _, cx| {
                                this.set_options(|o| o.binary = *on, cx)
                            }),
                        ),
                    )
                    .child(
                        check(
                            "compare-only",
                            "Only differences",
                            self.only_differences,
                        )
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.only_differences = *on;
                            this.show_rows();
                            this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                            cx.notify();
                        })),
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
            .when_some(below, |el, [left, right]| {
                el.child(
                    v_flex()
                        .flex_none()
                        .border_t_1()
                        .border_color(theme.border)
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(div().h(px(ROW_HEIGHT)).flex().child(self.render_side(left, false, cx)))
                        .child(div().h(px(ROW_HEIGHT)).flex().child(self.render_side(right, false, cx))),
                )
            })
            .child(
                div()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .child(self.status()),
            )
    }
}
