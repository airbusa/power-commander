//! Servers: connecting and what it asks, and files there for the viewer
//! and editors, which work on a temporary copy.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use futures::StreamExt;
use futures::channel::{mpsc as channel, oneshot};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::remote::{self, Address, Ask, Prompt, Reply};
use pc_core::secrets;
use pc_core::vfs::{self, Local};
use pc_core::{edit, vfs::Fs};

use crate::{Commander, editor, viewer};

impl Commander {
    /// `sftp://…`, `ftp://…`: connects in the background, then shows it on `side`.
    pub(crate) fn connect(
        &mut self,
        side: usize,
        address: Address,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (prompts, mut asked) = channel::unbounded::<(Prompt, mpsc::Sender<Reply>)>();
        // A password the user asked to keep, once it has logged in.
        let keep: Keep = Arc::default();
        let kept = keep.clone();
        let ask: Ask = Arc::new(move |prompt| {
            let (reply, answer) = mpsc::channel();
            match prompts.unbounded_send((prompt, reply)) {
                Ok(()) => answer.recv().unwrap_or(Reply::Cancel),
                Err(_) => Reply::Cancel,
            }
        });
        // The questions, for as long as the connection may ask.
        cx.spawn_in(window, async move |_, cx| {
            while let Some((prompt, reply)) = asked.next().await {
                let keep = keep.clone();
                let _ = cx.update(|window, cx| ask_user(prompt, reply, keep, window, cx));
            }
        })
        .detach();
        let server = address.host.clone();
        let text = format!("Connecting to {server}...");
        window.push_notification(Notification::info(text), cx);
        // A thread of its own: it may wait long on the user.
        let (done, connected) = oneshot::channel();
        std::thread::spawn(move || {
            let connected = remote::connect(&address, ask);
            if connected.is_ok()
                && let Some((key, password)) = kept.lock().unwrap().take()
            {
                // shortcut: a keyring that cannot keep it means asking again next time.
                let _ = secrets::remember(&key, &password);
            }
            let _ = done.send(connected);
        });
        cx.spawn_in(window, async move |this, cx| {
            let connected = connected
                .await
                .unwrap_or_else(|_| Err(std::io::Error::other("stopped")));
            let _ = this.update_in(cx, |this, window, cx| match connected {
                Ok((sftp, start)) => this.navigate_on(
                    side,
                    true,
                    move |panel| panel.go_remote(sftp, start),
                    window,
                    cx,
                ),
                Err(err) => {
                    let text = format!("{server}: {err}");
                    window.push_notification(Notification::error(text).autohide(false), cx);
                }
            });
        })
        .detach();
    }

    /// Net > Disconnect: the active panel goes home.
    pub(crate) fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panels[self.active].tabs.current().remote().is_none() {
            return;
        }
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        self.navigate_on(
            self.active,
            false,
            move |panel| panel.disconnect(home),
            window,
            cx,
        );
    }

    /// F3, F4 and Ctrl+Shift+F4 on a server's file: a temporary copy;
    /// what the editors save goes back.
    pub(crate) fn open_remote(&mut self, how: Open, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(entry) = panel.current_entry().filter(|e| !e.is_dir) else {
            return;
        };
        let (fs, path) = (panel.fs(), panel.folder().join(&entry.name));
        let (settings, bounds) = (self.settings.clone(), window.window_bounds());
        let name = entry.display_name();
        cx.spawn_in(window, async move |_, cx| {
            let fetch = (fs.clone(), path.clone());
            let fetched = cx
                .background_executor()
                .spawn(async move { vfs::fetch(&*fetch.0, &fetch.1) })
                .await;
            let _ = cx.update(|window, cx| {
                let opened = fetched.and_then(|local| match how {
                    Open::View => viewer::open(vec![local], 0, settings.viewer, bounds, cx),
                    Open::Edit => {
                        let upload = editor::Upload { fs, path };
                        editor::open(local, settings.editor.limit_mb, Some(upload), cx)
                    }
                    Open::External => {
                        edit_externally(local, fs, path, &settings.editor.external, window, cx)
                    }
                });
                if let Err(err) = opened {
                    window.push_notification(Notification::error(format!("{name}: {err}")), cx);
                }
            });
        })
        .detach();
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Open {
    View,
    Edit,
    External,
}

/// Uploads once the editor ends, if it changed the file. An editor that
/// returns at once (`xdg-open`, `code` without `--wait`) is not waited for.
fn edit_externally(
    local: PathBuf,
    fs: Arc<dyn Fs>,
    path: PathBuf,
    line: &str,
    window: &mut Window,
    cx: &mut App,
) -> std::io::Result<()> {
    let before = Local.metadata(&local)?.modified;
    let mut child = edit::start_editor(line, &local)?;
    let (done, ended) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let changed = Local
            .metadata(&local)
            .is_ok_and(|meta| meta.modified != before);
        let uploaded = changed.then(|| vfs::upload(&local, &*fs, &path).map(|()| path));
        let _ = done.send(uploaded);
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Some(uploaded)) = ended.await else {
                return;
            };
            let _ = cx.update(|window, cx| {
                let note = match uploaded {
                    Ok(path) => Notification::success(format!("Uploaded {}", path.display())),
                    Err(err) => {
                        Notification::error(format!("Could not upload: {err}")).autohide(false)
                    }
                };
                window.push_notification(note, cx);
            });
        })
        .detach();
    Ok(())
}

/// Sends Cancel if the dialog goes without an answer, so the connection
/// never waits for ever.
struct Answer(Option<mpsc::Sender<Reply>>);

impl Answer {
    fn send(&mut self, reply: Reply) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(reply);
        }
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        self.send(Reply::Cancel);
    }
}

/// The keyring key and password to keep after a login.
type Keep = Arc<Mutex<Option<(String, String)>>>;

fn ask_user(
    prompt: Prompt,
    reply: mpsc::Sender<Reply>,
    keep: Keep,
    window: &mut Window,
    cx: &mut App,
) {
    let answer = Rc::new(RefCell::new(Answer(Some(reply))));
    match prompt {
        Prompt::UnknownHost {
            host,
            key_type,
            fingerprint,
        } => {
            let text = format!(
                "The server {host} is not known yet. Its {key_type} fingerprint is\n\n{fingerprint}\n\n\
                 Connect only if this is the server you mean; its administrator can confirm the fingerprint."
            );
            window.open_dialog(cx, move |dialog, _, _| {
                let button = |label: &'static str, reply: Reply| {
                    let answer = answer.clone();
                    Button::new(label)
                        .label(label)
                        .on_click(move |_, window, cx| {
                            answer.borrow_mut().send(reply.clone());
                            window.close_dialog(cx);
                        })
                };
                dialog
                    .title("Unknown server")
                    .width(px(560.))
                    .keyboard(false)
                    .close_button(false)
                    .overlay_closable(false)
                    .child(
                        v_flex()
                            .gap_3()
                            .text_sm()
                            .child(div().whitespace_normal().child(text.clone()))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .justify_end()
                                    .child(
                                        button(
                                            "Connect and remember",
                                            Reply::Trust { remember: true },
                                        )
                                        .primary(),
                                    )
                                    .child(
                                        button("Connect once", Reply::Trust { remember: false })
                                            .outline(),
                                    )
                                    .child(button("Cancel", Reply::Cancel).outline()),
                            ),
                    )
            });
        }
        Prompt::Secret {
            title,
            text,
            keyring,
        } => {
            let input = cx.new(|cx| InputState::new(window, cx).masked(true));
            let view = cx.new(|cx| {
                let entered = cx.subscribe_in(
                    &input,
                    window,
                    |this: &mut SecretDialog, _, event, window, cx| {
                        if let InputEvent::PressEnter { .. } = event {
                            this.finish(window, cx);
                        }
                    },
                );
                SecretDialog {
                    text,
                    input: input.clone(),
                    answer,
                    keyring,
                    remember: false,
                    keep,
                    _entered: entered,
                }
            });
            window.open_dialog(cx, move |dialog, _, _| {
                dialog
                    .title(title.clone())
                    .width(px(460.))
                    .overlay_closable(false)
                    .child(view.clone())
            });
            input.update(cx, |input, cx| input.focus(window, cx));
        }
    }
}

struct SecretDialog {
    text: String,
    input: Entity<InputState>,
    answer: Rc<RefCell<Answer>>,
    /// A password's keyring key; a passphrase has none.
    keyring: Option<String>,
    remember: bool,
    keep: Keep,
    _entered: Subscription,
}

impl SecretDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let secret = self.input.read(cx).value().to_string();
        if let Some(key) = self.keyring.clone().filter(|_| self.remember) {
            *self.keep.lock().unwrap() = Some((key, secret.clone()));
        }
        self.answer.borrow_mut().send(Reply::Secret(secret));
        window.close_dialog(cx);
    }
}

impl Render for SecretDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.text.clone())
                    .child(Input::new(&self.input)),
            )
            .when(self.keyring.is_some(), |el| {
                el.child(
                    Checkbox::new("secret-remember")
                        .label("Remember it in the system keyring")
                        .checked(self.remember)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.remember = *checked;
                            cx.notify();
                        })),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("secret-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(
                        Button::new("secret-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer.borrow_mut().send(Reply::Cancel);
                                window.close_dialog(cx);
                            })),
                    ),
            )
    }
}
