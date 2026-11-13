#!/bin/sh
# Makes this platform's add-ons from vendor/ (built first; one not built is
# skipped): a zip each in target/addons, signed on macOS as the app is
# (SIGN_IDENTITY, else ad hoc), and target/addons/addons.json, the release's
# list with these in it (other platforms' entries kept). `--upload` puts them
# on the GitHub release `addons`, which Settings > Add-ons reads. Windows: in
# Git Bash; PLATFORM=windows-aarch64 for ARM64's.
set -eu
cd "$(dirname "$0")/.."
[ -n "${PLATFORM:-}" ] || case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PLATFORM=macos-aarch64 ;;
    Darwin-x86_64) PLATFORM=macos-x86_64 ;;
    Linux-x86_64) PLATFORM=linux-x86_64 ;;
    Linux-aarch64) PLATFORM=linux-aarch64 ;;
    MINGW*-x86_64 | MSYS*-x86_64) PLATFORM=windows-x86_64 ;;
    *) echo "No add-ons for $(uname -s) $(uname -m)." >&2; exit 1 ;;
esac
EXE=""
PY=python3
case $PLATFORM in windows-*) EXE=.exe PY=python ;; esac
# The bundled 7-Zip zips where zip is missing (Git Bash).
SEVENZIP=$(pwd)/vendor/7zip/windows-x86_64/7z.exe
REPO=airbusa/power-commander
TAG=addons
URL=https://github.com/$REPO/releases/download/$TAG
OUT=target/addons
mkdir -p "$OUT"
MANIFEST=$OUT/addons.json
# The published list as it is now, to add to: never an older copy, which
# would drop what another machine added since. Empty only while there is none.
rm -f "$MANIFEST"
if ! gh release download "$TAG" -R "$REPO" -p addons.json -O "$MANIFEST" 2>/dev/null &&
    ! curl -fsSL "$URL/addons.json" -o "$MANIFEST" 2>/dev/null; then
    if curl -fsSI "https://github.com/$REPO/releases/tag/$TAG" >/dev/null 2>&1; then
        echo "The release has an add-on list that could not be read: try again." >&2
        exit 1
    fi
    echo '{"addons": []}' > "$MANIFEST"
fi

# The versions the build scripts pin.
MPV=$(sed -n 's/.*mpv-\([0-9.]*\)\.tar\.gz.*/\1/p' vendor/mpv/build.sh | head -n 1)
RCLONE=$(sed -n 's/^TAG=v//p' vendor/rclone/build.sh)
CRAFT=$(grep -oE '(wordcraft|gridcraft|deckcraft):v[0-9.]+' vendor/craft/build.sh | sed 's/:v/ /; s/^w/W/; s/^g/G/; s/^d/D/; s/craft/Craft/' | paste -sd ',' - | sed 's/,/, /g')

SIGN=${SIGN_IDENTITY:--}
MADE=""
# make <id> <version> <dir> <files...>
make() {
    id=$1 version=$2 dir=$3
    shift 3
    for file in "$@"; do
        [ -e "$dir/$file" ] || { echo "$id: $dir/$file is missing, skipped (build it first)" >&2; return 0; }
    done
    stage=$OUT/.$id
    rm -rf "$stage"
    mkdir -p "$stage"
    for file in "$@"; do cp "$dir/$file" "$stage/"; done
    if [ "$(uname -s)" = Darwin ]; then
        set -- --force --sign "$SIGN"
        [ "$SIGN" = - ] || set -- "$@" --options runtime --timestamp
        for file in "$stage"/*; do
            file -b "$file" | grep -q Mach-O && codesign "$@" "$file"
        done
    fi
    slug=$(echo "$version" | tr -cs '0-9.' '-' | sed 's/^-//; s/-$//')
    zip=$id-$slug-$PLATFORM.zip
    rm -f "$OUT/$zip"
    if command -v zip >/dev/null; then
        (cd "$stage" && zip -q -X -r "../$zip" .)
    else
        (cd "$stage" && "$SEVENZIP" a -tzip -bso0 -bsp0 "../$zip" .)
    fi
    rm -rf "$stage"
    "$PY" -I - "$MANIFEST" "$id" "$version" "$PLATFORM" "$OUT/$zip" "$URL/$zip" <<'PY'
import hashlib, json, os, sys
manifest, id, version, platform, path, url = sys.argv[1:]
data = json.load(open(manifest))
entry = next((a for a in data["addons"] if a["id"] == id), None)
if entry is None:
    entry = {"id": id, "version": version, "downloads": {}}
    data["addons"].append(entry)
if entry["version"] != version:
    entry["version"], entry["downloads"] = version, {}
body = open(path, "rb").read()
entry["downloads"][platform] = {"url": url, "sha256": hashlib.sha256(body).hexdigest(), "size": len(body)}
json.dump(data, open(manifest, "w"), indent=2)
PY
    MADE="$MADE $OUT/$zip"
    echo "$OUT/$zip ($version)"
}

case $PLATFORM in
    macos-*)
        dylibs=$(cd "vendor/mpv/$PLATFORM" 2>/dev/null && ls *.dylib 2>/dev/null || true)
        [ -n "$dylibs" ] && cp vendor/mpv/README.md "vendor/mpv/$PLATFORM/libmpv.md"
        # shellcheck disable=SC2086
        make mpv "$MPV" "vendor/mpv/$PLATFORM" libmpv.2.dylib $dylibs libmpv.md ;;
    linux-*)
        libs=$(cd "vendor/mpv/$PLATFORM" 2>/dev/null && ls *.so.* 2>/dev/null || true)
        [ -n "$libs" ] && cp vendor/mpv/README.md "vendor/mpv/$PLATFORM/libmpv.md"
        # shellcheck disable=SC2086
        make mpv "$MPV" "vendor/mpv/$PLATFORM" libmpv.so.2 $libs libmpv.md ;;
    windows-*)
        # shortcut: shinchiro's build is 0.42.0 and 38 commits, listed as 0.42.0 so the other platforms' stay.
        [ -e "vendor/mpv/$PLATFORM/libmpv-2.dll" ] && cp vendor/mpv/README.md "vendor/mpv/$PLATFORM/libmpv.md"
        make mpv "$MPV" "vendor/mpv/$PLATFORM" libmpv-2.dll libmpv.md ;;
esac
make rclone "$RCLONE" "vendor/rclone/$PLATFORM" rclone$EXE COPYING
make craft "$CRAFT" "vendor/craft/$PLATFORM" wordcraft-cli$EXE gridcraft-cli$EXE deckcraft-cli$EXE wordcraft.txt gridcraft.txt deckcraft.txt

if [ "${1:-}" = --upload ] && [ -n "$MADE" ]; then
    gh release view "$TAG" -R "$REPO" >/dev/null 2>&1 ||
        gh release create "$TAG" -R "$REPO" --title "Add-ons" --latest=false \
            --notes "Power Commander's add-ons, downloaded by Settings > Add-ons: libmpv (video and sound), rclone (online storage), the Craft tools (office documents). addons.json lists them."
    # shellcheck disable=SC2086
    gh release upload "$TAG" -R "$REPO" --clobber $MADE "$MANIFEST"
fi
echo "$MANIFEST"
