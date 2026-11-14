# Builds the Windows release: a folder with the programs, 7-Zip and PDFium from
# vendor/ (fetched first), and a zip of it, both in target/release. libmpv,
# rclone and the Craft tools are add-ons (packaging/addons.sh). -Arch aarch64 cross-builds for ARM64 (needs MSVC's ARM64
# build tools and clang).
param([ValidateSet('x86_64', 'aarch64')] [string] $Arch = $(if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }))
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$platform = "windows-$Arch"
foreach ($file in "7zip\$platform\7z.exe", "pdfium\$platform\pdfium.dll") {
    if (-not (Test-Path "vendor\$file")) { throw "vendor\$file is missing: run vendor\$($file.Split('\')[0])\fetch.bat" }
}
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"$' | Select-Object -First 1).Matches.Groups[1].Value

$target = "$Arch-pc-windows-msvc"
cargo build --release --target $target -p sc-gui -p sc-tui
if ($LASTEXITCODE) { throw 'cargo build failed' }
$bin = "target\$target\release"

# Each tool's folder, its license inside, beside the programs: where sc-core's bundled.rs looks.
$dir = "target\release\spark-commander-$version-$platform"
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item "$bin\sparkcommander.exe", "$bin\scmd.exe", LICENSE $dir
foreach ($tool in '7zip', 'pdfium') {
    Copy-Item -Recurse "vendor\$tool\$platform" "$dir\$tool"
}

$zip = "$dir.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path $dir -DestinationPath $zip
$dir
$zip
