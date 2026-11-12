# rclone 1.75.2, our build (bundled)

Online storage connects through rclone: `rclone serve sftp --stdio` for each
connection, which the app's own SFTP client speaks. This build has only the
services the app offers (`main.go`): S3 (Amazon, Backblaze B2, Cloudflare R2,
Wasabi, MinIO…), Google Drive, OneDrive, Dropbox, Box, pCloud, Yandex Disk,
iCloud Drive, Mega and Proton Drive, so it is half the size of rclone's own.

It is not in git: `vendor/rclone/build.sh` builds it from rclone's source
(MIT, `COPYING` beside each build) into `<os>-<arch>/`. Without it the app
uses an `rclone` on `PATH`.

Releases copy the platform's folder to `rclone/` beside the executable.
