//! Saved servers, and the hosts `~/.ssh/config` names.

use std::io;

use serde::{Deserialize, Serialize};

use crate::remote::{self, Address, Scheme};
use crate::{secrets, settings};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Connection {
    pub name: String,
    pub scheme: Scheme,
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    /// The folder it starts in; the login folder if none.
    pub path: Option<String>,
}

impl Connection {
    pub fn address(&self) -> Address {
        Address {
            scheme: self.scheme,
            user: self.user.clone().filter(|user| !user.is_empty()),
            host: self.host.trim().to_owned(),
            port: self.port,
            path: self.path.clone().filter(|path| path.starts_with('/')),
        }
    }

    /// `sftp://user@host:2222/path`, as the path field takes it.
    pub fn url(&self) -> String {
        self.address().url()
    }
}

/// Kept in `connections.json`; passwords are in the system keyring.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Connections {
    pub saved: Vec<Connection>,
}

impl Connections {
    pub fn load() -> io::Result<Self> {
        settings::read_json("connections.json")
    }

    pub fn save(&self) -> io::Result<()> {
        settings::write_json("connections.json", self)
    }
}

/// After `gone` left the saved: its password too, unless another logs in with it.
pub fn forget_password(saved: &[Connection], gone: &Connection) {
    let key = remote::keyring_key(&gone.address());
    if !saved.iter().any(|kept| remote::keyring_key(&kept.address()) == key) {
        std::thread::spawn(move || secrets::forget(&key));
    }
}

/// The `Host` names of `~/.ssh/config`, without patterns.
pub fn ssh_hosts() -> Vec<String> {
    let home = std::env::home_dir().unwrap_or_default();
    let text = std::fs::read_to_string(home.join(".ssh").join("config")).unwrap_or_default();
    hosts_in(&text)
}

fn hosts_in(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("host"))
        {
            continue;
        }
        for host in words.filter(|host| !host.contains(['*', '?', '!'])) {
            if !hosts.iter().any(|known| known == host) {
                hosts.push(host.to_owned());
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_servers() {
        let config = "Host *\n  ServerAliveInterval 30\nHost web web2 !bad\n  HostName example.org\nhost db?\nHOST backup\n";
        assert_eq!(hosts_in(config), ["web", "web2", "backup"]);
        let saved = Connection {
            name: "Mine".into(),
            scheme: Scheme::Ftps,
            host: " ftp.example.org ".into(),
            port: Some(990),
            user: Some("me".into()),
            path: Some("/pub".into()),
        };
        assert_eq!(saved.url(), "ftps://me@ftp.example.org:990/pub");
        assert_eq!(Address::parse(&saved.url()), Some(saved.address()));
        let plain = Connection {
            host: "web".into(),
            ..Connection::default()
        };
        assert_eq!(plain.url(), "sftp://web");
    }
}
