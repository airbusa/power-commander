//! Go > Manage Bookmarks: Places' bookmarks added, removed, renamed and put
//! in order, by the buttons or by dragging; this computer's folders only.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::icons;

use crate::Commander;
use crate::drag_drop::Dragged;

impl Commander {
    pub(crate) fn manage_bookmarks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity();
        let view = cx.new(|cx| BookmarksDialog {
            // Renamed, dropped on the sidebar: shown here too.
            _changed: cx.observe(&commander, |_, _, cx| cx.notify()),
            commander: commander.downgrade(),
            selected: None,
        });
        window.open_dialog(cx, move |dialog, _, _| dialog.title("Bookmarks").width(px(640.)).child(view.clone()));
    }
}

struct BookmarksDialog {
    commander: WeakEntity<Commander>,
    selected: Option<usize>,
    _changed: Subscription,
}

/// A row on its way to another place.
#[derive(Clone)]
struct Moved {
    ix: usize,
    name: SharedString,
}

impl Render for Moved {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().px_2().py_1().rounded(theme.radius).border_1().border_color(theme.border).bg(theme.popover).text_sm().child(self.name.clone())
    }
}

impl BookmarksDialog {
    fn run(&self, window: &mut Window, cx: &mut Context<Self>, run: impl FnOnce(&mut Commander, &mut Window, &mut Context<Commander>)) {
        let _ = self.commander.update(cx, |this, cx| run(this, window, cx));
    }

    fn count(&self, cx: &App) -> usize {
        self.commander.read_with(cx, |this, _| this.settings.bookmarks.len()).unwrap_or(0)
    }

    fn move_to(&mut self, ix: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        if to < self.count(cx) {
            self.run(window, cx, |this, window, cx| this.move_bookmark(ix, to, window, cx));
            self.selected = Some(to);
        }
    }

    fn remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, |this, window, cx| this.remove_bookmark(ix, window, cx));
        let count = self.count(cx);
        self.selected = (count > 0).then(|| ix.min(count - 1));
    }

    fn add(&mut self, paths: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, |this, window, cx| this.add_bookmarks(paths, window, cx));
    }

    /// The system's folder chooser.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Bookmark".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = paths.await;
            let _ = this.update_in(cx, |this, window, cx| match chosen {
                Ok(Ok(Some(paths))) => this.add(paths, window, cx),
                Ok(Err(err)) => window.push_notification(Notification::error(format!("Could not show the folder chooser: {err}")), cx),
                _ => {}
            });
        })
        .detach();
    }
}

impl Render for BookmarksDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(commander) = self.commander.upgrade() else {
            return div();
        };
        let settings = &commander.read(cx).settings;
        let home = std::env::home_dir();
        let rows: Vec<_> = settings.bookmarks.iter().map(|path| (settings.bookmark_name(path), path.clone())).collect();
        let selected = self.selected.filter(|&ix| ix < rows.len());
        let theme = cx.theme();
        let (target, muted, dark) = (theme.accent.opacity(0.35), theme.muted_foreground, theme.mode.is_dark());
        let mut list = v_flex()
            .id("bookmark-list")
            .h(px(320.))
            .overflow_y_scroll()
            .p_1()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .drag_over::<Dragged>(move |style, _, _, _| style.bg(target))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
            .on_drop(cx.listener(|this, dragged: &Dragged, window, cx| {
                let folders = this.commander.read_with(cx, |this, _| this.dragged_folders(*dragged)).unwrap_or_default();
                this.add(folders, window, cx);
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.add(paths.paths().to_vec(), window, cx)));
        if rows.is_empty() {
            list = list.child(div().p_2().text_sm().text_color(muted).child("No bookmarks: add folders, or drop them here."));
        }
        for (ix, (name, path)) in rows.iter().enumerate() {
            let theme = cx.theme();
            let folder = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let icon = icons::material().icon(&folder, true, !dark);
            let moved = Moved { ix, name: name.clone().into() };
            list = list.child(
                h_flex()
                    .id(("bookmark-row", ix))
                    .flex_none()
                    .h(px(28.))
                    .px_1p5()
                    .gap_2()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .when(selected == Some(ix), |el| el.bg(theme.list_active))
                    .when(selected != Some(ix), |el| el.hover(|style| style.bg(theme.list_hover)))
                    .child(img(format!("material/{icon}")).size(px(16.)).flex_none())
                    .child(div().flex_none().max_w(relative(0.4)).truncate().child(name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(if selected == Some(ix) { theme.foreground.opacity(0.7) } else { muted }).child(crate::sidebar::tilde(path, home.as_deref())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(ix);
                        cx.notify();
                    }))
                    .on_drag(moved, |moved, _, _, cx| cx.new(|_| moved.clone()))
                    .drag_over::<Moved>(move |style, _, _, _| style.bg(target))
                    .on_drop(cx.listener(move |this, moved: &Moved, window, cx| this.move_to(moved.ix, ix, window, cx))),
            );
        }
        let count = rows.len();
        // On the selected row.
        let on_selected = |button: Button, run: fn(&mut Self, usize, &mut Window, &mut Context<Self>)| {
            button.outline().disabled(selected.is_none()).on_click(cx.listener(move |this, _, window, cx| {
                if let Some(ix) = this.selected {
                    run(this, ix, window, cx);
                }
            }))
        };
        let buttons = h_flex()
            .gap_2()
            .child(Button::new("bookmarks-add").outline().label("Add Folders...").on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))))
            .child(on_selected(Button::new("bookmarks-rename").label("Rename..."), |this, ix, window, cx| {
                this.run(window, cx, |this, window, cx| this.rename_bookmark(ix, window, cx))
            }))
            .child(on_selected(Button::new("bookmarks-remove").label("Remove"), Self::remove))
            .child(
                on_selected(Button::new("bookmarks-up").icon(IconName::ArrowUp).tooltip("Move Up"), |this, ix, window, cx| {
                    this.move_to(ix, ix.wrapping_sub(1), window, cx)
                })
                .disabled(selected.is_none_or(|ix| ix == 0)),
            )
            .child(
                on_selected(Button::new("bookmarks-down").icon(IconName::ArrowDown).tooltip("Move Down"), |this, ix, window, cx| {
                    this.move_to(ix, ix + 1, window, cx)
                })
                .disabled(selected.is_none_or(|ix| ix + 1 >= count)),
            )
            .child(
                Button::new("bookmarks-sort")
                    .outline()
                    .label("Sort A–Z")
                    .disabled(count < 2)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run(window, cx, Commander::sort_bookmarks);
                        this.selected = None;
                    })),
            )
            .child(div().flex_1())
            .child(Button::new("bookmarks-close").primary().label("Close").min_w(px(80.)).on_click(|_, window, cx| window.close_dialog(cx)));
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(div().text_color(muted).child("This computer's folders, in Places' order: drag one to move it, or drop folders here to add them."))
            .child(list)
            .child(buttons)
    }
}
