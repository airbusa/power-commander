//! Quick View (Total Commander's Ctrl+Q): the panel opposite the active one
//! shows what the active one's cursor is on, as the viewer does; a folder,
//! or `..`, shows the space it occupies. A server's or an archive's file is
//! copied here first, once the cursor rests on it. Tab takes the keys to the viewer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::origin::Origin;
use pc_core::space::{self, Space};
use pc_core::vfs::Local;
use pc_core::{grouped, human_size};

use crate::{Commander, PATH_HEIGHT, viewer};

pub(crate) struct QuickView {
    /// The panel it covers; the other one's cursor chooses what it shows.
    pub(crate) side: usize,
    /// The file or folder shown, and whether it is a folder.
    shown: Option<(PathBuf, bool)>,
    viewer: Option<viewer::Quick>,
    folder: Option<Folder>,
    /// Shown instead of the viewer: a file being copied here, or why not.
    note: Option<String>,
    /// Copying the server's or archive's file; dropped, it stops and is not shown.
    taking: Option<Taking>,
    /// The folder the shown file was copied to, removed once it goes.
    copy: Option<PathBuf>,
}

/// Larger files of servers and archives are not copied to look at in passing: F3 does.
const TAKEN_OUT_MAX: u64 = 256 << 20;

impl Drop for QuickView {
    fn drop(&mut self) {
        if let Some(copy) = self.copy.take() {
            let _ = std::fs::remove_dir_all(copy);
        }
    }
}

struct Taking {
    stop: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for Taking {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A folder being counted: none until done.
struct Folder {
    space: Option<Space>,
    stop: Arc<AtomicBool>,
    _count: Task<()>,
}

impl Drop for Folder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl QuickView {
    /// The viewer, while a file is shown.
    fn file_viewer(&self) -> Option<&viewer::Quick> {
        self.viewer.as_ref().filter(|_| matches!(self.shown, Some((_, false))) && self.note.is_none())
    }
}

impl Commander {
    pub(crate) fn toggle_quick_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick = match self.quick.take() {
            Some(_) => {
                window.focus(&self.focus, cx);
                None
            }
            None => Some(QuickView {
                side: 1 - self.active,
                shown: None,
                viewer: None,
                folder: None,
                note: None,
                taking: None,
                copy: None,
            }),
        };
        self.reload_menus(cx);
        cx.notify();
    }

    /// Whether the keys go to Quick View's viewer.
    pub(crate) fn quick_focused(&self, window: &Window, cx: &App) -> bool {
        let viewer = self.quick.as_ref().and_then(QuickView::file_viewer);
        viewer.is_some_and(|viewer| viewer.focus_handle(cx).contains_focused(window, cx))
    }

    /// Tab: between the panel and the viewer. False: no Quick View, the panels switch.
    pub(crate) fn tab_quick_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(quick) = &self.quick else {
            return false;
        };
        if let Some(viewer) = quick.file_viewer() {
            match self.quick_focused(window, cx) {
                true => window.focus(&self.focus, cx),
                false => window.focus(&viewer.focus_handle(cx), cx),
            }
            cx.notify();
        }
        true
    }

    /// Shows what the cursor is on, once it moved; opposite the active panel.
    pub(crate) fn sync_quick_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active;
        let Some(quick) = &mut self.quick else {
            return;
        };
        quick.side = 1 - active;
        let panel = self.panels[active].tabs.current();
        let wanted = panel.quick_view();
        if wanted == quick.shown {
            return;
        }
        let copied = panel.origin().filter(|origin| !origin.is_local()).zip(panel.current_entry().map(|entry| entry.size));
        let elsewhere = panel.in_archive() || panel.remote().is_some();
        quick.shown = wanted.clone();
        (quick.folder, quick.note, quick.taking) = (None, None, None);
        if let Some(copy) = quick.copy.take() {
            let _ = std::fs::remove_dir_all(copy);
        }
        match wanted {
            Some((_, false)) if let Some((origin, size)) = copied.as_ref().filter(|(_, size)| *size > TAKEN_OUT_MAX) => {
                let copy = match origin {
                    Origin::Server(..) => "download",
                    _ => "take out of the archive",
                };
                quick.note = Some(format!("{} is too large to {copy} here: F3 views it.", pc_core::human_size(*size)));
            }
            Some((_, false)) if let Some((origin, _)) = copied => {
                quick.note = Some(match origin {
                    Origin::Server(..) => "Downloading…".into(),
                    _ => "Taking it out of the archive…".into(),
                });
                quick.taking = Some(take_out(origin, window, cx));
            }
            Some((_, true)) if elsewhere => quick.note = Some("On servers and inside archives, Quick View shows files.".into()),
            Some((path, false)) => match &quick.viewer {
                Some(viewer) => viewer.show(path, window, cx),
                None => quick.viewer = Some(viewer::Quick::new(path, self.settings.viewer.clone(), window, cx)),
            },
            // Video or sound must not go on unseen.
            Some((path, true)) => {
                quick.viewer = None;
                quick.folder = Some(count(path, cx));
            }
            None => quick.viewer = None,
        }
    }

    /// Under the side's `tab_bar`, inside the same border as a panel's, so the tabs stay put.
    pub(crate) fn render_quick_view(&self, quick_focused: bool, tab_bar: Option<AnyElement>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let quick = self.quick.as_ref()?;
        let theme = cx.theme();
        let name = |path: &Path| path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        let (title, body) = match (&quick.shown, quick.file_viewer(), &quick.folder, &quick.note) {
            (Some((path, _)), _, _, Some(text)) => (name(path), note(text.clone(), cx)),
            (Some((path, false)), Some(viewer), _, _) => (name(path), viewer.view().into_any_element()),
            (Some((path, true)), _, Some(folder), _) => (path.display().to_string(), folder_text(path, folder.space, cx)),
            _ => (String::new(), note("Quick View shows what the other panel's cursor is on.".into(), cx)),
        };
        let (title_bg, title_fg, stripe) = match quick_focused {
            true => (self.colors.active_path, theme.foreground, theme.primary),
            false => (self.colors.inactive_path, theme.muted_foreground, self.colors.inactive_path),
        };
        let header = div()
            .h(PATH_HEIGHT)
            .flex()
            .items_center()
            .px_2()
            .border_l(px(3.))
            .border_color(stripe)
            .bg(title_bg)
            .text_color(title_fg)
            .child(div().flex_1().min_w_0().truncate().child(title));
        let element = v_flex()
            .size_full()
            .min_w_0()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if let Some(viewer) = this.quick.as_ref().and_then(QuickView::file_viewer) {
                        window.focus(&viewer.focus_handle(cx), cx);
                        cx.notify();
                    }
                }),
            )
            .children(tab_bar)
            .child(header)
            .child(div().flex_1().min_h_0().relative().child(body));
        Some(element.into_any_element())
    }
}

/// Counts in the background; dropping it stops the count.
fn count(path: PathBuf, cx: &mut Context<Commander>) -> Folder {
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let counting = cx.background_executor().spawn(async move {
        let mut space = Space::default();
        space::measure(&Local, &path, &mut space, &stopping, &mut |_| {}).map(|()| space)
    });
    let count = cx.spawn(async move |this, cx| {
        let counted = counting.await;
        let _ = this.update(cx, |this, cx| {
            let folder = this.quick.as_mut().and_then(|quick| quick.folder.as_mut());
            if let (Ok(space), Some(folder)) = (counted, folder) {
                folder.space = Some(space);
                cx.notify();
            }
        });
    });
    Folder { space: None, stop, _count: count }
}

/// As Total Commander's: the folder, then what is in it.
fn folder_text(path: &std::path::Path, space: Option<Space>, cx: &App) -> AnyElement {
    let lines = match space {
        None => vec!["Counting…".to_owned()],
        Some(space) => {
            // The folder itself counts as one.
            let folders = space.dirs.saturating_sub(1);
            let mut lines = vec![
                "Total space occupied:".to_owned(),
                format!("{} bytes in {} file(s), {} folder(s)", grouped(space.bytes), grouped(space.files), grouped(folders)),
                format!("(= {})", human_size(space.bytes)),
            ];
            if space.unreadable > 0 {
                lines.push(String::new());
                lines.push(format!("{} could not be read.", grouped(space.unreadable)));
            }
            lines
        }
    };
    let text = v_flex().p_3().gap_1().text_color(cx.theme().foreground);
    let text = text.child(div().pb_2().child(path.display().to_string()));
    text.children(lines.into_iter().map(|line| div().when(line.is_empty(), |el| el.h_2()).child(line))).into_any_element()
}

/// Copies the server's or archive's file once the cursor has rested on it, then shows it.
fn take_out(origin: Origin, window: &mut Window, cx: &mut Context<Commander>) -> Taking {
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let task = cx.spawn_in(window, async move |this, cx| {
        // Passing over files copies none.
        cx.background_executor().timer(Duration::from_millis(200)).await;
        let file = cx.background_executor().spawn(async move {
            let file = origin.view_until(&stopping)?;
            // Moved on while it came: nothing will show it or remove it.
            if stopping.load(Ordering::Relaxed) && !origin.is_local() {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::remove_dir_all(dir);
                }
                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "stopped"));
            }
            Ok(file)
        });
        let file = file.await;
        let _ = this.update_in(cx, |this, window, cx| {
            let settings = this.settings.viewer.clone();
            let Some(quick) = &mut this.quick else {
                return;
            };
            match file {
                Ok(file) => {
                    quick.copy = file.parent().map(Path::to_owned);
                    quick.note = None;
                    match &quick.viewer {
                        Some(viewer) => viewer.show(file, window, cx),
                        None => quick.viewer = Some(viewer::Quick::new(file, settings, window, cx)),
                    }
                }
                Err(err) => quick.note = Some(err.to_string()),
            }
            cx.notify();
        });
    });
    Taking { stop, _task: task }
}

fn note(text: String, cx: &App) -> AnyElement {
    div().p_3().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}
