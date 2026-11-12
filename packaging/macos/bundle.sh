#!/bin/sh
# Builds "Power Commander.app" for this Mac's processor, with 7-Zip, PDFium
# and libmpv from vendor/ (fetched or built first), and a zip of it for a
# release, both in target/release.
# SIGN_IDENTITY: a "Developer ID Application" certificate to sign with, for
# notarizing; unset, the app is signed ad hoc.
set -eu
cd "$(dirname "$0")/../.."
case "$(uname -m)" in
    arm64) ARCH=aarch64 ;;
    x86_64) ARCH=x86_64 ;;
    *) echo "No macOS build for $(uname -m)." >&2; exit 1 ;;
esac
PLATFORM=macos-$ARCH
for vendored in 7zip/$PLATFORM/7zz:fetch.sh pdfium/$PLATFORM/libpdfium.dylib:fetch.sh mpv/$PLATFORM/libmpv.2.dylib:build.sh; do
    file=${vendored%:*}
    [ -e "vendor/$file" ] || { echo "vendor/$file is missing: run vendor/${file%%/*}/${vendored#*:}" >&2; exit 1; }
done
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

cargo build --release -p pc-gui

# Programs in MacOS, libraries in Frameworks (pc-core's bundled.rs looks there), the rest in Resources.
APP="target/release/Power Commander.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources/licenses/PDFium"
cp target/release/powercommander "vendor/7zip/$PLATFORM/7zz" "$APP/Contents/MacOS/"
cp "vendor/pdfium/$PLATFORM/libpdfium.dylib" "vendor/mpv/$PLATFORM/"*.dylib "$APP/Contents/Frameworks/"
LICENSES="$APP/Contents/Resources/licenses"
cp LICENSE "$LICENSES/Power Commander.txt"
cp "vendor/7zip/$PLATFORM/License.txt" "$LICENSES/7-Zip.txt"
cp -R "vendor/pdfium/$PLATFORM/LICENSE" "vendor/pdfium/$PLATFORM/licenses" "$LICENSES/PDFium/"
cp vendor/mpv/README.md "$LICENSES/libmpv.md"
cp packaging/macos/PowerCommander.icns "$APP/Contents/Resources/"

# macOS 13: PDFium's. Automatic graphics switching: mpv's OpenGL leaves a
# laptop on its integrated GPU.
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Power Commander</string>
    <key>CFBundleExecutable</key>
    <string>powercommander</string>
    <key>CFBundleIconFile</key>
    <string>PowerCommander</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.airbusa.powercommander</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Power Commander</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSDesktopFolderUsageDescription</key>
    <string>Power Commander shows and works on the files you open.</string>
    <key>NSDocumentsFolderUsageDescription</key>
    <string>Power Commander shows and works on the files you open.</string>
    <key>NSDownloadsFolderUsageDescription</key>
    <string>Power Commander shows and works on the files you open.</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSLocalNetworkUsageDescription</key>
    <string>Power Commander connects to the SFTP and FTP servers you open on your network.</string>
    <key>NSNetworkVolumesUsageDescription</key>
    <string>Power Commander shows and works on the files you open.</string>
    <key>NSRemovableVolumesUsageDescription</key>
    <string>Power Commander shows and works on the files you open.</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
EOF

# What is inside first, then the app around it.
SIGN=${SIGN_IDENTITY:--}
set -- --force --sign "$SIGN"
[ "$SIGN" = - ] || set -- "$@" --options runtime --timestamp
codesign "$@" "$APP"/Contents/Frameworks/*.dylib "$APP/Contents/MacOS/7zz"
codesign "$@" "$APP"
codesign --verify --strict "$APP"

ZIP="target/release/PowerCommander-$VERSION-macos-$ARCH.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "$APP"
echo "$ZIP"
