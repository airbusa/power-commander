# The Windows release

`package.ps1` builds the release: a folder and a zip of it, in
`target/release`, for x86_64, or ARM64 with `-Arch aarch64` (needs MSVC's
ARM64 build tools and clang):

```bat
vendor\7zip\fetch.bat && vendor\pdfium\fetch.bat
powershell -ExecutionPolicy Bypass -File packaging\windows\package.ps1
```

ARM64 on an x64 PC: Visual Studio's "MSVC ARM64 build tools" and LLVM
(`winget install LLVM.LLVM`); clang finds the C headers only in a developer
prompt for ARM64:

```bat
"%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64_arm64
set "PATH=%ProgramFiles%\LLVM\bin;%PATH%"
powershell -ExecutionPolicy Bypass -File packaging\windows\package.ps1 -Arch aarch64
```

| In the folder | From |
|---|---|
| `powercommander.exe`, `pc-tui.exe` | `cargo build --release`; the C runtime is linked in |
| `7zip\` | `vendor/7zip`, with its license |
| `pdfium\` | `vendor/pdfium`, with its licenses |

libmpv (video and sound), rclone (online storage) and the Craft tools (office
documents) are add-ons: the app works without them, and Settings > Add-ons
downloads them (`packaging/addons.sh`, in Git Bash, makes them).

## Installing

Unzip it anywhere and run `powercommander.exe`; nothing is installed.
