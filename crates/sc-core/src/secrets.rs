//! Server passwords in the system's keyring, never in a file. Calls may
//! wait on the keyring (KWallet asks to open the wallet): not on the UI thread.

use std::io;

use keyring::{Entry, Error};

const SERVICE: &str = "Spark Commander";
/// The app's old name, which passwords were kept under before.
const OLD_SERVICE: &str = "Power Commander";

/// The password kept under `key` (`sftp://user@host:22`), if any; one kept
/// under the old name is moved to the new.
pub fn password(key: &str) -> Option<String> {
    let found = |service| Entry::new(service, key).ok()?.get_password().ok();
    found(SERVICE).or_else(|| {
        let password = found(OLD_SERVICE)?;
        if remember(key, &password).is_ok() {
            let _ = delete(OLD_SERVICE, key);
        }
        Some(password)
    })
}

pub fn remember(key: &str, password: &str) -> io::Result<()> {
    let entry = Entry::new(SERVICE, key).map_err(io::Error::other)?;
    entry.set_password(password).map_err(io::Error::other)
}

/// Also under the old name, so it does not come back from there.
pub fn forget(key: &str) -> io::Result<()> {
    delete(OLD_SERVICE, key)?;
    delete(SERVICE, key)
}

fn delete(service: &str, key: &str) -> io::Result<()> {
    let entry = Entry::new(service, key).map_err(io::Error::other)?;
    match entry.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(err) => Err(io::Error::other(err)),
    }
}

/// The key a server's password is kept under.
pub fn key(scheme: &str, user: &str, host: &str, port: u16) -> String {
    format!("{scheme}://{user}@{host}:{port}")
}
