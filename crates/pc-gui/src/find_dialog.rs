use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, IconName, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use pc_core::viewer::Query;

type Done = Rc<dyn Fn(Query, &mut Window, &mut App)>;

/// Lister's Find dialog; `done` gets the query on OK or Enter.
pub fn open(
    query: Query,
    history: Vec<String>,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Query, &mut Window, &mut App) + 'static,
) {
    let text = query.text.clone();
    let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
    let view = cx.new(|_| FindDialog {
        input: input.clone(),
        query,
        history,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        dialog
            .title("Find")
            .width(px(460.))
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

struct FindDialog {
    input: Entity<InputState>,
    /// Its options; the text is the input's.
    query: Query,
    history: Vec<String>,
    done: Done,
}

impl FindDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = Query {
            text: self.input.read(cx).value().to_string(),
            ..self.query.clone()
        };
        (self.done)(query, window, cx);
    }

    fn option(
        &self,
        id: &'static str,
        label: &'static str,
        on: fn(&mut Query) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Checkbox {
        let mut query = self.query.clone();
        Checkbox::new(id)
            .label(label)
            .checked(*on(&mut query))
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                *on(&mut this.query) = *checked;
                cx.notify();
            }))
    }
}

impl Render for FindDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.input.clone();
        let history = self.history.clone();
        let earlier = Button::new("find-history")
            .ghost()
            .icon(IconName::ChevronDown)
            .disabled(history.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for text in &history {
                    let (input, text) = (input.clone(), text.clone());
                    let item = PopupMenuItem::new(text.clone()).on_click(move |_, window, cx| {
                        input.update(cx, |input, cx| input.set_value(text.clone(), window, cx))
                    });
                    menu = menu.item(item);
                }
                menu
            });
        let left = v_flex()
            .flex_1()
            .gap_2()
            .child(self.option("find-words", "Whole words only", |q| &mut q.whole_words, cx))
            .child(self.option("find-back", "Search backwards", |q| &mut q.backwards, cx))
            .child(self.option("find-regex", "RegEx", |q| &mut q.regex, cx));
        let right = v_flex()
            .flex_1()
            .gap_2()
            .child(self.option("find-case", "Case sensitive", |q| &mut q.case_sensitive, cx))
            .child(self.option("find-hex", "Search hex string", |q| &mut q.hex, cx));
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex().gap_1().child("Enter search text").child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.input)))
                        .child(earlier),
                ),
            )
            .child(h_flex().items_start().child(left).child(right))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("find-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("find-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
