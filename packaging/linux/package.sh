#!/bin/sh
# Builds the Linux release for this computer's processor: a folder with the
# programs, 7-Zip, PDFium and rclone from vendor/ (fetched or built first),
# the menu entry and icons, and a tar.gz of it, both in target/release.
# BIN: a folder with powercommander and pc-tui (and the style helpers) already built
# (for another distribution, in its container); unset, they are built here.
set -eu
cd "$(dirname "$0")/../.."
# ARCH: another processor's, for programs built for it elsewhere (BIN).
ARCH=${ARCH:-$(uname -m)}
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
    [ "$ARCH" = "$(uname -m)" ] || { echo "BIN: a folder with the programs built for $ARCH" >&2; exit 1; }
    cargo build --release -p pc-gui -p pc-tui
    BIN=${CARGO_TARGET_DIR:-target}/release
fi

# Each tool's folder, its license inside, beside the programs: where pc-core's bundled.rs looks.
DIR=target/release/powercommander-$VERSION-linux-$ARCH
rm -rf "$DIR"
mkdir -p "$DIR/share/applications" "$DIR/share/icons"
cp "$BIN/powercommander" "$BIN/pc-tui" "$DIR/"
# The desktop's look through its toolkit, KDE's Qt style and the GTK 3 theme
# of Xfce, MATE and Cinnamon: built with the oldest Qt 6 and GTK 3 the release is for.
for helper in pc-qtstyle:Qt6Widgets pc-gtkstyle:gtk+-3.0; do
    name=${helper%%:*}
    if [ -e "$BIN/$name" ]; then
        cp "$BIN/$name" "$DIR/"
    elif [ "$ARCH" = "$(uname -m)" ] && pkg-config --exists "${helper#*:}"; then
        "crates/$name/build.sh" "$DIR" >/dev/null
    else
        echo "No $name (no ${helper#*:} development files): the app draws those controls itself." >&2
    fi
done
for tool in 7zip pdfium rclone; do
    cp -R "vendor/$tool/$PLATFORM" "$DIR/$tool"
done
cp LICENSE "$DIR/"
cp packaging/linux/powercommander.desktop "$DIR/share/applications/"
cp -R packaging/linux/hicolor "$DIR/share/icons/"

tar -czf "$DIR.tar.gz" -C target/release "$(basename "$DIR")"
echo "$DIR"
echo "$DIR.tar.gz"
