# Bundled color themes

VS Code's built-in color themes, copied unchanged from
https://github.com/microsoft/vscode (commit 9471f232862ef9f4a1e2389562cbb5bbb67b2569),
`extensions/theme-*/themes/*.json`. PowerCommander embeds them in `pc-core`
(`src/theme.rs` lists them with the labels and kinds of their extensions'
`package.json`).

- `LICENSE-vscode.txt`: VS Code's MIT license, for all of them, and for the
  color defaults `src/theme.rs` ports from VS Code's color registry.
- `LICENSE-colorsublime.txt`: Abyss, Kimbie Dark, Monokai, Monokai Dimmed,
  Quiet Light, Red, Solarized Dark, Solarized Light and Tomorrow Night Blue
  come from Colorsublime-Themes (MIT), as VS Code's `ThirdPartyNotices.txt`
  says.

Releases must ship both license texts. To update, copy the files again from a
newer commit and change the commit above.
