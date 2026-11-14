//! The terminal under the panels: its tabs, each a shell; shown or hidden;
//! the command line's commands run there, in the active panel's folder.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, v_flex};
use gpui_kit::*;
use pc_core::menu::Command;

use crate::Commander;
use crate::menus::Run;
use crate::terminal::{self, Exited, TerminalView};

/// One of the terminal's shells.
pub(crate) struct TerminalTab {
    pub(crate) view: Entity<TerminalView>,
    /// The shell's folder, as sent to it or seen; none for an ssh session.
    pub(crate) dir: Option<PathBuf>,
    _ended: Subscription,
}

/// The terminal's top edge, dragged.
pub(crate) struct TerminalDrag;

impl Commander {
    fn terminal_tab(&self) -> Option<&TerminalTab> {
        self.terminals.get(self.terminal_tab)
    }

    /// After a menu that ran the command has given the focus back to where it was.
    fn focus_later(&self, handle: FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, move |_, window, cx| window.focus(&handle, cx));
    }

    /// Shown or hidden: the backdrop, and the menus' check on Show > Terminal.
    fn terminal_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_backdrop(window, cx);
        self.reload_menus(cx);
        cx.notify();
    }

    /// Shows the terminal, a shell started if none runs; shown and in use, hides it, the shells kept.
    pub(crate) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let using = self.terminal_tab().map(|tab| tab.view.read(cx).focus.clone());
        match using {
            Some(focus) if self.terminal_shown && focus.is_focused(window) => {
                self.terminal_shown = false;
                self.focus_later(self.focus.clone(), window, cx);
            }
            Some(focus) => {
                self.terminal_shown = true;
                self.focus_later(focus, window, cx);
            }
            None => return self.new_terminal_tab(window, cx),
        }
        self.terminal_changed(window, cx);
    }

    /// Another shell, in the active panel's folder, or by ssh in an SFTP panel's.
    pub(crate) fn new_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let ssh = panel.remote().and_then(|remote| pc_core::remote::ssh_command(&remote.url(), panel.folder()));
        // An archive's folder is not this computer's.
        let dir = (panel.remote().is_none() && !panel.in_archive()).then(|| panel.folder().to_owned());
        self.open_terminal_tab(dir, ssh, window, cx);
    }

    /// A saved SFTP server's shell, by ssh, in its folder if it has one.
    pub(crate) fn ssh_in_terminal(&mut self, address: &pc_core::remote::Address, window: &mut Window, cx: &mut Context<Self>) {
        let dir = address.path.as_deref().unwrap_or(".");
        let ssh = pc_core::remote::ssh_command(&address.url(), Path::new(dir));
        self.open_terminal_tab(None, ssh, window, cx);
    }

    /// A shell in `dir` (else home), or `program` there.
    fn open_terminal_tab(&mut self, dir: Option<PathBuf>, program: Option<(String, Vec<String>)>, window: &mut Window, cx: &mut Context<Self>) {
        let home = std::env::home_dir().unwrap_or_default();
        match TerminalView::open(dir.as_deref().unwrap_or(&home), program, window, cx) {
            Ok(view) => {
                let ended = cx.subscribe_in(&view, window, |this, view, _: &Exited, window, cx| {
                    if let Some(ix) = this.terminals.iter().position(|tab| &tab.view == view) {
                        this.close_terminal_tab(ix, window, cx);
                    }
                });
                self.focus_later(view.read(cx).focus.clone(), window, cx);
                self.terminals.push(TerminalTab { view, dir, _ended: ended });
                self.terminal_tab = self.terminals.len() - 1;
                self.terminal_shown = true;
            }
            Err(err) => window.push_notification(Notification::error(format!("Could not start the shell: {err}")), cx),
        }
        self.terminal_changed(window, cx);
    }

    /// Its shell hung up; the last one gone, the terminal hides.
    fn close_terminal_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.terminals.len() {
            return;
        }
        self.terminals.remove(ix);
        if self.terminal_tab > ix || self.terminal_tab == self.terminals.len() {
            self.terminal_tab = self.terminal_tab.saturating_sub(1);
        }
        match self.terminal_tab() {
            Some(tab) => self.focus_later(tab.view.read(cx).focus.clone(), window, cx),
            None => {
                self.terminal_shown = false;
                self.focus_later(self.focus.clone(), window, cx);
            }
        }
        self.terminal_changed(window, cx);
    }

    fn select_terminal_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.terminals.get(ix) {
            self.terminal_tab = ix;
            self.focus_later(tab.view.read(cx).focus.clone(), window, cx);
            cx.notify();
        }
    }

    /// `cd` to the active panel's folder, unless the shell is there, on a
    /// server or busy. Linux and macOS: Windows' PowerShell cannot be told idle.
    fn send_panel_folder(&mut self, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let folder = panel.folder().to_owned();
        let local = panel.remote().is_none() && !panel.in_archive();
        let Some(tab) = self.terminals.get_mut(self.terminal_tab) else {
            return;
        };
        if !local || tab.dir.is_none() || tab.dir.as_ref() == Some(&folder) || tab.view.read(cx).idle() != Some(true) {
            return;
        }
        // Ctrl+U clears the line; a leading space keeps it out of the history.
        let line = format!("\x15 cd -- '{}'\r", folder.to_string_lossy().replace('\'', r"'\''"));
        tab.view.update(cx, |view, _| view.send(&line));
        tab.dir = Some(folder);
    }

    /// The command line's `line`, run by the shown terminal in the panel's
    /// `folder`: none without one, false while a program runs there.
    pub(crate) fn run_in_terminal(&mut self, line: &str, folder: &Path, window: &mut Window, cx: &mut Context<Self>) -> Option<bool> {
        let tab = self.terminals.get(self.terminal_tab).filter(|tab| self.terminal_shown && tab.dir.is_some())?;
        if tab.view.read(cx).idle() == Some(false) {
            let text = "A program is running in the terminal: run the command once it ends.";
            window.push_notification(Notification::warning(text), cx);
            return Some(false);
        }
        let view = tab.view.clone();
        self.send_panel_folder(cx);
        // PowerShell's prompt cannot be told apart from a program: the folder goes with the command.
        #[cfg(windows)]
        let line = match self.terminals[self.terminal_tab].dir.as_deref() == Some(folder) {
            true => line.to_owned(),
            false => {
                self.terminals[self.terminal_tab].dir = Some(folder.to_owned());
                format!("Set-Location -LiteralPath '{}'; {line}", folder.display().to_string().replace('\'', "''"))
            }
        };
        let _ = folder;
        // Ctrl+U clears what was typed at the prompt.
        let clear = if cfg!(windows) { "" } else { "\x15" };
        view.update(cx, |view, _| view.send(&format!("{clear}{line}\r")));
        Some(true)
    }

    /// The tabs and a new one's +.
    fn render_terminal_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tabs = self.terminals.iter().enumerate().map(|(ix, tab)| {
            let view = tab.view.read(cx);
            let name = match (&view.title, &tab.dir) {
                (Some(title), _) => title.clone(),
                (None, Some(dir)) => dir.file_name().map_or_else(|| dir.display().to_string(), |name| name.to_string_lossy().into_owned()),
                (None, None) => "ssh".to_owned(),
            };
            let close = Button::new(("terminal-close", ix))
                .ghost()
                .xsmall()
                .label("×")
                .on_click(cx.listener(move |this, _, window, cx| this.close_terminal_tab(ix, window, cx)));
            Tab::new().label(name).suffix(close)
        });
        let tabs: Vec<Tab> = tabs.collect();
        let new = Button::new("terminal-new").ghost().xsmall().label("+").on_click(cx.listener(|this, _, window, cx| this.new_terminal_tab(window, cx)));
        TabBar::new("terminal-tabs")
            .small()
            .selected_index(self.terminal_tab)
            .children(tabs)
            .suffix(new)
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.select_terminal_tab(*ix, window, cx)))
    }

    /// The terminal under the panels, its top edge dragged for its height.
    pub(crate) fn render_terminal(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let tab = self.terminal_tab().filter(|_| self.terminal_shown)?;
        let edge = div()
            .id("terminal-edge")
            .absolute()
            .top(px(-3.))
            .left_0()
            .right_0()
            .h(px(6.))
            .occlude()
            .cursor(CursorStyle::ResizeUpDown)
            .on_drag(TerminalDrag, |_, _, _, cx| cx.new(|_| EmptyView));
        let pane = v_flex()
            .relative()
            .flex_none()
            .h(px(self.terminal_height))
            .border_1()
            .border_color(cx.theme().border)
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<TerminalDrag>, window, cx| {
                let most = f32::from(window.viewport_size().height) - 160.;
                this.terminal_height = f32::from(event.bounds.bottom() - event.event.position.y).clamp(80., most.max(80.));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &terminal::NewTerminalTab, window, cx| this.new_terminal_tab(window, cx)))
            .on_action(cx.listener(|this, _: &terminal::CloseTerminalTab, window, cx| this.close_terminal_tab(this.terminal_tab, window, cx)))
            .on_action(cx.listener(|this, _: &terminal::NextTerminalTab, window, cx| {
                let next = (this.terminal_tab + 1) % this.terminals.len().max(1);
                this.select_terminal_tab(next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &terminal::PreviousTerminalTab, window, cx| {
                let count = this.terminals.len().max(1);
                this.select_terminal_tab((this.terminal_tab + count - 1) % count, window, cx);
            }))
            .on_action(cx.listener(|this, Run(command): &Run, window, cx| {
                if *command == Command::TerminalPanel {
                    return this.toggle_terminal(window, cx);
                }
                // A panels' command from the menus: the panels take the keys again.
                let command = *command;
                cx.defer_in(window, move |this, window, cx| {
                    window.focus(&this.focus, cx);
                    this.focus.clone().dispatch_action(&Run(command), window, cx);
                });
            }))
            .child(self.render_terminal_tabs(cx))
            .child(div().flex_1().min_h_0().child(tab.view.clone()))
            .child(edge);
        // Its own background: the desktop may show through it.
        let tint = self.terminal_look.0.unwrap_or(1.);
        Some(v_flex().flex_none().px_1().bg(cx.theme().background.opacity(tint)).child(pane))
    }
}
