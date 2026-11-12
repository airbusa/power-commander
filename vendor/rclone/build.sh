#!/bin/sh
# Builds rclone with only the services Power Commander offers (main.go) into
# vendor/rclone/<os>-<arch>, for every platform: rclone's source at the tag
# below, checked by its commit. Needs git and Go; Go cross-builds, so one
# machine builds them all. `./build.sh linux-x86_64` builds one.
set -eu
cd "$(dirname "$0")"
TAG=v1.75.2
COMMIT=be8d6044eacb843b74d335db0c5fbe09222031fe
HERE=$(pwd)
SRC=$HERE/.build/rclone
if [ ! -d "$SRC" ]; then
    git clone -q --depth 1 --branch "$TAG" https://github.com/rclone/rclone.git "$SRC"
fi
[ "$(git -C "$SRC" rev-parse HEAD)" = "$COMMIT" ] || { echo "rclone $TAG is not commit $COMMIT" >&2; exit 1; }
mkdir -p "$SRC/cmd/powercommander"
cp main.go "$SRC/cmd/powercommander/main.go"
TARGETS=${1:-"linux-x86_64 linux-aarch64 macos-x86_64 macos-aarch64 windows-x86_64 windows-aarch64"}
for target in $TARGETS; do
    os=${target%-*}
    arch=${target#*-}
    case $os in macos) goos=darwin ;; *) goos=$os ;; esac
    case $arch in x86_64) goarch=amd64 ;; aarch64) goarch=arm64 ;; esac
    exe=rclone
    [ "$os" = windows ] && exe=rclone.exe
    mkdir -p "$HERE/$target"
    (cd "$SRC" && CGO_ENABLED=0 GOOS=$goos GOARCH=$goarch go build -trimpath -ldflags "-s -w" \
        -o "$HERE/$target/$exe" ./cmd/powercommander)
    cp "$SRC/COPYING" "$HERE/$target/COPYING"
    echo "$target/$exe"
done
