use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::*;

type Done = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Asks for a name; `done` gets it on OK or Enter.
pub fn open(
    title: &'static str,
    label: &'static str,
    value: String,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
    let view = cx.new(|_| NameDialog {
        label,
        input: input.clone(),
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        dialog
            .title(title)
            .width(px(400.))
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

struct NameDialog {
    label: &'static str,
    input: Entity<InputState>,
    done: Done,
}

impl NameDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().to_string();
        (self.done)(name, window, cx);
    }
}

impl Render for NameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label)
                    .child(Input::new(&self.input)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("name-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("name-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
