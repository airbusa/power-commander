# 7-Zip 26.06 (bundled)

Power Commander uses only this copy of 7-Zip, never a system one. It is not
in git: fetch it once after cloning, and again after an update.

- Linux, macOS: `vendor/7zip/fetch.sh` (all platforms; needs `curl`, `tar`)
- Windows: `vendor\7zip\fetch.bat` (Windows builds only; Windows 10 or later)

They download the official, unmodified builds from
https://github.com/ip7z/7zip/releases/tag/26.06 and check them against
`SHA256SUMS` (GitHub's published digests):

| Folder | From | Files |
|---|---|---|
| `linux-x86_64` | `7z2606-linux-x64.tar.xz` | `7zzs` (static) as `7zz` |
| `linux-aarch64` | `7z2606-linux-arm64.tar.xz` | `7zzs` (static) as `7zz` |
| `macos-aarch64` | `7z2606-mac.tar.xz` | `7zz` |
| `macos-x86_64` | `7z2606-mac-x64.tar.xz` | `7zz` |
| `windows-x86_64` | `7z2606-x64.exe` | `7z.exe`, `7z.dll` |
| `windows-aarch64` | `7z2606-arm64.exe` | `7z.exe`, `7z.dll` |

Each folder gets its package's `License.txt` (LGPL 2.1+, BSD, unRAR
restriction).

Releases copy the platform's folder to `7zip/` beside the executable
(`packaging/linux/package.sh`), with
its `License.txt`, and attach `7z2606-src.tar.xz` (also in `SHA256SUMS`),
the source the LGPL asks us to offer. The macOS app has `7zz` beside the
executable, in `Contents/MacOS`, signed with the app, and the license in
`Contents/Resources/licenses` (`packaging/macos/bundle.sh`).

To update: change the version in both scripts and the digests in
`SHA256SUMS`, run a fetch script, and `cargo test -p pc-core`.
