// rclone with only the services Spark Commander offers, and the commands it runs.
package main

import (
	_ "github.com/rclone/rclone/backend/box"
	_ "github.com/rclone/rclone/backend/drive"
	_ "github.com/rclone/rclone/backend/dropbox"
	_ "github.com/rclone/rclone/backend/iclouddrive"
	_ "github.com/rclone/rclone/backend/local"
	_ "github.com/rclone/rclone/backend/mega"
	_ "github.com/rclone/rclone/backend/onedrive"
	_ "github.com/rclone/rclone/backend/pcloud"
	_ "github.com/rclone/rclone/backend/protondrive"
	_ "github.com/rclone/rclone/backend/s3"
	_ "github.com/rclone/rclone/backend/yandex"
	"github.com/rclone/rclone/cmd"
	_ "github.com/rclone/rclone/cmd/authorize"
	_ "github.com/rclone/rclone/cmd/config"
	_ "github.com/rclone/rclone/cmd/obscure"
	_ "github.com/rclone/rclone/cmd/serve/sftp"
	_ "github.com/rclone/rclone/cmd/version"
)

func main() {
	cmd.Main()
}
