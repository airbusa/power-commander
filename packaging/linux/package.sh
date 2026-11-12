#!/bin/sh
# Builds the Linux release for this computer's processor: a folder with the
# programs, 7-Zip, PDFium and rclone from vendor/ (fetched or built first),
# the menu entry and icons, and a tar.gz of it, both in target/release.
# BIN: a folder with powercommander and pc-tui already built (for another
# distribution, in its container); unset, they are built here.
set -eu
cd "$(dirname "$0")/../.."
ARCH=$(uname -m)
case "$ARCH" in
    x86_64 | aarch64) ;;
    *) echo "No Linux build for $ARCH." >&2; exit 1 ;;
esac
PLATFORM=linux-$ARCH
for vendored in 7zip/$PLATFORM/7zz:fetch.sh pdfium/$PLATFORM/libpdfium.so:fetch.sh rclone/$PLATFORM/rclone:build.sh; do
    file=${vendored%:*}
    [ -e "vendor/$file" ] || { echo "vendor/$file is missing: run vendor/${file%%/*}/${vendored#*:}" >&2; exit 1; }
done
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

if [ -z "${BIN:-}" ]; then
    cargo build --release -p pc-gui -p pc-tui
    BIN=${CARGO_TARGET_DIR:-target}/release
fi

# Each tool's folder, its license inside, beside the programs: where pc-core's bundled.rs looks.
DIR=target/release/powercommander-$VERSION-linux-$ARCH
rm -rf "$DIR"
mkdir -p "$DIR/share/applications" "$DIR/share/icons"
cp "$BIN/powercommander" "$BIN/pc-tui" "$DIR/"
for tool in 7zip pdfium rclone; do
    cp -R "vendor/$tool/$PLATFORM" "$DIR/$tool"
done
cp LICENSE "$DIR/"
cp packaging/linux/powercommander.desktop "$DIR/share/applications/"
cp -R packaging/linux/hicolor "$DIR/share/icons/"

tar -czf "$DIR.tar.gz" -C target/release "$(basename "$DIR")"
echo "$DIR"
echo "$DIR.tar.gz"
