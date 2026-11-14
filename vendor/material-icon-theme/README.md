# Material Icon Theme 5.40.0 (bundled)

The file and folder icons of the panels, from the VS Code extension
`PKief.material-icon-theme` 5.40.0 (https://open-vsx.org/extension/PKief/material-icon-theme,
source https://github.com/material-extensions/vscode-material-icon-theme), unmodified:

- `material-icons.json`: the extension's `dist/material-icons.json`, which maps
  file names, extensions and folder names to icons.
- `icons/`: its SVGs.
- `LICENSE.txt`: MIT. The icons draw on Material Design Icons
  (https://pictogrammers.com/library/mdi/) and Material Symbols
  (https://fonts.google.com/icons), both Apache 2.0.

`sc-core` reads the mapping; `sc-gui` embeds the SVGs at build time.

The `.vsix` from Open VSX had SHA-256
`55b565f200881abfd807baf9be52fce45fd52fef013ba71b0f8394984ce1ae06` (checked).
To update: unpack a newer `.vsix` the same way and run `cargo test -p sc-core`.
