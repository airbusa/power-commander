//! Server passwords in the system's keyring, never in a file. Calls may
//! wait on the keyring (KWallet asks to open the wallet): not on the UI thread.

use std::io;

use keyring::{Entry, Error};

const SERVICE: &str = "Power Commander";

/// The password kept under `key` (`sftp://user@host:22`), if any.
pub fn password(key: &str) -> Option<String> {
    Entry::new(SERVICE, key).ok()?.get_password().ok()
}

pub fn remember(key: &str, password: &str) -> io::Result<()> {
    let entry = Entry::new(SERVICE, key).map_err(io::Error::other)?;
    entry.set_password(password).map_err(io::Error::other)
}

pub fn forget(key: &str) -> io::Result<()> {
    let entry = Entry::new(SERVICE, key).map_err(io::Error::other)?;
    match entry.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(err) => Err(io::Error::other(err)),
    }
}

/// The key a server's password is kept under.
pub fn key(scheme: &str, user: &str, host: &str, port: u16) -> String {
    format!("{scheme}://{user}@{host}:{port}")
}
