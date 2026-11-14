# The Linux release

`package.sh` builds the release for the computer's processor (x86_64 or
aarch64): a folder and a tar.gz of it, in `target/release`:

```sh
vendor/7zip/fetch.sh && vendor/pdfium/fetch.sh && vendor/rclone/build.sh linux-$(uname -m)
packaging/linux/package.sh
```

| In the folder | From |
|---|---|
| `sparkcommander` (the app), `scmd` (in the terminal) | `cargo build --release`, or `BIN=<folder>` with them built elsewhere (another distribution's glibc, in its container) |
| `sc-qtstyle` | `crates/sc-qtstyle/build.sh` (needs Qt 6's development files): on KDE the user's Qt style draws the controls; without it, the built-in Breeze look |
| `sc-gtkstyle` | `crates/sc-gtkstyle/build.sh` (needs GTK 3's development files): on Xfce, MATE and Cinnamon the GTK 3 theme draws the controls |
| `7zip/` | `vendor/7zip`, with its license |
| `pdfium/` | `vendor/pdfium`, with its licenses |
| `rclone/` | `vendor/rclone` (built from source; needs Go), with its license |
| `share/applications`, `share/icons` | the menu entry and icons, beside this file |

Video and sound play through libmpv: the add-on (Settings > Add-ons, our
build: `vendor/mpv/build.sh`), else the system's (`libmpv2` on Debian and
Ubuntu, `mpv-libs` on Fedora); without either they do not play. Office
documents need the Craft tools, the add-on or the system's.

## Building in a container

`Dockerfile` is the release's build machine: Ubuntu 20.04, the oldest the
release is for (glibc 2.31 and later run what it builds: Debian 11, Ubuntu
20.04, RHEL 9 and every distribution since), with Rust and what the app,
libmpv and `sc-gtkstyle` build with. Built there, the programs run on newer
distributions too. Check a build with `objdump -T <file> | grep -o
'GLIBC_[0-9.]*' | sort -V | tail -1` (2.31 at most). `sc-qtstyle` needs Qt
6, which 20.04 lacks: `Dockerfile.qt` (24.04) builds it, for Plasma 6, whose
distributions all have glibc 2.39 or later:

```sh
docker build -t sparkcommander-build packaging/linux
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/src/target/cargo -v "$PWD:/src" -w /src \
    sparkcommander-build cargo build --release -p sc-gui -p sc-tui
```

## 64-bit ARM (aarch64)

Cross-built on x86_64: `Dockerfile.arm64` adds Ubuntu's arm64 libraries and
the aarch64 compilers to the image above.

```sh
docker build -t sparkcommander-build-arm64 -f packaging/linux/Dockerfile.arm64 packaging/linux
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/src/target/cargo -v "$PWD:/src" -w /src \
    sparkcommander-build-arm64 cargo build --release --target aarch64-unknown-linux-gnu -p sc-gui -p sc-tui
vendor/rclone/build.sh linux-aarch64
ARCH=aarch64 BIN=target/aarch64-unknown-linux-gnu/release packaging/linux/package.sh
```

Its add-ons build in the same image with `PLATFORM=linux-aarch64`
(`vendor/craft/build.sh`, `vendor/mpv/build.sh`), and `PLATFORM=linux-aarch64
packaging/addons.sh` zips them. `sc-qtstyle` and `sc-gtkstyle` are not
cross-built (Qt's and GTK's arm64 development files need arm64's Python):
built on an ARM machine with their `build.sh`, into `BIN`; without them the
app draws those controls itself.

## Installing

Also over a running copy, which keeps running until closed:

```sh
sudo cp -rT --remove-destination spark-commander-*-linux-*/ /opt/spark-commander
sudo ln -sf /opt/spark-commander/sparkcommander /opt/spark-commander/scmd /usr/local/bin/
sudo cp -r /opt/spark-commander/share/* /usr/local/share/
```
