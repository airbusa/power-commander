# The macOS app

`bundle.sh` builds `Power Commander.app` for the Mac it runs on (Apple
silicon or Intel) and a zip of it for a GitHub release, both in
`target/release`:

```sh
vendor/7zip/fetch.sh && vendor/pdfium/fetch.sh
packaging/macos/bundle.sh
```

| In the app | From |
|---|---|
| `Contents/MacOS/powercommander` | `cargo build --release -p pc-gui` |
| `Contents/MacOS/7zz` | `vendor/7zip` |
| `Contents/Frameworks/libpdfium.dylib` | `vendor/pdfium` |
| `Contents/Resources/PowerCommander.icns` | the app's icon, beside this file |
| `Contents/Resources/licenses` | the licenses of all of them |

It needs macOS 13 or later (PDFium's).

libmpv (video and sound), rclone (online storage) and the Craft tools (office
documents) are add-ons: the app works without them, and Settings > Add-ons
downloads them (`packaging/addons.sh` makes them).

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
