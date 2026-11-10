//! Servers, whatever the protocol: addresses, what connecting asks, and the
//! connection itself.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::secrets;
use crate::vfs::Remote;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scheme {
    #[default]
    Sftp,
    Ftp,
    /// FTP over TLS, explicit (AUTH TLS) on the FTP port.
    Ftps,
}

impl Scheme {
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Sftp => 22,
            Scheme::Ftp | Scheme::Ftps => 21,
        }
    }
}

/// `sftp://[user@]host[:port][/path]`, or `ftp://`, `ftps://`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub scheme: Scheme,
    pub user: Option<String>,
    /// For SFTP a `~/.ssh/config` host name, or the server's.
    pub host: String,
    pub port: Option<u16>,
    pub path: Option<String>,
}

impl Address {
    pub fn parse(text: &str) -> Option<Self> {
        let (scheme, rest) = text.trim().split_once("://")?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "sftp" => Scheme::Sftp,
            "ftp" => Scheme::Ftp,
            "ftps" => Scheme::Ftps,
            _ => return None,
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], Some(rest[at..].to_owned())),
            None => (rest, None),
        };
        let (user, server) = match authority.rsplit_once('@') {
            Some((user, server)) => (Some(user.to_owned()), server),
            None => (None, authority),
        };
        let (host, port) = match server.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => (host, port.parse().ok()),
            _ => (server, None),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        (!host.is_empty()).then(|| Address {
            scheme,
            user: user.filter(|user| !user.is_empty()),
            host: host.to_owned(),
            port,
            path,
        })
    }
}

/// What connecting asks the user.
#[derive(Clone, Debug)]
pub enum Prompt {
    /// The server's key, or TLS certificate, is not known yet.
    UnknownHost {
        host: String,
        key_type: String,
        fingerprint: String,
    },
    /// A password, or a key file's passphrase. A password has the key it
    /// would be kept under in the system keyring.
    Secret {
        title: String,
        text: String,
        keyring: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Trust { remember: bool },
    Secret(String),
    Cancel,
}

/// Answers a prompt; called on a thread of its own, so it may block.
pub type Ask = Arc<dyn Fn(Prompt) -> Reply + Send + Sync>;

/// The key the keyring keeps the server's password under, as the login asks for it.
pub fn keyring_key(address: &Address) -> String {
    match address.scheme {
        Scheme::Sftp => crate::sftp::keyring_key(address),
        scheme => {
            let user = address.user.as_deref().unwrap_or("anonymous");
            let port = address.port.unwrap_or(scheme.default_port());
            secrets::key(scheme.name(), user, &address.host, port)
        }
    }
}

/// Connects and logs in; a password the keyring has is tried before
/// asking. Blocks: run it off the UI thread. Gives the folder to show.
pub fn connect(address: &Address, ask: Ask) -> io::Result<(Arc<dyn Remote>, PathBuf)> {
    let tried = Arc::new(Mutex::new(HashSet::new()));
    let ask: Ask = Arc::new(move |prompt| {
        if let Prompt::Secret {
            keyring: Some(key), ..
        } = &prompt
            && tried.lock().unwrap().insert(key.clone())
            && let Some(password) = secrets::password(key)
        {
            return Reply::Secret(password);
        }
        ask(prompt)
    });
    match address.scheme {
        Scheme::Sftp => {
            let sftp = crate::sftp::Sftp::connect(address, ask)?;
            let start = sftp.start(address);
            Ok((sftp, start))
        }
        Scheme::Ftp | Scheme::Ftps => {
            let ftp = crate::ftp::Ftp::connect(address, ask)?;
            let start = ftp.start(address);
            Ok((ftp, start))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_addresses() {
        let address = |text| Address::parse(text);
        assert_eq!(
            address("sftp://me@example.org:2222/var/log"),
            Some(Address {
                scheme: Scheme::Sftp,
                user: Some("me".into()),
                host: "example.org".into(),
                port: Some(2222),
                path: Some("/var/log".into()),
            })
        );
        let alias = address("sftp://myserver").unwrap();
        assert_eq!(
            (alias.user, alias.host.as_str(), alias.port, alias.path),
            (None, "myserver", None, None)
        );
        assert_eq!(address("FTPS://[::1]:21/").unwrap().scheme, Scheme::Ftps);
        assert_eq!(
            address("ftp://anonymous@ftp.example.org").unwrap().scheme,
            Scheme::Ftp
        );
        assert!(address("sftp://").is_none());
        assert!(address("http://example.org").is_none());
        assert!(address("/home").is_none());
    }
}
