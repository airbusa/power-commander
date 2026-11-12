//! Online storage through rclone (vendor/rclone): each connection is `rclone
//! serve sftp --stdio`, which our SFTP client speaks. S3 so far: Amazon's and
//! the stores speaking it (Backblaze B2, Cloudflare R2, Wasabi, MinIO…).

use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::remote::{self, Address, Ask, Prompt, Reply};
use crate::sftp::Sftp;
use crate::vfs::Fs;

const TRIES: usize = 3;

/// Connects to the store, asking for the secret key until the store takes it. Blocks.
pub fn connect(address: &Address, ask: &Ask) -> io::Result<Arc<Sftp>> {
    let program = rclone()?;
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
        let mut command = rclone_command(&program);
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

/// rclone with no config file of the user's, asking nothing on its pipe.
fn rclone_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    // A short directory cache: a refresh shows what changed elsewhere.
    command.args(["--config", "", "--ask-password=false", "--dir-cache-time", "5s", "--poll-interval", "0"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Ours (vendor/rclone, or `rclone/` beside the app), else the system's.
fn rclone() -> io::Result<PathBuf> {
    let name = if cfg!(windows) { "rclone.exe" } else { "rclone" };
    let ours = crate::bundled::path("rclone", name);
    match ours.is_file() {
        true => Ok(ours),
        // shortcut: no download on first use yet; releases ship it, source builds run vendor/rclone/build.sh.
        false => crate::convert::on_path(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "rclone is missing: run vendor/rclone/build.sh, or install rclone")),
    }
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
