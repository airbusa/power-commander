//! A saved server's details, for the sidebar's Connections section.
//! Passwords go to the system keyring only.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::connections::Connection;
use pc_core::remote::{self, Scheme};
use pc_core::secrets;

/// Ours, the desktop's, then online storage, as the Protocol list shows them.
const SCHEMES: [(Scheme, &str); 8] = [
    (Scheme::Sftp, "SFTP"),
    (Scheme::Ftp, "FTP"),
    (Scheme::Ftps, "FTPS"),
    (Scheme::Smb, "SMB"),
    (Scheme::Webdav, "WebDAV"),
    (Scheme::Webdavs, "WebDAVS"),
    (Scheme::Nfs, "NFS"),
    (Scheme::S3, "S3"),
];

type Protocols = SelectState<Vec<&'static str>>;

type Saved = Rc<dyn Fn(Connection, &mut Window, &mut App)>;

/// A saved server's details; the password goes to the keyring.
pub(crate) fn open_edit(
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
    let at = SCHEMES.iter().position(|(scheme, _)| *scheme == connection.scheme);
    let labels = SCHEMES.map(|(_, label)| label).to_vec();
    let protocol = cx.new(|cx| SelectState::new(labels, at.map(IndexPath::new), window, cx));
    let view = cx.new(|cx| EditConnection {
        scheme: connection.scheme,
        _picked: cx.subscribe(&protocol, |this: &mut EditConnection, _, event: &SelectEvent<Vec<&'static str>>, cx| {
            let SelectEvent::Confirm(Some(label)) = event else { return };
            if let Some((scheme, _)) = SCHEMES.iter().find(|(_, it)| it == label) {
                this.scheme = *scheme;
                cx.notify();
            }
        }),
        protocol,
        fields: [name.clone(), host, port, user, path],
        password,
        forget: false,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let enter = view.clone();
        dialog
            .title("Connection")
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
    protocol: Entity<Protocols>,
    _picked: Subscription,
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
        if self.scheme.logs_in() && (!password.is_empty() || self.forget) {
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
        let (labels, secret) = match self.scheme {
            Scheme::S3 => (["Name", "Endpoint", "Port", "Access key", "Bucket/folder"], "Secret key"),
            _ => (["Name", "Server", "Port", "User", "Start folder"], "Password"),
        };
        let field = |label: &'static str, input: &Entity<InputState>| {
            h_flex()
                .gap_3()
                .child(div().w(px(96.)).flex_none().child(label))
                .child(div().flex_1().child(Input::new(input)))
        };
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
                    .child(div().flex_1().child(Select::new(&self.protocol))),
            )
            .children(
                labels[1..]
                    .iter()
                    .zip(rest)
                    .map(|(label, input)| field(label, input)),
            )
            .when(self.scheme.logs_in(), |el| {
                el.child(field(secret, &self.password)).child(
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
            })
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
