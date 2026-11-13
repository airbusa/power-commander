#!/bin/sh
# Builds the Craft tools' command-line programs, which turn office documents
# into PDF for the viewer (wordcraft-cli, gridcraft-cli, deckcraft-cli), into
# vendor/craft/<os>-<arch> for this machine: each at the tag below, checked by
# its commit. Without the craft-fonts pack (the system's fonts instead), one
# processor, fully optimised and stripped: a fraction of the released CLIs.
# Needs git and Rust. `./build.sh gridcraft` builds one. Windows: in Git Bash;
# PLATFORM=windows-aarch64 cross-builds ARM64's (in its developer prompt). A
# Mac builds the other processor's too: PLATFORM=macos-x86_64 on Apple silicon
# (rustup target add x86_64-apple-darwin).
set -eu
cd "$(dirname "$0")"
HERE=$(pwd)
[ -n "${PLATFORM:-}" ] || case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PLATFORM=macos-aarch64 ;;
    Darwin-x86_64) PLATFORM=macos-x86_64 ;;
    Linux-x86_64) PLATFORM=linux-x86_64 ;;
    Linux-aarch64) PLATFORM=linux-aarch64 ;;
    MINGW*-x86_64 | MSYS*-x86_64) PLATFORM=windows-x86_64 ;;
    *) echo "No Craft tools build for $(uname -s) $(uname -m)." >&2; exit 1 ;;
esac
EXE="" TARGET="" OUTDIR=release
case $PLATFORM in
    windows-x86_64) EXE=.exe ;;
    windows-aarch64) EXE=.exe TARGET=aarch64-pc-windows-msvc OUTDIR=$TARGET/release ;;
    macos-x86_64) TARGET=x86_64-apple-darwin OUTDIR=$TARGET/release ;;
    macos-aarch64) TARGET=aarch64-apple-darwin OUTDIR=$TARGET/release ;;
    # From an x86_64 Linux too, with that target's linker (packaging/linux/Dockerfile.arm64).
    linux-aarch64) TARGET=aarch64-unknown-linux-gnu OUTDIR=$TARGET/release ;;
esac
TOOLS="wordcraft:v0.18.1:079f478bd85df7bcc567e3bafa7d7219096f9c3c
gridcraft:v0.19.1:476448d7d9f3a5f8e93fcf7d98b47fd52293fe7f
deckcraft:v0.18.1:7cc73ab1895ed2e30c8afbcada20c70b9ea0a440"
mkdir -p "$HERE/$PLATFORM"
for tool in $TOOLS; do
    name=${tool%%:*}
    rest=${tool#*:}
    tag=${rest%%:*}
    commit=${rest#*:}
    [ $# -eq 0 ] || [ "$1" = "$name" ] || continue
    src=$HERE/.build/$name
    if [ ! -d "$src" ]; then
        git clone -q --depth 1 --branch "$tag" "https://github.com/storytold/$name.git" "$src"
    fi
    [ "$(git -C "$src" rev-parse HEAD)" = "$commit" ] || { echo "$name $tag is not commit $commit" >&2; exit 1; }
    (cd "$src" && env -u CRAFT_FONTS_DIR -u CRAFT_FONTS_REQUIRED \
        CARGO_TARGET_DIR="$HERE/.build/target" \
        CARGO_PROFILE_RELEASE_LTO=fat \
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
        CARGO_PROFILE_RELEASE_STRIP=symbols \
        cargo build --release --locked ${TARGET:+--target "$TARGET"} -p "$name-cli")
    cp "$HERE/.build/target/$OUTDIR/$name-cli$EXE" "$HERE/$PLATFORM/"
    cat "$src/LICENSE-MIT" "$src/NOTICE" > "$HERE/$PLATFORM/$name.txt"
    ls -l "$HERE/$PLATFORM/$name-cli$EXE"
done
