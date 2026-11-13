# libmpv 0.42.0 (built for macOS), and for Windows

The viewer plays video and sound with libmpv. Linux has it from the system
(`libmpv.so.2`, in every distribution); macOS and Windows have none, so Power
Commander ships its own there and uses only that one. Like PDFium it is not
in git: get it once after cloning, and again after an update.

- macOS: `vendor/mpv/build.sh` (needs Xcode's command line tools, and
  Homebrew's `ninja` and `pkg-config`, on Intel `nasm` too; it can take most
  of an hour)
- Windows: `vendor\mpv\fetch.bat` (Windows 10 or later)

## Windows

`fetch.bat` downloads shinchiro's prebuilt libmpv
(https://github.com/shinchiro/mpv-winbuild-cmake/releases/tag/20261111, the
builds mpv's site links to), `mpv-dev-x86_64-…` and `mpv-dev-aarch64-…`,
checks them against `SHA256SUMS` (GitHub's digests) and puts
`libmpv-2.dll` into `windows-x86_64` and `windows-aarch64`: one library
with FFmpeg and the rest in it, 120 MB. It is a GPL build, as the app is
GPL; the build scripts and the sources' versions are in that repository.
It unpacks them with 7-Zip's `7zr.exe`, downloaded beside them.
Releases have it in `mpv/` beside the executable.

mpv draws with OpenGL (WGL); where it is too old for mpv, as Windows' own
1.1 in Remote Desktop, on the CPU (mpv's software renderer), slower.

To update: change `BUILD` and `URL` in `fetch.bat` and the two digests in
`SHA256SUMS`, run it, and `cargo test -p pc-core media -- --ignored`.

## macOS

It downloads the sources below, checks them against `SHA256SUMS` (the
digests Homebrew's formulas and GitHub publish), builds them for this Mac's
processor and macOS 13 or later (as PDFium), and puts `libmpv.2.dylib` and
the 12 libraries it loads into `macos-aarch64` (Apple silicon) or
`macos-x86_64` (Intel), 24 MB. They find each other beside `libmpv`
(`@loader_path`); they are stripped and signed ad hoc.

What is built:

- FFmpeg as LGPL (no `--enable-gpl`): every decoder, demuxer and filter, no
  encoders, muxers or devices; VideoToolbox decodes in hardware.
- mpv as a library only, LGPL (`-Dgpl=false`): OpenGL in the context
  pc-core makes, Core Audio. No Cocoa: in 0.42 its code needs mpv's Swift
  part; so VideoToolbox's frames are copied out rather than handed to
  OpenGL, as pc-core reads each frame back anyway.
- Everything else with all optional features off, but FreeType in HarfBuzz,
  Core Text in libass and OpenGL in libplacebo.

The app (`packaging/macos/bundle.sh`) copies the libraries to
`Contents/Frameworks` and signs them with itself; this file goes with them
as their notice.

## What is in it

Releases offer these sources, as the LGPL asks.

| Library | Version | License | Source |
|---|---|---|---|
| mpv | 0.42.0 | LGPL 2.1 or later | https://github.com/mpv-player/mpv/archive/refs/tags/v0.42.0.tar.gz |
| FFmpeg (libavcodec, libavfilter, libavformat, libavutil, libswresample, libswscale) | 9.0.2 | LGPL 2.1 or later | https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz |
| libplacebo | 7.387.0 | LGPL 2.1 or later | https://code.videolan.org/videolan/libplacebo/-/archive/v7.387.0/libplacebo-v7.387.0.tar.bz2 |
| libass | 0.17.5 | ISC | https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz |
| FreeType | 2.15.0 | FreeType License or GPL 2 | https://downloads.sourceforge.net/project/freetype/freetype2/2.15.0/freetype-2.15.0.tar.xz |
| FriBidi | 1.0.18 | LGPL 2.1 or later | https://github.com/fribidi/fribidi/releases/download/v1.0.18/fribidi-1.0.18.tar.xz |
| HarfBuzz | 15.1.0 | MIT | https://github.com/harfbuzz/harfbuzz/releases/download/15.1.0/harfbuzz-15.1.0.tar.xz |
| dav1d | 1.5.4 | BSD 2-Clause | https://code.videolan.org/videolan/dav1d/-/archive/1.5.4/dav1d-1.5.4.tar.bz2 |

Built into libplacebo, which its archive leaves out: fast_float 8.3.3 (MIT,
Apache 2.0 or Boost), Vulkan-Headers 1.4.363.0 (Apache 2.0; only its stubs
use them, Vulkan is off), and the OpenGL loader that glad 2.0.8 (MIT)
generates.
Only for building: Meson 1.11.2 (the last for the Python 3.9 macOS has),
Jinja2 3.1.6 and MarkupSafe 3.0.4.

To update: change the versions in `build.sh` and their digests in
`SHA256SUMS` (Homebrew's formulas have both), update the table, run
`build.sh`, and `cargo test -p pc-core media -- --ignored`.
