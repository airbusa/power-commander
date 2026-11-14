//! An archive taking a while to open: 7-Zip's progress, and Cancel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::progress::Progress as Bar;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use sc_core::archive::Watch;

/// Closes itself once `done`; Cancel, Escape or the close button stop the listing.
pub fn show(name: String, watch: Watch, done: Arc<AtomicBool>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let poll = cx.spawn_in(window, async move |this: WeakEntity<Opening>, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let finished = this.update_in(cx, |this, window, cx| {
                    let finished = this.done.load(Ordering::Relaxed);
                    match finished {
                        true => window.close_dialog(cx),
                        false => cx.notify(),
                    }
                    finished
                });
                if finished.unwrap_or(true) {
                    break;
                }
            }
        });
        Opening { name, watch, done, _poll: poll }
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let stop = view.read(cx).watch.stop.clone();
        dialog
            .title("Opening archive")
            .width(px(480.))
            .overlay_closable(false)
            .child(view.clone())
            .on_cancel(move |_, _, _| {
                stop.store(true, Ordering::Relaxed);
                true
            })
    });
}

struct Opening {
    name: String,
    watch: Watch,
    done: Arc<AtomicBool>,
    _poll: Task<()>,
}

impl Render for Opening {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let percent = self.watch.percent.load(Ordering::Relaxed);
        // A plain archive's listing, or a server's download, tells no percentage.
        let state = match percent {
            0 => "Reading…".to_owned(),
            percent => format!("{percent}%"),
        };
        let stop = self.watch.stop.clone();
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(div().truncate().child(self.name.clone()))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Bar::new("opening-progress").value(percent as f32)))
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .text_color(cx.theme().muted_foreground)
                            .child(state),
                    ),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("opening-cancel")
                        .outline()
                        .label("Cancel")
                        .min_w(px(80.))
                        .on_click(move |_, window, cx| {
                            stop.store(true, Ordering::Relaxed);
                            window.close_dialog(cx);
                        }),
                ),
            )
    }
}
