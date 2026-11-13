# The Windows release

`package.ps1` builds the release: a folder and a zip of it, in
`target/release`, for x86_64, or ARM64 with `-Arch aarch64` (needs MSVC's
ARM64 build tools and clang):

```bat
vendor\7zip\fetch.bat && vendor\pdfium\fetch.bat && vendor\mpv\fetch.bat
powershell -ExecutionPolicy Bypass -File packaging\windows\package.ps1
```

| In the folder | From |
|---|---|
| `powercommander.exe`, `pc-tui.exe` | `cargo build --release`; the C runtime is linked in |
| `7zip\` | `vendor/7zip`, with its license |
| `pdfium\` | `vendor/pdfium`, with its licenses |
| `mpv\` | `vendor/mpv`, with its README as notice |
| `rclone\` | `vendor/rclone` (`build.sh`, needs Go), with its license; without it, online storage needs an `rclone` on `PATH` |

## Installing

Unzip it anywhere and run `powercommander.exe`; nothing is installed.
