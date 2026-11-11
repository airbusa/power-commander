//! The protocols our own clients do not speak (SMB, WebDAV, NFS, MTP, Google
//! Drive…), through the desktop's own bridge: KDE's kio-fuse, else GNOME's
//! gvfs. Either mounts the address as a folder, which the panels browse as
//! they do this computer's; the desktop asks for passwords itself.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `smb://…`, `mtp:/`, `gdrive:/…`: an address, not a path. Windows' `C:/` is a path.
pub fn is_url(text: &str) -> bool {
    let Some((scheme, rest)) = text.trim().split_once(':') else {
        return false;
    };
    let named = scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    named && rest.starts_with('/') && !scheme.eq_ignore_ascii_case("file")
}

/// Mounts `url`; the folder it is at. Blocks while the desktop logs in,
/// which may ask the user: run it off the UI thread.
pub fn mount(url: &str) -> io::Result<PathBuf> {
    let url = url.trim();
    let mounted = match kio_fuse::available() {
        true => kio_fuse::mount(url),
        false => gvfs::mount(url),
    };
    // kio-fuse may have started for it: its folder is read again.
    *ROOTS.lock().unwrap() = None;
    mounted
}

/// Where the desktops keep the folders they mount, found once.
static ROOTS: Mutex<Option<Vec<Root>>> = Mutex::new(None);

#[derive(Clone)]
enum Root {
    /// `<root>/<scheme>/<host>/<path>`.
    KioFuse(PathBuf),
    /// `<root>/smb-share:server=nas,share=media/<path>`.
    Gvfs(PathBuf),
}

/// The address a mounted folder came from, for the path bar; none for
/// this computer's own folders.
pub fn url_of(path: &Path) -> Option<String> {
    let mut roots = ROOTS.lock().unwrap();
    let roots = roots.get_or_insert_with(|| {
        let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        find_roots(&mounts, runtime.as_deref())
    });
    roots.iter().find_map(|root| address_in(root, path))
}

fn find_roots(mounts: &str, runtime: Option<&Path>) -> Vec<Root> {
    let kio = mounts.lines().filter_map(|line| {
        let mut fields = line.split(' ').skip(1);
        let (path, kind) = (fields.next()?, fields.next()?);
        (kind == "fuse.kio-fuse").then(|| Root::KioFuse(PathBuf::from(path)))
    });
    let gvfs = runtime.map(|runtime| Root::Gvfs(runtime.join("gvfs")));
    kio.chain(gvfs).collect()
}

fn address_in(root: &Root, path: &Path) -> Option<String> {
    let url = |scheme: &str, host: &str, rest: &Path| {
        let rest = rest.to_string_lossy();
        let rest = if rest.is_empty() { String::new() } else { format!("/{rest}") };
        format!("{scheme}://{host}{rest}")
    };
    match root {
        Root::KioFuse(root) => {
            let mut parts = path.strip_prefix(root).ok()?.iter();
            let (scheme, host) = (parts.next()?.to_str()?, parts.next()?.to_str()?);
            Some(url(scheme, host, parts.as_path()))
        }
        Root::Gvfs(root) => {
            let mut parts = path.strip_prefix(root).ok()?.iter();
            let name = parts.next()?.to_str()?;
            let (scheme, fields) = name.split_once(':')?;
            let field = |key: &str| fields.split(',').find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='));
            let (scheme, host) = match (scheme, field("server"), field("share"), field("host")) {
                ("smb-share", Some(server), Some(share), _) => ("smb", format!("{server}/{share}")),
                ("dav", _, _, Some(host)) if field("ssl") == Some("true") => ("webdavs", host.to_owned()),
                ("dav", _, _, Some(host)) => ("webdav", host.to_owned()),
                (scheme, _, _, Some(host)) => (scheme, host.to_owned()),
                (scheme, Some(server), _, _) => (scheme, server.to_owned()),
                _ => return None,
            };
            Some(url(scheme, &host, parts.as_path()))
        }
    }
}

/// KDE's: `org.kde.KIOFuse` on the session bus, started on demand.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod kio_fuse {
    use std::io;
    use std::path::PathBuf;

    use zbus::blocking::Connection;

    const SERVICE: &str = "org.kde.KIOFuse";

    /// Running, or startable by D-Bus.
    pub fn available() -> bool {
        let found = || -> zbus::Result<bool> {
            let bus = Connection::session()?;
            let ask = |method: &str| -> zbus::Result<Vec<String>> {
                let names = bus.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), method, &())?;
                names.body().deserialize()
            };
            let has = |names: Vec<String>| names.iter().any(|name| name == SERVICE);
            Ok(has(ask("ListNames")?) || has(ask("ListActivatableNames")?))
        };
        found().unwrap_or(false)
    }

    pub fn mount(url: &str) -> io::Result<PathBuf> {
        let bus = Connection::session().map_err(io::Error::other)?;
        let reply = bus.call_method(Some(SERVICE), "/org/kde/KIOFuse", Some("org.kde.KIOFuse.VFS"), "mountUrl", &url);
        let path: String = reply.and_then(|reply| reply.body().deserialize()).map_err(|err| match err {
            // KIO's own words: "Could not connect to host nas.", "Access denied to …"
            zbus::Error::MethodError(_, Some(text), _) => io::Error::other(text),
            err => io::Error::other(err),
        })?;
        Ok(PathBuf::from(path))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod kio_fuse {
    pub fn available() -> bool {
        false
    }

    pub fn mount(_: &str) -> std::io::Result<std::path::PathBuf> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// GNOME's: `gio mount`, then the folder gvfs-fuse gives the address.
mod gvfs {
    use std::io;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    pub fn mount(url: &str) -> io::Result<PathBuf> {
        // gvfs names WebDAV dav.
        let url = match url.split_once("://") {
            Some(("webdav", rest)) => format!("dav://{rest}"),
            Some(("webdavs", rest)) => format!("davs://{rest}"),
            _ => url.to_owned(),
        };
        let gio = |args: &[&str]| Command::new("gio").args(args).stdin(Stdio::null()).output();
        let mounted = gio(&["mount", &url]).map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => io::Error::other("needs KDE's kio-fuse or GNOME's gvfs"),
            _ => err,
        })?;
        let said = String::from_utf8_lossy(&mounted.stderr).into_owned();
        // Mounted before is no failure; a password it cannot ask for is.
        // shortcut: gio asks on a terminal; a password must be in the keyring, or mounted once in Files.
        if !mounted.status.success() && !said.contains("already mounted") {
            return Err(io::Error::other(said.trim().trim_start_matches("gio: ").to_owned()));
        }
        let info = gio(&["info", &url])?;
        let info = String::from_utf8_lossy(&info.stdout);
        let local = info.lines().find_map(|line| line.trim().strip_prefix("local path:"));
        local
            .map(|path| PathBuf::from(path.trim()))
            .ok_or_else(|| io::Error::other("mounted, but there is no folder for it: gvfs-fuse is missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_addresses_from_paths() {
        for url in ["smb://nas/share", "mtp:/", "gdrive:/My Drive", "webdavs://cloud.example.org/remote.php"] {
            assert!(is_url(url), "{url}");
        }
        for path in ["/home/me", "C:/Windows", "file:///home", "notes.txt", "a:b"] {
            assert!(!is_url(path), "{path}");
        }
    }

    #[test]
    fn maps_mounted_folders_back_to_addresses() {
        let mounts = "/dev/sda1 / btrfs rw 0 0\nkio-fuse /run/user/1000/kio-fuse-AbC fuse.kio-fuse rw 0 0\n";
        let roots = find_roots(mounts, Some(Path::new("/run/user/1000")));
        let url = |path: &str| roots.iter().find_map(|root| address_in(root, Path::new(path)));
        assert_eq!(url("/run/user/1000/kio-fuse-AbC/smb/nas/media/Films").as_deref(), Some("smb://nas/media/Films"));
        assert_eq!(url("/run/user/1000/kio-fuse-AbC/webdav/127.0.0.1:8088").as_deref(), Some("webdav://127.0.0.1:8088"));
        let gvfs = "/run/user/1000/gvfs/smb-share:server=nas,share=media/Films";
        assert_eq!(url(gvfs).as_deref(), Some("smb://nas/media/Films"));
        let dav = "/run/user/1000/gvfs/dav:host=cloud.example.org,ssl=true/docs";
        assert_eq!(url(dav).as_deref(), Some("webdavs://cloud.example.org/docs"));
        assert_eq!(url("/home/me"), None);
    }
}
