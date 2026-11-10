//! Associate With and Internal Associations: by file mask, the programs
//! Enter, F3 and F4 run. Kept in the settings.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::associations::{self, Association};

use crate::Commander;

type Saved = Rc<dyn Fn(Association, &mut Window, &mut App)>;

impl Commander {
    /// The cursor's file type: its association, new or to change.
    pub(crate) fn associate_with(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(entry) = panel.current_entry().filter(|e| !e.is_dir) else {
            let text = "Put the cursor on a file of the type to associate.";
            return window.push_notification(Notification::warning(text), cx);
        };
        let mask = associations::mask_for(&entry.display_name());
        let list = &self.settings.associations;
        let at = list.iter().position(|a| a.mask.eq_ignore_ascii_case(&mask));
        let association = match at {
            Some(at) => list[at].clone(),
            None => Association {
                mask,
                ..Association::default()
            },
        };
        let commander = cx.entity().downgrade();
        edit(association, window, cx, move |association, window, cx| {
            let _ = commander.update(cx, |this, cx| {
                let mut list = this.settings.associations.clone();
                match at {
                    Some(at) => list[at] = association,
                    None => list.push(association),
                }
                this.save_associations(list, window, cx);
            });
        });
    }

    pub(crate) fn internal_associations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        let view = cx.new(|_| AssociationsDialog { commander });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Internal associations")
                .width(px(820.))
                .child(view.clone())
        });
    }

    fn save_associations(
        &mut self,
        list: Vec<Association>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.associations = list;
        if let Err(err) = self.settings.save() {
            let text = format!("Could not save the settings: {err}");
            window.push_notification(Notification::error(text), cx);
        }
        cx.notify();
    }
}

struct AssociationsDialog {
    commander: WeakEntity<Commander>,
}

impl AssociationsDialog {
    fn change(
        &self,
        cx: &mut App,
        change: impl FnOnce(&mut Vec<Association>),
        window: &mut Window,
    ) {
        let _ = self.commander.update(cx, |this, cx| {
            let mut list = this.settings.associations.clone();
            change(&mut list);
            this.save_associations(list, window, cx);
        });
    }

    fn open_edit(&self, at: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let list = self
            .commander
            .read_with(cx, |this, _| this.settings.associations.clone())
            .unwrap_or_default();
        let association = at.and_then(|at| list.get(at).cloned()).unwrap_or_default();
        let this = cx.entity().downgrade();
        edit(association, window, cx, move |association, window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.change(
                    cx,
                    |list| match at {
                        Some(at) if at < list.len() => list[at] = association,
                        _ => list.push(association),
                    },
                    window,
                );
                cx.notify();
            });
        });
    }
}

impl Render for AssociationsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .commander
            .read_with(cx, |this, _| this.settings.associations.clone())
            .unwrap_or_default();
        let muted = cx.theme().muted_foreground;
        let cell = |text: &str, width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .truncate()
                .child(if text.is_empty() {
                    "–".to_string()
                } else {
                    text.to_string()
                })
        };
        let rows = list.iter().enumerate().map(|(ix, a)| {
            h_flex()
                .gap_2()
                .child(cell(&a.mask, 150.))
                .child(cell(&a.open, 160.))
                .child(cell(&a.view, 160.))
                .child(cell(&a.edit, 160.))
                .child(
                    Button::new(("assoc-edit", ix))
                        .xsmall()
                        .outline()
                        .label("Edit...")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_edit(Some(ix), window, cx)
                        })),
                )
                .child(
                    Button::new(("assoc-remove", ix))
                        .xsmall()
                        .outline()
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.change(cx, |list| drop(list.remove(ix)), window);
                            cx.notify();
                        })),
                )
        });
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child("Enter, F3 and F4 run these programs for the files of a mask; empty keeps the usual. %1 is the file.")
            .child(
                h_flex()
                    .gap_2()
                    .text_color(muted)
                    .child(cell("Files", 150.))
                    .child(cell("Open (Enter)", 160.))
                    .child(cell("View (F3)", 160.))
                    .child(cell("Edit (F4)", 160.)),
            )
            .children(rows)
            .when(list.is_empty(), |el| el.child(div().text_color(muted).child("None yet.")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("assoc-add")
                            .outline()
                            .label("Add...")
                            .on_click(cx.listener(|this, _, window, cx| this.open_edit(None, window, cx))),
                    )
                    .child(
                        Button::new("assoc-close")
                            .primary()
                            .label("Close")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

/// One association's mask and programs; `done` gets it on OK.
fn edit(
    association: Association,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Association, &mut Window, &mut App) + 'static,
) {
    let fields = [
        &association.mask,
        &association.open,
        &association.view,
        &association.edit,
    ]
    .map(|value| {
        let value = value.clone();
        cx.new(|cx| InputState::new(window, cx).default_value(value))
    });
    let mask = fields[0].clone();
    let view = cx.new(|_| EditAssociation {
        fields,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        dialog
            .title("Associate")
            .width(px(560.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| ok.update(cx, |this, cx| this.finish(window, cx)))
    });
    crate::pack_dialog::focus_all(&mask, window, cx);
}

struct EditAssociation {
    /// Mask, open, view, edit.
    fields: [Entity<InputState>; 4],
    done: Saved,
}

impl EditAssociation {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let [mask, open, view, edit] = self
            .fields
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_string());
        if mask.is_empty() {
            window.push_notification(Notification::error("The files' mask is needed: *.txt"), cx);
            return false;
        }
        (self.done)(
            Association {
                mask,
                open,
                view,
                edit,
            },
            window,
            cx,
        );
        true
    }
}

impl Render for EditAssociation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = [
            "Files (*.jpg *.png):",
            "Open with (Enter):",
            "View with (F3):",
            "Edit with (F4):",
        ];
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .children(labels.iter().zip(&self.fields).map(|(label, input)| {
                h_flex()
                    .gap_2()
                    .child(div().w(px(150.)).flex_none().child(*label))
                    .child(div().flex_1().child(Input::new(input).small()))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("assoc-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.finish(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("assoc-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
