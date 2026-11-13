# rclone 1.75.2, our build (an add-on)

Online storage connects through rclone: `rclone serve sftp --stdio` for each
connection, which the app's own SFTP client speaks. This build has only the
services the app offers (`main.go`): S3 (Amazon, Backblaze B2, Cloudflare R2,
Wasabi, MinIO…), Google Drive, OneDrive, Dropbox, Box, pCloud, Yandex Disk,
iCloud Drive, Mega and Proton Drive, so it is half the size of rclone's own.

It is not in git: `vendor/rclone/build.sh` builds it from rclone's source
(MIT, `COPYING` beside each build) into `<os>-<arch>/`. Without it the app
uses an `rclone` on `PATH`.

It is an add-on, apart from the app: `packaging/addons.sh` zips it with its
license, signed as the app is on macOS, and Settings > Add-ons downloads it.
