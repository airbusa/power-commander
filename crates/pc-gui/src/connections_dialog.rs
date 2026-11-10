//! Net > Connect to Server: an address typed, a saved server, or a host of
//! `~/.ssh/config`. Passwords go to the system keyring only.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::connections::{self, Connection, Connections};
use pc_core::remote::{self, Address, Scheme};
use pc_core::secrets;

use crate::Commander;

const SCHEMES: [Scheme; 3] = [Scheme::Sftp, Scheme::Ftp, Scheme::Ftps];

pub fn open(commander: WeakEntity<Commander>, side: usize, window: &mut Window, cx: &mut App) {
    let address = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("sftp://user@host:port/path, ftp://, ftps://, or a host below")
    });
    let view = cx.new(|_| {
        let saved = Connections::load().unwrap_or_default().saved;
        let hosts = connections::ssh_hosts();
        let selected = match (saved.is_empty(), hosts.is_empty()) {
            (false, _) => Some(Pick::Saved(0)),
            (true, false) => Some(Pick::Host(0)),
            (true, true) => None,
        };
        ConnectionsDialog {
            commander,
            side,
            selected,
            saved,
            hosts,
            address: address.clone(),
        }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let enter = view.clone();
        dialog
            .title("Connect to server")
            .width(px(640.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| {
                enter.update(cx, |this, cx| this.connect_typed(window, cx));
                false
            })
    });
    crate::pack_dialog::focus_all(&address, window, cx);
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Saved(usize),
    Host(usize),
}

struct ConnectionsDialog {
    commander: WeakEntity<Commander>,
    side: usize,
    saved: Vec<Connection>,
    hosts: Vec<String>,
    selected: Option<Pick>,
    address: Entity<InputState>,
}

impl ConnectionsDialog {
    fn connect(&mut self, address: Address, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let side = self.side;
        let _ = self.commander.update(cx, |commander, cx| {
            commander.connect(side, address, window, cx)
        });
    }

    /// The scheme is optional: SFTP.
    fn connect_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.address.read(cx).value().trim().to_string();
        if text.is_empty() {
            return self.connect_selected(window, cx);
        }
        let text = match text.contains("://") {
            true => text,
            false => format!("sftp://{text}"),
        };
        match Address::parse(&text) {
            Some(address) => self.connect(address, window, cx),
            None => {
                window.push_notification(Notification::error(format!("Not an address: {text}")), cx)
            }
        }
    }

    fn connect_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = match self.selected {
            Some(Pick::Saved(ix)) => self.saved[ix].address(),
            Some(Pick::Host(ix)) => Address {
                scheme: Scheme::Sftp,
                user: None,
                host: self.hosts[ix].clone(),
                port: None,
                path: None,
            },
            None => return,
        };
        self.connect(address, window, cx);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = Connections {
            saved: self.saved.clone(),
        };
        if let Err(err) = saved.save() {
            let note = Notification::error(format!("Could not save the connections: {err}"));
            window.push_notification(note, cx);
        }
        cx.notify();
    }

    /// New (none), edit or duplicate a saved one.
    fn edit(
        &mut self,
        editing: Option<usize>,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut connection = editing.map(|ix| self.saved[ix].clone()).unwrap_or_default();
        if duplicate {
            connection.name = format!("{} (copy)", connection.name);
        }
        let replace = editing.filter(|_| !duplicate);
        let this = cx.entity().downgrade();
        open_edit(connection, window, cx, move |connection, window, cx| {
            let _ = this.update(cx, |this, cx| {
                let ix = match replace {
                    Some(ix) => {
                        this.saved[ix] = connection;
                        ix
                    }
                    None => {
                        this.saved.push(connection);
                        this.saved.len() - 1
                    }
                };
                this.selected = Some(Pick::Saved(ix));
                this.save(window, cx);
            });
        });
    }

    fn delete(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let gone = self.saved.remove(ix);
        let key = remote::keyring_key(&gone.address());
        // Another saved one may log in with that password.
        if !self
            .saved
            .iter()
            .any(|kept| remote::keyring_key(&kept.address()) == key)
        {
            std::thread::spawn(move || secrets::forget(&key));
        }
        self.selected = (!self.saved.is_empty()).then(|| Pick::Saved(ix.min(self.saved.len() - 1)));
        self.save(window, cx);
    }

    fn row(
        &self,
        pick: Pick,
        title: String,
        detail: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let id = match pick {
            Pick::Saved(ix) => ("connection", ix),
            Pick::Host(ix) => ("ssh-host", ix),
        };
        let selected = self.selected == Some(pick);
        h_flex()
            .id(id)
            .px_2()
            .py_1()
            .gap_3()
            .when(selected, |el| {
                el.bg(theme.accent).text_color(theme.accent_foreground)
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.selected = Some(pick);
                cx.notify();
                if event.click_count() == 2 {
                    this.connect_selected(window, cx);
                }
            }))
            .child(div().flex_none().child(title))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(!selected, |el| el.text_color(theme.muted_foreground))
                    .child(detail),
            )
    }
}

impl Render for ConnectionsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let heading = |text: &'static str| {
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_xs()
                .text_color(muted)
                .child(text)
        };
        let saved: Vec<_> = (0..self.saved.len())
            .map(|ix| {
                let connection = &self.saved[ix];
                let name = match connection.name.trim() {
                    "" => connection.host.clone(),
                    name => name.to_owned(),
                };
                self.row(Pick::Saved(ix), name, connection.url(), cx)
            })
            .collect();
        let hosts: Vec<_> = (0..self.hosts.len())
            .map(|ix| self.row(Pick::Host(ix), self.hosts[ix].clone(), "sftp".into(), cx))
            .collect();
        let empty = saved.is_empty() && hosts.is_empty();
        let saved_pick = match self.selected {
            Some(Pick::Saved(ix)) => Some(ix),
            _ => None,
        };
        let button =
            |id: &'static str, label: &'static str| Button::new(id).outline().label(label).w_full();
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.address)))
                    .child(
                        Button::new("connect-typed")
                            .primary()
                            .label("Connect")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.connect_typed(window, cx)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .id("connections")
                            .flex_1()
                            .h(px(300.))
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(border)
                            .when(!saved.is_empty(), |el| {
                                el.child(heading("Saved")).children(saved)
                            })
                            .when(!hosts.is_empty(), |el| {
                                el.child(heading("~/.ssh/config")).children(hosts)
                            })
                            .when(empty, |el| {
                                el.child(
                                    div()
                                        .p_3()
                                        .text_color(muted)
                                        .child("No saved servers yet: New... adds one."),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .w(px(120.))
                            .gap_2()
                            .child(
                                button("connection-connect", "Connect")
                                    .disabled(self.selected.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.connect_selected(window, cx)
                                    })),
                            )
                            .child(button("connection-new", "New...").on_click(cx.listener(
                                |this, _, window, cx| this.edit(None, false, window, cx),
                            )))
                            .child(
                                button("connection-edit", "Edit...")
                                    .disabled(saved_pick.is_none())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit(saved_pick, false, window, cx)
                                    })),
                            )
                            .child(
                                button("connection-duplicate", "Duplicate")
                                    .disabled(saved_pick.is_none())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit(saved_pick, true, window, cx)
                                    })),
                            )
                            .child(
                                button("connection-delete", "Delete")
                                    .disabled(saved_pick.is_none())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(ix) = saved_pick {
                                            this.delete(ix, window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("connections-close")
                        .outline()
                        .label("Close")
                        .min_w(px(80.))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    }
}

type Saved = Rc<dyn Fn(Connection, &mut Window, &mut App)>;

/// A saved server's details; the password goes to the keyring.
fn open_edit(
    connection: Connection,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Connection, &mut Window, &mut App) + 'static,
) {
    let mut input = |value: Option<String>, placeholder: &str, cx: &mut App| {
        let placeholder = placeholder.to_owned();
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value.unwrap_or_default())
        })
    };
    let name = input(Some(connection.name.clone()), "as the list shows it", cx);
    let host = input(
        Some(connection.host.clone()),
        "server, or a host of ~/.ssh/config",
        cx,
    );
    let port = input(connection.port.map(|port| port.to_string()), "default", cx);
    let user = input(
        connection.user.clone(),
        "as ~/.ssh/config says, or anonymous",
        cx,
    );
    let path = input(connection.path.clone(), "the login folder", cx);
    let password = cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder("kept in the system keyring; empty keeps it")
    });
    let view = cx.new(|_| EditConnection {
        scheme: connection.scheme,
        fields: [name.clone(), host, port, user, path],
        password,
        forget: false,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let enter = view.clone();
        dialog
            .title("Server")
            .width(px(520.))
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| {
                enter.update(cx, |this, cx| this.finish(window, cx));
                false
            })
    });
    crate::pack_dialog::focus_all(&name, window, cx);
}

struct EditConnection {
    scheme: Scheme,
    /// Name, host, port, user, path.
    fields: [Entity<InputState>; 5],
    password: Entity<InputState>,
    forget: bool,
    done: Saved,
}

impl EditConnection {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [name, host, port, user, path] = self
            .fields
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_string());
        if host.is_empty() {
            return window.push_notification(Notification::error("A server is needed."), cx);
        }
        let port = match port.as_str() {
            "" => None,
            text => match text.parse() {
                Ok(port) => Some(port),
                Err(_) => {
                    return window
                        .push_notification(Notification::error(format!("Not a port: {text}")), cx);
                }
            },
        };
        let some = |text: String| (!text.is_empty()).then_some(text);
        let connection = Connection {
            name,
            scheme: self.scheme,
            host,
            port,
            user: some(user),
            path: some(path),
        };
        let password = self.password.read(cx).value().to_string();
        if !password.is_empty() || self.forget {
            let key = remote::keyring_key(&connection.address());
            let forget = self.forget;
            std::thread::spawn(move || match forget {
                true => secrets::forget(&key),
                false => secrets::remember(&key, &password),
            });
        }
        (self.done)(connection, window, cx);
        window.close_dialog(cx);
    }
}

impl Render for EditConnection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = ["Name", "Server", "Port", "User", "Start folder"];
        let field = |label: &'static str, input: &Entity<InputState>| {
            h_flex()
                .gap_3()
                .child(div().w(px(96.)).flex_none().child(label))
                .child(div().flex_1().child(Input::new(input)))
        };
        let selected = SCHEMES.iter().position(|scheme| *scheme == self.scheme);
        let [name, rest @ ..] = &self.fields;
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(field(labels[0], name))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().w(px(96.)).flex_none().child("Protocol"))
                    .child(
                        RadioGroup::horizontal("connection-scheme")
                            .children(["SFTP", "FTP", "FTPS"])
                            .selected_index(selected)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.scheme = SCHEMES[*ix];
                                cx.notify();
                            })),
                    ),
            )
            .children(
                labels[1..]
                    .iter()
                    .zip(rest)
                    .map(|(label, input)| field(label, input)),
            )
            .child(field("Password", &self.password))
            .child(
                h_flex().gap_3().child(div().w(px(96.)).flex_none()).child(
                    Checkbox::new("connection-forget")
                        .label("Forget the kept password")
                        .checked(self.forget)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.forget = *checked;
                            cx.notify();
                        })),
                ),
            )
            .child(
                h_flex()
                    .pt_2()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("connection-ok")
                            .primary()
                            .label("OK")
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(
                        Button::new("connection-cancel")
                            .outline()
                            .label("Cancel")
                            .min_w(px(80.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
