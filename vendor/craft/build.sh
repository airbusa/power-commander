#!/bin/sh
# Builds the Craft tools' command-line programs, which turn office documents
# into PDF for the viewer (wordcraft-cli, gridcraft-cli, deckcraft-cli), into
# vendor/craft/<os>-<arch> for this machine: each at the tag below, checked by
# its commit. Without the craft-fonts pack (the system's fonts instead), one
# processor, fully optimised and stripped: a fraction of the released CLIs.
# Needs git and Rust. `./build.sh gridcraft` builds one.
set -eu
cd "$(dirname "$0")"
HERE=$(pwd)
case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PLATFORM=macos-aarch64 ;;
    Darwin-x86_64) PLATFORM=macos-x86_64 ;;
    Linux-x86_64) PLATFORM=linux-x86_64 ;;
    Linux-aarch64) PLATFORM=linux-aarch64 ;;
    *) echo "No Craft tools build for $(uname -s) $(uname -m)." >&2; exit 1 ;;
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
        cargo build --release --locked -p "$name-cli")
    cp "$HERE/.build/target/release/$name-cli" "$HERE/$PLATFORM/"
    cat "$src/LICENSE-MIT" "$src/NOTICE" > "$HERE/$PLATFORM/$name.txt"
    ls -l "$HERE/$PLATFORM/$name-cli"
done
