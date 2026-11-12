//! Online storage through rclone (vendor/rclone): each connection is `rclone
//! serve sftp --stdio`, which our SFTP client speaks. S3 (Amazon's and the
//! stores speaking it: Backblaze B2, Cloudflare R2, Wasabi, MinIO…), and the
//! remotes of the user's own rclone config.

use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::connections::Connections;
use crate::remote::{self, Address, Ask, Prompt, Reply};
use crate::secrets;
use crate::sftp::Sftp;
use crate::vfs::Fs;

const TRIES: usize = 3;

/// Connects to the store, asking for the secret key until the store takes it. Blocks.
pub fn connect(address: &Address, ask: &Ask) -> io::Result<Arc<Sftp>> {
    let program = ours().or_else(system_rclone).ok_or_else(missing)?;
    let key = address.user.clone().unwrap_or_default();
    for attempt in 1..=TRIES {
        let wrong = if attempt > 1 { "Wrong key. " } else { "" };
        let prompt = Prompt::Secret {
            title: "Secret key".into(),
            text: format!("{wrong}Secret key of {key} at {}:", address.host),
            keyring: Some(remote::keyring_key(address)),
        };
        let Reply::Secret(secret) = ask(prompt) else {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        };
        let mut command = rclone_command(&program, None);
        command.args(["serve", "sftp", "--stdio", ":s3:"]);
        // Secrets go by the environment, which only the user's own programs can read.
        command.envs(s3_options(address)).env("RCLONE_S3_ACCESS_KEY_ID", &key).env("RCLONE_S3_SECRET_ACCESS_KEY", secret);
        let server = Address { path: None, ..address.clone() };
        let store = Sftp::over_pipe(server.url(), command)?;
        // The store hears the key first when listed.
        match store.list(&store.start(address)) {
            Ok(_) => return Ok(store),
            Err(err) if refused(&err) && attempt < TRIES => continue,
            Err(err) => return Err(err),
        }
    }
    unreachable!("the last try returns")
}

/// One of the user's rclone remotes, `rclone://name/folder`, by their own
/// rclone and config; an encrypted config's password is asked. Blocks.
pub fn connect_remote(address: &Address, ask: &Ask) -> io::Result<Arc<Sftp>> {
    let config = user_config()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no rclone config found: choose yours in Connect to server"))?;
    let encrypted = std::fs::read_to_string(&config).is_ok_and(|text| text.contains("RCLONE_ENCRYPT_V0:"));
    let url = Address { path: None, ..address.clone() }.url();
    let serve = |program: &Path, password: Option<&str>| {
        let mut command = rclone_command(program, Some(&config));
        command.args(["serve", "sftp", "--stdio", &format!("{}:", address.host)]);
        command.envs(password.map(|password| ("RCLONE_CONFIG_PASS", password)));
        Sftp::over_pipe(url.clone(), command)
    };
    // Theirs knows every service their config may use; ours stands in for none, or one too old to serve on a pipe.
    let theirs = system_rclone();
    let program = theirs.clone().or_else(ours).ok_or_else(missing)?;
    for attempt in 1..=TRIES {
        let password = match encrypted {
            true => {
                let wrong = if attempt > 1 { "Wrong password. " } else { "" };
                let prompt = Prompt::Secret {
                    title: "rclone config".into(),
                    text: format!("{wrong}Password of {}:", config.display()),
                    keyring: Some(secrets::key("rclone", "config", &config.display().to_string(), 0)),
                };
                match ask(prompt) {
                    Reply::Secret(password) => Some(password),
                    _ => return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
                }
            }
            false => None,
        };
        let mut served = serve(&program, password.as_deref());
        if let (Err(err), Some(ours)) = (&served, ours())
            && err.to_string().contains("unknown flag")
            && theirs.is_some()
        {
            served = serve(&ours, password.as_deref());
        }
        match served {
            Err(err) if encrypted && err.to_string().contains("decrypt") && attempt < TRIES => continue,
            served => return served,
        }
    }
    unreachable!("the last try returns")
}

/// The user's own rclone config: the one chosen in Connect to server, else
/// the first of where rclone looks for it.
pub fn user_config() -> Option<PathBuf> {
    let chosen = Connections::load().ok().and_then(|connections| connections.rclone_config);
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    let home = std::env::home_dir();
    let places = [
        chosen,
        env("RCLONE_CONFIG"),
        env("APPDATA").filter(|_| cfg!(windows)).map(|dir| dir.join("rclone").join("rclone.conf")),
        env("XDG_CONFIG_HOME").map(|dir| dir.join("rclone").join("rclone.conf")),
        home.as_ref().map(|home| home.join(".config").join("rclone").join("rclone.conf")),
        home.map(|home| home.join(".rclone.conf")),
    ];
    places.into_iter().flatten().find(|path| path.is_file())
}

/// The user's rclone remotes, name and type, as the connections list them;
/// one their rclone mounts says so.
pub fn user_remotes() -> Vec<(String, String)> {
    let text = user_config().and_then(|config| std::fs::read_to_string(config).ok());
    let mut remotes = text.map(|text| remotes_in(&text)).unwrap_or_default();
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for (name, kind) in &mut remotes {
        if mount_of(&mounts, name).is_some() {
            kind.push_str(", mounted");
        }
    }
    remotes
}

/// The folder an rclone address is at when the user's rclone mounts its
/// remote: no second rclone for it then.
pub fn mounted_at(address: &Address) -> Option<PathBuf> {
    // shortcut: Linux's mounts only; macOS's and Windows' are in Devices > Cloud all the same.
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let root = mount_of(&mounts, &address.host)?;
    Some(root.join(address.path.as_deref().unwrap_or("").trim_start_matches('/')))
}

/// Where `/proc/self/mounts` has `remote`'s root mounted by `rclone mount`.
fn mount_of(mounts: &str, remote: &str) -> Option<PathBuf> {
    mounts.lines().find_map(|line| {
        let mut fields = line.split(' ');
        let (device, path, kind) = (fields.next()?, fields.next()?, fields.next()?);
        let root = crate::devices::unescape(device).as_os_str() == format!("{remote}:").as_str();
        (kind == "fuse.rclone" && root).then(|| crate::devices::unescape(path))
    })
}

/// A config's `[name]` sections and their `type`.
fn remotes_in(text: &str) -> Vec<(String, String)> {
    // shortcut: an encrypted config lists none; typing rclone://name opens one, asking the password.
    let mut remotes: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            remotes.push((name.to_owned(), String::new()));
        } else if let Some((key, value)) = line.split_once('=')
            && key.trim() == "type"
            && let Some((_, kind)) = remotes.last_mut()
        {
            *kind = value.trim().to_owned();
        }
    }
    remotes
}

/// rclone with the given config, or none, asking nothing on its pipe.
fn rclone_command(program: &Path, config: Option<&Path>) -> Command {
    let mut command = Command::new(program);
    command.arg("--config").arg(config.map_or_else(Default::default, Path::as_os_str));
    // A short directory cache: a refresh shows what changed elsewhere.
    command.args(["--ask-password=false", "--dir-cache-time", "5s", "--poll-interval", "0"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

const RCLONE: &str = if cfg!(windows) { "rclone.exe" } else { "rclone" };

/// Ours: vendor/rclone, or `rclone/` beside the app.
fn ours() -> Option<PathBuf> {
    Some(crate::bundled::path("rclone", RCLONE)).filter(|path| path.is_file())
}

/// The one installed: on `PATH`, or Homebrew's, which a Mac app started from the Finder has not on it.
fn system_rclone() -> Option<PathBuf> {
    let homebrew = ["/opt/homebrew/bin/rclone", "/usr/local/bin/rclone"].map(PathBuf::from);
    let homebrew = homebrew.into_iter().filter(|_| cfg!(target_os = "macos"));
    crate::convert::on_path(RCLONE).or_else(|| homebrew.into_iter().find(|path| path.is_file()))
}

// shortcut: no download on first use yet; releases ship ours, source builds run vendor/rclone/build.sh.
fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "rclone is missing: run vendor/rclone/build.sh, or install rclone")
}

/// A wrong key or secret, as S3 says it.
fn refused(err: &io::Error) -> bool {
    let text = err.to_string();
    ["InvalidAccessKeyId", "SignatureDoesNotMatch"].iter().any(|code| text.contains(code))
}

/// The endpoint's store, region and URL, as rclone's S3 takes them.
fn s3_options(address: &Address) -> [(&'static str, String); 3] {
    let host = address.host.to_lowercase();
    let provider = match () {
        _ if host.ends_with(".amazonaws.com") => "AWS",
        _ if host.ends_with(".r2.cloudflarestorage.com") => "Cloudflare",
        _ if host.ends_with(".wasabisys.com") => "Wasabi",
        _ => "Other",
    };
    let labels: Vec<&str> = host.split('.').collect();
    // s3.eu-central-1.amazonaws.com, s3.us-west-004.backblazeb2.com: the region comes second.
    let region = match labels.as_slice() {
        _ if provider == "Cloudflare" => "auto",
        ["s3", region, _, _, ..] => region,
        _ => "us-east-1",
    };
    // shortcut: a store on this computer or the local network is plain HTTP, as MinIO's usually is; a setting when one has TLS.
    let local = host == "localhost"
        || host.parse::<IpAddr>().is_ok_and(|ip| match ip {
            IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            IpAddr::V6(ip) => ip.is_loopback(),
        });
    let scheme = if local { "http" } else { "https" };
    let server = if host.contains(':') { format!("[{host}]") } else { host.clone() };
    let port = address.port.map(|port| format!(":{port}")).unwrap_or_default();
    [
        ("RCLONE_S3_PROVIDER", provider.to_owned()),
        ("RCLONE_S3_REGION", region.to_owned()),
        ("RCLONE_S3_ENDPOINT", format!("{scheme}://{server}{port}")),
    ]
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;
    use crate::vfs::Remote;

    #[test]
    fn lists_remotes() {
        let config = "[work]\ntype = onedrive\ntoken = {}\n\n[photos]\nclient_id = x\ntype = drive\n";
        assert_eq!(remotes_in(config), [("work".into(), "onedrive".into()), ("photos".into(), "drive".into())]);
        assert!(remotes_in("# Encrypted rclone configuration File\n\nRCLONE_ENCRYPT_V0:\nabc\n").is_empty());
        let mounts = "work: /home/me/Work\\040Drive fuse.rclone rw 0 0\nphotos:Albums /home/me/Albums fuse.rclone rw 0 0\n";
        assert_eq!(mount_of(mounts, "work"), Some(PathBuf::from("/home/me/Work Drive")));
        assert_eq!(mount_of(mounts, "photos"), None);
    }

    #[test]
    fn finds_the_store() {
        let options = |url: &str| s3_options(&Address::parse(url).unwrap()).map(|(_, value)| value);
        let aws = options("s3://AK@s3.eu-central-1.amazonaws.com/bucket");
        assert_eq!(aws, ["AWS", "eu-central-1", "https://s3.eu-central-1.amazonaws.com"]);
        let b2 = options("s3://AK@s3.us-west-004.backblazeb2.com");
        assert_eq!(b2, ["Other", "us-west-004", "https://s3.us-west-004.backblazeb2.com"]);
        let r2 = options("s3://AK@abc.r2.cloudflarestorage.com");
        assert_eq!(r2, ["Cloudflare", "auto", "https://abc.r2.cloudflarestorage.com"]);
        let minio = options("s3://AK@192.168.1.5:9000");
        assert_eq!(minio, ["Other", "us-east-1", "http://192.168.1.5:9000"]);
    }

    /// `rclone serve s3 --auth-key AKIATESTKEY0001,SECRETKEY0000000000001` as
    /// the store, with a folder `bucket`: PC_S3_TEST=<its port>.
    #[test]
    #[ignore]
    fn talks_to_a_store() {
        let port = std::env::var("PC_S3_TEST").unwrap();
        let address = Address::parse(&format!("s3://AKIATESTKEY0001@127.0.0.1:{port}/bucket")).unwrap();
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = tries.clone();
        let ask: Ask = Arc::new(move |_| {
            let first = counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
            Reply::Secret(if first { "wrong".into() } else { "SECRETKEY0000000000001".into() })
        });
        let store = connect(&address, &ask).unwrap();
        assert_eq!(tries.load(std::sync::atomic::Ordering::Relaxed), 2);
        let dir = store.start(&address);
        let mut file = store.create(&dir.join("made.txt")).unwrap();
        file.write_all(b"made").unwrap();
        drop(file);
        let mut text = String::new();
        store.open(&dir.join("made.txt")).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "made");
        store.rename(&dir.join("made.txt"), &dir.join("moved.txt")).unwrap();
        let names: Vec<String> = store.entries(&dir).unwrap().iter().map(|entry| entry.display_name()).collect();
        assert!(names.contains(&"moved.txt".to_owned()), "{names:?}");
        store.remove(&dir.join("moved.txt"), false).unwrap();
    }
}
