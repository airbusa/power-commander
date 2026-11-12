#!/bin/sh
# Builds libmpv for macOS from source into vendor/mpv/macos-<arch>, for this
# Mac's processor: LGPL, for macOS 13 or later (as PDFium), with what the
# viewer plays. The sources are checked against SHA256SUMS.
# Needs Xcode's command line tools (their python3 too), ninja and pkg-config
# (Homebrew's), and on Intel nasm.
# Linux has libmpv from the system; Windows plays no video yet.
set -eu
cd "$(dirname "$0")"
[ "$(uname -s)" = Darwin ] || { echo "Only macOS builds bundle libmpv." >&2; exit 1; }
case "$(uname -m)" in
    arm64) ARCH=aarch64 TOOLS="ninja pkg-config" ;;
    x86_64) ARCH=x86_64 TOOLS="ninja pkg-config nasm" ;;
    *) echo "No macOS build for $(uname -m)." >&2; exit 1 ;;
esac
for tool in $TOOLS; do
    command -v $tool >/dev/null || { echo "$tool is missing (brew install $tool)" >&2; exit 1; }
done
# Apple's: one for this Mac's processor; another Intel one would build for Intel.
PYTHON=/usr/bin/python3
HERE=$(pwd)
OUT=$HERE/macos-$ARCH
WORK=$HERE/.build
DL=$WORK/download
SRC=$WORK/src
BUILD=$WORK/build-$ARCH
PREFIX=$WORK/prefix-$ARCH
JOBS=$(sysctl -n hw.ncpu)
export MACOSX_DEPLOYMENT_TARGET=13.0
# Only what is built here: none of Homebrew's libraries slips in.
export PKG_CONFIG_LIBDIR=$PREFIX/lib/pkgconfig
export PKG_CONFIG_PATH=

# Package as saved, URL: downloaded once, checked, unpacked into $SRC.
get() {
    check() { (cd "$DL" && grep "  $1\$" "$HERE/SHA256SUMS" | shasum -a 256 -c - >/dev/null 2>&1); }
    check "$1" || { curl -fsSL -o "$DL/$1" "$2" && check "$1"; } ||
        { echo "$1: download failed or its checksum differs" >&2; exit 1; }
    tar -xf "$DL/$1" -C "$SRC"
}

mkdir -p "$DL"
rm -rf "$SRC" "$BUILD" "$PREFIX"
mkdir -p "$SRC" "$BUILD" "$PREFIX"
# 1.11, the last for python 3.9, as macOS has it.
get meson-1.11.2.tar.gz https://github.com/mesonbuild/meson/releases/download/1.11.2/meson-1.11.2.tar.gz
get dav1d-1.5.4.tar.bz2 https://code.videolan.org/videolan/dav1d/-/archive/1.5.4/dav1d-1.5.4.tar.bz2
get ffmpeg-9.0.2.tar.xz https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz
get freetype-2.15.0.tar.xz https://downloads.sourceforge.net/project/freetype/freetype2/2.15.0/freetype-2.15.0.tar.xz
get fribidi-1.0.18.tar.xz https://github.com/fribidi/fribidi/releases/download/v1.0.18/fribidi-1.0.18.tar.xz
get harfbuzz-15.1.0.tar.xz https://github.com/harfbuzz/harfbuzz/releases/download/15.1.0/harfbuzz-15.1.0.tar.xz
get libass-0.17.5.tar.xz https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz
get libplacebo-v7.387.0.tar.bz2 https://code.videolan.org/videolan/libplacebo/-/archive/v7.387.0/libplacebo-v7.387.0.tar.bz2
get fast_float-8.3.3.tar.gz https://github.com/fastfloat/fast_float/archive/refs/tags/v8.3.3.tar.gz
get glad2-2.0.8.tar.gz https://files.pythonhosted.org/packages/6e/5a/d62b24fe1c7c2f34e15c2aa4418a5327a8550fdc272999a59e0dddebc3ee/glad2-2.0.8.tar.gz
get jinja2-3.1.6.tar.gz https://files.pythonhosted.org/packages/df/bf/f7da0350254c0ed7c72f3e33cef02e048281fec7ecec5f032d4aac52226b/jinja2-3.1.6.tar.gz
get markupsafe-3.0.4.tar.gz https://files.pythonhosted.org/packages/38/9b/e422a865e1d5d57d0e509b4e0bf1c1a70a7f6382c29a5aa428df994c8bc8/markupsafe-3.0.4.tar.gz
get Vulkan-Headers-1.4.363.0.tar.gz https://github.com/KhronosGroup/Vulkan-Headers/archive/refs/tags/vulkan-sdk-1.4.363.0.tar.gz
get mpv-0.42.0.tar.gz https://github.com/mpv-player/mpv/archive/refs/tags/v0.42.0.tar.gz

# Source folder, then its options.
meson() { "$PYTHON" "$SRC/meson-1.11.2/meson.py" "$@"; }
build() {
    name=$(basename "$1")
    shift
    meson setup "$BUILD/$name" "$SRC/$name" --prefix="$PREFIX" --libdir=lib \
        --buildtype=release --default-library=shared -Db_ndebug=true "$@"
    ninja -C "$BUILD/$name" install
}

# Features are off unless asked for: nothing more is built than is needed.
build "$SRC/dav1d-1.5.4" -Denable_tools=false -Denable_examples=false -Denable_tests=false -Denable_docs=false

# FFmpeg as LGPL (no --enable-gpl): every decoder and demuxer, no encoders or devices.
mkdir "$BUILD/ffmpeg"
(cd "$BUILD/ffmpeg" &&
    "$SRC/ffmpeg-9.0.2/configure" --prefix="$PREFIX" --install-name-dir=@rpath \
        --enable-shared --disable-static --disable-programs --disable-doc --disable-debug \
        --disable-autodetect --enable-audiotoolbox --enable-videotoolbox --enable-zlib \
        --enable-bzlib --enable-iconv --enable-libdav1d \
        --disable-avdevice --disable-encoders --disable-muxers \
        --extra-cflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET" \
        --extra-ldflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET" --extra-libs=-liconv &&
    make -j"$JOBS" && make install)

build "$SRC/freetype-2.15.0" --auto-features=disabled -Dharfbuzz=disabled -Dzlib=none
build "$SRC/fribidi-1.0.18" -Ddocs=false -Dbin=false -Dtests=false
build "$SRC/harfbuzz-15.1.0" --auto-features=disabled -Dfreetype=enabled
build "$SRC/libass-0.17.5" --auto-features=disabled -Dcoretext=enabled

# libplacebo's generators and headers, which its archive leaves out (its
# Vulkan stubs want Vulkan's headers, Vulkan off as it is).
PLACEBO=$SRC/libplacebo-v7.387.0
for part in jinja:jinja2-3.1.6 markupsafe:markupsafe-3.0.4 glad:glad2-2.0.8 fast_float:fast_float-8.3.3 \
    Vulkan-Headers:Vulkan-Headers-vulkan-sdk-1.4.363.0; do
    rm -rf "$PLACEBO/3rdparty/${part%%:*}"
    mv "$SRC/${part#*:}" "$PLACEBO/3rdparty/${part%%:*}"
done
build "$PLACEBO" --auto-features=disabled -Dopengl=enabled -Ddemos=false -Dtests=false

# mpv 0.42 builds the function its Core Audio output needs only with Cocoa.
patch -s -d "$SRC/mpv-0.42.0" -p1 < "$HERE/mpv-coreaudio-without-cocoa.patch"
# mpv as a library, LGPL: OpenGL in the context pc-core makes, Core Audio.
# No Cocoa, whose code wants mpv's Swift: VideoToolbox's frames are copied
# out, not handed to OpenGL, as pc-core reads each one back anyway.
build "$SRC/mpv-0.42.0" --auto-features=disabled -Dlibmpv=true -Dcplayer=false -Dgpl=false \
    -Dcoreaudio=enabled -Dgl=enabled -Dplain-gl=enabled -Diconv=enabled -Dzlib=enabled

# libmpv and the libraries of ours it loads, by the names it loads them with.
rm -rf "$OUT"
mkdir "$OUT"
collect() {
    name=$(basename "$1")
    [ -e "$OUT/$name" ] && return
    cp -L "$1" "$OUT/$name"
    chmod u+w "$OUT/$name"
    for dep in $(otool -L "$1" | tail -n +2 | awk '{print $1}' | grep -E "^(@rpath/|$PREFIX/)"); do
        collect "$PREFIX/lib/$(basename "$dep")"
    done
}
collect "$PREFIX/lib/libmpv.2.dylib"
# Each finds the others beside libmpv; no symbols but the exported ones.
for lib in "$OUT"/*.dylib; do
    install_name_tool -id "@rpath/$(basename "$lib")" "$lib" 2>/dev/null
    for dep in $(otool -L "$lib" | tail -n +2 | awk '{print $1}' | grep "^$PREFIX/"); do
        install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$lib" 2>/dev/null
    done
    for path in $(otool -l "$lib" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }'); do
        install_name_tool -delete_rpath "$path" "$lib" 2>/dev/null
    done
    strip -x "$lib"
done
install_name_tool -add_rpath @loader_path "$OUT/libmpv.2.dylib" 2>/dev/null
# Changed, they are signed again, ad hoc; releases sign them with the app.
codesign --force --sign - "$OUT"/*.dylib 2>/dev/null
# mpv links with -undefined dynamic_lookup: a function nothing has would only
# fail once called. Loaded binding every symbol, it fails here.
"$PYTHON" -I -c 'import ctypes, sys; ctypes.CDLL(sys.argv[1])' "$OUT/libmpv.2.dylib"
echo "libmpv is in $OUT"
