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
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::edit;
use pc_core::origin::{self, Upload};
use pc_core::remote::{self, Address, Ask, Prompt, Reply};
use pc_core::mounts::{self, Login};
use pc_core::secrets;

use crate::Commander;

impl Commander {
    /// `sftp://…`, `ftp://…`: connects in the background, then shows it on
    /// `side`; `smb://…` and the like the desktop mounts.
    pub(crate) fn connect(
        &mut self,
        side: usize,
        address: Address,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if address.scheme.mounted() {
            return self.mount_url(side, address.url(), window, cx);
        }
        if let Some(folder) = pc_core::cloud::mounted_at(&address) {
            let go = move |panel: &mut pc_core::Panel| match panel.remote() {
                Some(_) => panel.disconnect(folder),
                None => panel.go_to_folder(folder),
            };
            return self.navigate_on(side, true, go, window, cx);
        }
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
        let note = Notification::info(text).id1::<Connecting>(server.clone()).autohide(false);
        window.push_notification(note, cx);
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
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Connecting>(server.clone(), cx);
                match connected {
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
                }
            });
        })
        .detach();
    }

    /// The desktop mounts `url` (kio-fuse, gvfs) in the background; `side`
    /// shows the folder, read in the background too: it is on the network.
    pub(crate) fn mount_url(&mut self, side: usize, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.mount_as(side, url, None, false, window, cx);
    }

    /// As `login`; else with the password the keyring keeps for the
    /// address's user, else as a guest. Refused, it asks for a login.
    fn mount_as(&mut self, side: usize, url: String, login: Option<Login>, remember: bool, window: &mut Window, cx: &mut Context<Self>) {
        let note = Notification::info(format!("Connecting to {url}...")).id1::<Connecting>(url.clone()).autohide(false);
        window.push_notification(note, cx);
        let address = Address::parse(&url).filter(|address| address.scheme.mounted() && address.scheme.logs_in());
        let mounting = {
            let (url, address, login) = (url.clone(), address.clone(), login.clone());
            cx.background_executor().spawn(async move {
                let kept = |address: Address| {
                    let password = secrets::password(&remote::keyring_key(&address))?;
                    Some(Login { user: address.user?, password })
                };
                let login = login.or_else(|| kept(address.clone()?));
                let mounted = mounts::mount(&url, login.as_ref());
                if let (true, Ok(_), Some(mut address), Some(login)) = (remember, &mounted, address, &login) {
                    address.user = Some(login.user.clone());
                    // shortcut: a keyring that cannot keep it means asking again next time.
                    let _ = secrets::remember(&remote::keyring_key(&address), &login.password);
                }
                mounted.map_err(|err| (err, login.map(|login| login.user)))
            })
        };
        cx.spawn_in(window, async move |this, cx| {
            let mounted = mounting.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Connecting>(url.clone(), cx);
                match (mounted, address) {
                    (Ok(path), _) => {
                        let go = move |panel: &mut pc_core::Panel| match panel.remote() {
                            Some(_) => panel.disconnect(path),
                            None => panel.go_to_folder(path),
                        };
                        this.navigate_on(side, true, go, window, cx);
                    }
                    // Why is not told apart: kio-fuse says it in the desktop's language.
                    (Err((err, user)), Some(address)) => {
                        let user = user.or(address.user).unwrap_or_default();
                        let text = format!("{url}: {err}");
                        ask_login(cx.entity(), side, url, user, text, window, cx);
                    }
                    (Err((err, _)), None) => {
                        let text = format!("{url}: {err}");
                        window.push_notification(Notification::error(text).autohide(false), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Disconnect: the active panel goes home.
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
}

/// The "Connecting to…" note, gone once connected or failed.
struct Connecting;

/// A share's user and password, after `why` it would not open.
fn ask_login(commander: Entity<Commander>, side: usize, url: String, user: String, why: String, window: &mut Window, cx: &mut App) {
    let user = cx.new(|cx| InputState::new(window, cx).default_value(user));
    let password = cx.new(|cx| InputState::new(window, cx).masked(true));
    let focus = match user.read(cx).value().is_empty() {
        true => user.clone(),
        false => password.clone(),
    };
    let view = cx.new(|_| LoginDialog { why, commander, side, url, user, password, remember: false });
    window.open_dialog(cx, move |dialog, _, _| {
        let enter = view.clone();
        dialog
            .title("Log in")
            .width(px(520.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| {
                enter.update(cx, |this, cx| this.finish(window, cx));
                false
            })
    });
    focus.update(cx, |input, cx| input.focus(window, cx));
}

struct LoginDialog {
    why: String,
    commander: Entity<Commander>,
    side: usize,
    url: String,
    user: Entity<InputState>,
    password: Entity<InputState>,
    remember: bool,
}

impl LoginDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let user = self.user.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        let (side, url, remember) = (self.side, self.url.clone(), self.remember);
        window.close_dialog(cx);
        self.commander.update(cx, |commander, cx| {
            commander.mount_as(side, url, Some(Login { user, password }), remember, window, cx);
        });
    }
}

impl Render for LoginDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |label: &'static str, input: &Entity<InputState>| {
            h_flex().gap_3().child(div().w(px(80.)).flex_none().child(label)).child(div().flex_1().child(Input::new(input)))
        };
        let why = div().whitespace_normal().text_color(cx.theme().muted_foreground).child(self.why.clone());
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(why)
            .child(field("User", &self.user))
            .child(field("Password", &self.password))
            .child(
                Checkbox::new("login-remember")
                    .label("Remember it in the system keyring")
                    .checked(self.remember)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.remember = *checked;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("login-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(
                        Button::new("login-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Open {
    View,
    Edit,
    External,
}

/// Saves it back once the editor ends, if it changed the file. An editor
/// that returns at once (`xdg-open`, `code` without `--wait`) is not waited for.
pub(crate) fn edit_externally(
    local: PathBuf,
    upload: Upload,
    line: &str,
    window: &mut Window,
    cx: &mut App,
) -> std::io::Result<()> {
    let before = origin::modified(&local)?;
    let mut child = edit::start_editor(line, &local)?;
    let (done, ended) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = done.send(origin::send_back(&local, before, &upload));
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Some(uploaded)) = ended.await else {
                return;
            };
            let _ = cx.update(|window, cx| {
                let note = match uploaded {
                    Ok(text) => Notification::success(text),
                    Err(err) => Notification::error(format!("Could not save it back: {err}"))
                        .autohide(false),
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
            let view = cx.new(|_| SecretDialog {
                text,
                input: input.clone(),
                answer,
                keyring,
                remember: false,
                keep,
            });
            window.open_dialog(cx, move |dialog, _, _| {
                let enter = view.clone();
                dialog
                    .title(title.clone())
                    .width(px(460.))
                    .overlay_closable(false)
                    .child(view.clone())
                    .on_ok(move |_, window, cx| {
                        enter.update(cx, |this, cx| this.finish(window, cx));
                        false
                    })
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
