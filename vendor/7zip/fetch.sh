#!/bin/sh
# Downloads the bundled 7-Zip into vendor/7zip/<os>-<arch>, checked against SHA256SUMS.
# Windows: fetch.bat.
set -eu
cd "$(dirname "$0")"
VERSION=26.06
TAG=2606
URL=https://github.com/ip7z/7zip/releases/download/$VERSION
DL=.download
mkdir -p $DL
if command -v sha256sum >/dev/null; then SUM=sha256sum; else SUM="shasum -a 256"; fi

# Run inside $DL.
check() { grep "  $1\$" ../SHA256SUMS | $SUM -c - >/dev/null 2>&1; }

get() {
    (cd $DL && { check "$1" || { curl -fsSL -o "$1" "$URL/$1" && check "$1"; }; }) ||
        { echo "$1: download failed or its checksum differs" >&2; exit 1; }
}

# Package, folder, the binary in it.
unix() {
    get "$1"
    rm -rf "$2" && mkdir "$2"
    tar -xJf "$DL/$1" -C "$2" "$3" License.txt
    [ "$3" = 7zz ] || mv "$2/$3" "$2/7zz"
}

unix 7z$TAG-linux-x64.tar.xz linux-x86_64 7zzs
unix 7z$TAG-linux-arm64.tar.xz linux-aarch64 7zzs
unix 7z$TAG-mac.tar.xz macos-aarch64 7zz
unix 7z$TAG-mac-x64.tar.xz macos-x86_64 7zz

# The Windows installers are 7z archives; this machine's 7-Zip unpacks them.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) HOST=linux-x86_64 ;;
    Linux-aarch64 | Linux-arm64) HOST=linux-aarch64 ;;
    Darwin-arm64) HOST=macos-aarch64 ;;
    Darwin-x86_64) HOST=macos-x86_64 ;;
    *) echo "No 7-Zip runs on $(uname -sm) to unpack the Windows installers." >&2; exit 1 ;;
esac

windows() {
    get "$1"
    rm -rf "$2" && mkdir "$2"
    "$HOST/7zz" e -y -bso0 -bsp0 -o"$2" "$DL/$1" 7z.exe 7z.dll License.txt
}

windows 7z$TAG-x64.exe windows-x86_64
windows 7z$TAG-arm64.exe windows-aarch64
rm -rf $DL
echo "7-Zip $VERSION is in $(pwd)"
