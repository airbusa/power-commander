//! Help > About: what Power Commander is, and what it is made with.

pub const NAME: &str = "Power Commander";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WEBSITE: &str = "https://github.com/airbusa/power-commander";
/// Its licence's name, and its text.
pub const LICENCE: (&str, &str) = ("GNU General Public License 3.0 or later", "https://www.gnu.org/licenses/gpl-3.0.html");

/// Another project's work in it: its name, what it does here, its licence and site.
pub struct Part {
    pub name: &'static str,
    pub what: &'static str,
    pub licence: &'static str,
    pub website: &'static str,
}

const fn part(name: &'static str, what: &'static str, licence: &'static str, website: &'static str) -> Part {
    Part { name, what, licence, website }
}

/// What comes with it besides the add-ons (`addons::Builtin`) and the
/// libraries it is built on; Cargo.lock lists every crate.
pub const SHIPPED: &[Part] = &[
    part("Material Icon Theme", "File and folder icons", "MIT", "https://github.com/material-extensions/vscode-material-icon-theme"),
    part("Lucide", "Icons", "ISC", "https://lucide.dev"),
];

pub const LIBRARIES: &[Part] = &[
    part("GPUI", "The window, drawing and input (Zed's)", "Apache 2.0", "https://github.com/zed-industries/zed"),
    part("GPUI Kit", "Controls: lists, menus, fields, the editor", "Apache 2.0", "https://github.com/longbridge/gpui-component"),
    part("tree-sitter", "Code's colours in the viewer and editor", "MIT", "https://tree-sitter.github.io"),
    part("image", "Pictures", "MIT or Apache 2.0", "https://github.com/image-rs/image"),
    part("pdfium-render", "PDFium from Rust", "MIT or Apache 2.0", "https://github.com/ajrcarey/pdfium-render"),
    part("russh, russh-sftp", "SSH and SFTP", "Apache 2.0", "https://github.com/warp-tech/russh"),
    part("suppaftp", "FTP and FTPS", "MIT or Apache 2.0", "https://github.com/veeso/suppaftp"),
    part("rustls", "TLS", "Apache 2.0, ISC or MIT", "https://github.com/rustls/rustls"),
    part("keyring", "Passwords in the system's keyring", "MIT or Apache 2.0", "https://github.com/open-source-cooperative/keyring-rs"),
    part("zbus", "D-Bus: the desktop, devices, KDE Connect", "MIT", "https://github.com/z-galaxy/zbus"),
    part("trash", "The trash", "MIT", "https://github.com/ArturKovacs/trash"),
    part("html5ever", "HTML and e-books in the viewer", "MIT or Apache 2.0", "https://github.com/servo/html5ever"),
    part("similar", "Compare By Content", "Apache 2.0", "https://github.com/mitsuhiko/similar"),
    part("encoding_rs", "Code pages", "MIT or Apache 2.0, BSD 3-Clause", "https://github.com/hsivonen/encoding_rs"),
    part("tokio", "Network work in the background", "MIT", "https://github.com/tokio-rs/tokio"),
    part("serde", "Settings files", "MIT or Apache 2.0", "https://github.com/serde-rs/serde"),
    part("ratatui, ratatui-image", "pc, the terminal version", "MIT", "https://github.com/ratatui/ratatui"),
];
