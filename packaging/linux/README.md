# The Linux release

`package.sh` builds the release for the computer's processor (x86_64 or
aarch64): a folder and a tar.gz of it, in `target/release`:

```sh
vendor/7zip/fetch.sh && vendor/pdfium/fetch.sh && vendor/rclone/build.sh linux-$(uname -m)
packaging/linux/package.sh
```

| In the folder | From |
|---|---|
| `powercommander`, `pc-tui` | `cargo build --release`, or `BIN=<folder>` with them built elsewhere (another distribution's glibc, in its container) |
| `pc-qtstyle` | `crates/pc-qtstyle/build.sh` (needs Qt 6's development files): on KDE the user's Qt style draws the controls; without it, the built-in Breeze look |
| `pc-gtkstyle` | `crates/pc-gtkstyle/build.sh` (needs GTK 3's development files): on Xfce, MATE and Cinnamon the GTK 3 theme draws the controls |
| `7zip/` | `vendor/7zip`, with its license |
| `pdfium/` | `vendor/pdfium`, with its licenses |
| `rclone/` | `vendor/rclone` (built from source; needs Go), with its license |
| `share/applications`, `share/icons` | the menu entry and icons, beside this file |

Video and sound play through libmpv: the add-on (Settings > Add-ons, our
build: `vendor/mpv/build.sh`), else the system's (`libmpv2` on Debian and
Ubuntu, `mpv-libs` on Fedora); without either they do not play. Office
documents need the Craft tools, the add-on or the system's.

## Installing

Also over a running copy, which keeps running until closed:

```sh
sudo cp -rT --remove-destination powercommander-*-linux-*/ /opt/powercommander
sudo ln -sf /opt/powercommander/powercommander /opt/powercommander/pc-tui /usr/local/bin/
sudo cp -r /opt/powercommander/share/* /usr/local/share/
```
