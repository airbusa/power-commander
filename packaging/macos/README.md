# The macOS app

`bundle.sh` builds `Power Commander.app` for Apple silicon or Intel, and a
zip of it for a GitHub release, both in `target/<arch>-apple-darwin/release`:
two downloads, `PowerCommander-<version>-macos-aarch64.zip` and
`…-macos-x86_64.zip`. An Apple silicon Mac builds both (Intel's needs
`rustup target add x86_64-apple-darwin`):

```sh
vendor/7zip/fetch.sh && vendor/pdfium/fetch.sh
packaging/macos/bundle.sh aarch64
packaging/macos/bundle.sh x86_64
```

| In the app | From |
|---|---|
| `Contents/MacOS/powercommander` | `cargo build --release -p pc-gui --target <arch>-apple-darwin` |
| `Contents/MacOS/7zz` | `vendor/7zip` |
| `Contents/Frameworks/libpdfium.dylib` | `vendor/pdfium` |
| `Contents/Resources/PowerCommander.icns` | the app's icon, beside this file |
| `Contents/Resources/licenses` | the licenses of all of them |

It needs macOS 13 or later (PDFium's).

libmpv (video and sound), rclone (online storage) and the Craft tools (office
documents) are add-ons: the app works without them, and Settings > Add-ons
downloads the ones for its processor. `packaging/addons.sh` makes them, one
platform at a time; Intel's on Apple silicon:

```sh
vendor/rclone/build.sh macos-x86_64
PLATFORM=macos-x86_64 vendor/craft/build.sh
arch -x86_64 vendor/mpv/build.sh    # under Rosetta; needs brew install nasm
PLATFORM=macos-x86_64 packaging/addons.sh --upload
```

## Signing

Without `SIGN_IDENTITY` everything is signed ad hoc: the app runs, but a
copy downloaded from the internet is stopped by Gatekeeper until its user
lets it through (below). With an Apple Developer ID (Apple's Developer
Program), it is signed with that and the hardened runtime, ready to notarize:

```sh
SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" packaging/macos/bundle.sh
xcrun notarytool submit target/release/PowerCommander-*-macos-*.zip --keychain-profile <profile> --wait
xcrun stapler staple "target/release/Power Commander.app"
```

Stapled, zip the app again for the release.

## Installing an app signed ad hoc

For a release's notes:

> Unzip and drag **Power Commander** to Applications. macOS stops apps that
> Apple has not checked: open it once, then go to **System Settings >
> Privacy & Security** and click **Open Anyway**. Or, in Terminal:
>
> ```sh
> xattr -dr com.apple.quarantine "/Applications/Power Commander.app"
> ```
