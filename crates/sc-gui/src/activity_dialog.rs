//! Commands > Activity Log: what was done to files, newest first, and how
//! each ended (`sc_core::activity`).

use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use sc_core::activity::{self, Outcome};

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|_| ActivityLog { entries: activity::recent() });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Activity Log").width(px(760.)).overlay_closable(false).child(view.clone())
    });
}

struct ActivityLog {
    entries: Vec<activity::Entry>,
}

impl Render for ActivityLog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (dim, bad) = (theme.muted_foreground, theme.danger);
        let rows = self.entries.iter().enumerate().map(|(ix, entry)| {
            let (mark, color) = match entry.outcome {
                Outcome::Done => ("✓", theme.success),
                Outcome::Failed => ("✗", bad),
                Outcome::Cancelled => ("–", dim),
            };
            h_flex()
                .id(("activity", ix))
                .gap_3()
                .items_start()
                .py_0p5()
                .child(div().flex_none().w(px(140.)).text_color(dim).child(sc_core::dates::date_time(entry.time())))
                .child(div().flex_none().w(px(14.)).text_color(color).child(mark))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().whitespace_normal().child(entry.title.clone()))
                        .children(entry.note.clone().map(|note| div().whitespace_normal().text_xs().text_color(dim).child(note))),
                )
        });
        let empty = self.entries.is_empty().then(|| div().text_color(dim).child("Nothing done yet."));
        v_flex()
            .gap_3()
            .text_sm()
            .child(v_flex().h(px(420.)).child(div().id("activity-list").flex_1().min_h_0().overflow_y_scrollbar().pr_3().children(empty).children(rows)))
            .child(
                h_flex().justify_end().child(
                    Button::new("activity-clear").outline().label("Clear").on_click(cx.listener(|this, _, window, cx| {
                        if let Err(err) = activity::clear() {
                            window.push_notification(format!("Could not clear the log: {err}"), cx);
                        }
                        this.entries.clear();
                        cx.notify();
                    })),
                ),
            )
    }
}
