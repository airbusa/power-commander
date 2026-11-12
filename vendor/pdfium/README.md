# PDFium chromium/8125 (bundled)

The viewer renders PDF files with PDFium, the PDF engine of Chromium. Like
7-Zip it is not in git: fetch it once after cloning, and again after an
update.

- Linux, macOS: `vendor/pdfium/fetch.sh` (all platforms; needs `curl`, `tar`)
- Windows: `vendor\pdfium\fetch.bat` (Windows builds only; Windows 10 or later)

They download the prebuilt, unmodified libraries of
https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium/8125 and
check them against `SHA256SUMS` (GitHub's published digests):

| Folder | From | Library |
|---|---|---|
| `linux-x86_64` | `pdfium-linux-x64.tgz` | `libpdfium.so` |
| `linux-aarch64` | `pdfium-linux-arm64.tgz` | `libpdfium.so` |
| `macos-aarch64` | `pdfium-mac-arm64.tgz` | `libpdfium.dylib` |
| `macos-x86_64` | `pdfium-mac-x64.tgz` | `libpdfium.dylib` |
| `windows-x86_64` | `pdfium-win-x64.tgz` | `pdfium.dll` |
| `windows-aarch64` | `pdfium-win-arm64.tgz` | `pdfium.dll` |

Each folder keeps its package's `LICENSE` (MIT, the builds) and `licenses/`
(PDFium's BSD-3-Clause and those of the libraries in it); releases ship them
with the library, in `pdfium/` beside the executable
(`packaging/linux/package.sh`). The macOS app has the
library in `Contents/Frameworks`, signed with the app, and the licenses in
`Contents/Resources/licenses` (`packaging/macos/bundle.sh`); it needs macOS
13, as this library does.

The pdfium-render crate is built for this API version (`pdfium_8125`). To
update: change the version in both scripts, the crate's API feature and
`SHA256SUMS`, run a fetch script, and `cargo test -p pc-core`.
