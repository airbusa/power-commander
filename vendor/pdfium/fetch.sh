#!/bin/sh
# Downloads the bundled PDFium into vendor/pdfium/<os>-<arch>, checked against SHA256SUMS.
# Windows: fetch.bat.
set -eu
cd "$(dirname "$0")"
VERSION=8125
URL=https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/$VERSION
DL=.download
mkdir -p $DL
if command -v sha256sum >/dev/null; then SUM=sha256sum; else SUM="shasum -a 256"; fi

# Run inside $DL.
check() { grep "  $1\$" ../SHA256SUMS | $SUM -c - >/dev/null 2>&1; }

# Package, folder, the library in it.
get() {
    (cd $DL && { check "$1" || { curl -fsSL -o "$1" "$URL/$1" && check "$1"; }; }) ||
        { echo "$1: download failed or its checksum differs" >&2; exit 1; }
    rm -rf "$2" && mkdir "$2"
    tar -xzf "$DL/$1" -C "$2" "$3" LICENSE licenses
    mv "$2/$3" "$2/"
    rmdir "$2/$(dirname "$3")"
}

get pdfium-linux-x64.tgz linux-x86_64 lib/libpdfium.so
get pdfium-linux-arm64.tgz linux-aarch64 lib/libpdfium.so
get pdfium-mac-arm64.tgz macos-aarch64 lib/libpdfium.dylib
get pdfium-mac-x64.tgz macos-x86_64 lib/libpdfium.dylib
get pdfium-win-x64.tgz windows-x86_64 bin/pdfium.dll
get pdfium-win-arm64.tgz windows-aarch64 bin/pdfium.dll
rm -rf $DL
echo "PDFium $VERSION is in $(pwd)"
