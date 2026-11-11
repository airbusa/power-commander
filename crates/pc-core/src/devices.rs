//! Phones and tablets paired through KDE Connect. A device's files, once
//! mounted, are ordinary folders: KDE Connect mounts them with sshfs under
//! the runtime folder, as its own `kdeconnect:/` does.

use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// As KDE Connect has it: phone, tablet, desktop, laptop or tv.
    pub kind: String,
    pub reachable: bool,
    /// Reachable, and sharing its files.
    pub browsable: bool,
}

/// The paired devices; none where KDE Connect is not running. It is not
/// started for this: D-Bus would start it on the first call.
pub fn paired() -> Vec<Device> {
    // shortcut: a failing bus reads as no devices; the tab hides, as without KDE Connect.
    kde_connect::paired().unwrap_or_default()
}

/// Mounts the device's files: its folders with their names, the main one first.
pub fn mount(id: &str) -> io::Result<Vec<(PathBuf, String)>> {
    let folders = main_first(kde_connect::mount(id)?);
    match folders.is_empty() {
        true => Err(io::Error::other("the device shares no folder")),
        false => Ok(folders),
    }
}

/// The internal storage holds the camera's folder: the folder holding most
/// others first, then by depth. A card's `/storage/1234-5678` is shallower
/// than Android's `/storage/emulated/0`.
fn main_first(mut folders: Vec<(PathBuf, String)>) -> Vec<(PathBuf, String)> {
    let paths: Vec<PathBuf> = folders.iter().map(|(path, _)| path.clone()).collect();
    let holds = |path: &PathBuf| paths.iter().filter(|other| *other != path && other.starts_with(path)).count();
    folders.sort_by_cached_key(|(path, _)| (std::cmp::Reverse(holds(path)), path.components().count()));
    folders
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod kde_connect {
    use std::collections::HashMap;
    use std::io;
    use std::path::PathBuf;

    use zbus::blocking::Connection;
    use zbus::zvariant::OwnedValue;

    use super::Device;

    const SERVICE: &str = "org.kde.kdeconnect";
    const DEVICE: &str = "org.kde.kdeconnect.device";

    fn call<R>(bus: &Connection, path: &str, interface: &str, method: &str, args: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<R>
    where
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        bus.call_method(Some(SERVICE), path, Some(interface), method, args)?.body().deserialize()
    }

    fn property<T: TryFrom<OwnedValue>>(bus: &Connection, path: &str, name: &str) -> Option<T> {
        let value: OwnedValue = call(bus, path, "org.freedesktop.DBus.Properties", "Get", &(DEVICE, name)).ok()?;
        T::try_from(value).ok()
    }

    pub fn paired() -> zbus::Result<Vec<Device>> {
        let bus = Connection::session()?;
        let running: bool = bus
            .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &SERVICE)?
            .body()
            .deserialize()?;
        if !running {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = call(&bus, "/modules/kdeconnect", "org.kde.kdeconnect.daemon", "devices", &(false, true))?;
        let devices = ids.into_iter().map(|id| {
            let path = format!("/modules/kdeconnect/devices/{id}");
            let reachable = property(&bus, &path, "isReachable").unwrap_or(false);
            let sftp = call(&bus, &path, DEVICE, "hasPlugin", &"kdeconnect_sftp").unwrap_or(false);
            Device {
                name: property(&bus, &path, "name").unwrap_or_else(|| id.clone()),
                kind: property(&bus, &path, "type").unwrap_or_default(),
                id,
                reachable,
                browsable: reachable && sftp,
            }
        });
        Ok(devices.collect())
    }

    pub fn mount(id: &str) -> io::Result<Vec<(PathBuf, String)>> {
        let bus = Connection::session().map_err(io::Error::other)?;
        let path = format!("/modules/kdeconnect/devices/{id}/sftp");
        let sftp = "org.kde.kdeconnect.device.sftp";
        let mounted: bool = call(&bus, &path, sftp, "mountAndWait", &()).map_err(io::Error::other)?;
        if !mounted {
            let error: String = call(&bus, &path, sftp, "getMountError", &()).unwrap_or_default();
            let error = if error.is_empty() { "could not mount the device".to_owned() } else { error };
            return Err(io::Error::other(error));
        }
        let folders: HashMap<String, OwnedValue> = call(&bus, &path, sftp, "getDirectories", &()).map_err(io::Error::other)?;
        let named = folders.into_iter().map(|(path, name)| (PathBuf::from(path), String::try_from(name).unwrap_or_default()));
        Ok(named.collect())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod kde_connect {
    use std::io;
    use std::path::PathBuf;

    use super::Device;

    pub fn paired() -> Option<Vec<Device>> {
        None
    }

    pub fn mount(_: &str) -> io::Result<Vec<(PathBuf, String)>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn internal_storage_comes_first() {
        let folder = |path: &str| (std::path::PathBuf::from(path), String::new());
        let folders = vec![folder("/m/storage/1234-5678"), folder("/m/storage/emulated/0/DCIM/Camera"), folder("/m/storage/emulated/0")];
        let order: Vec<_> = super::main_first(folders).into_iter().map(|(path, _)| path).collect();
        assert_eq!(order[0], std::path::Path::new("/m/storage/emulated/0"));
        assert_eq!(order[1], std::path::Path::new("/m/storage/1234-5678"));
    }

    /// Reads the session's KDE Connect; run by hand with `--ignored`.
    #[test]
    #[ignore]
    fn lists_paired_devices() {
        for device in super::paired() {
            assert!(!device.id.is_empty() && !device.name.is_empty());
            println!("{device:?}");
        }
    }
}
