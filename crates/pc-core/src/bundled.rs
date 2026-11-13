//! What Power Commander ships with or adds (vendor/): 7-Zip and PDFium, and
//! the add-ons: libmpv, rclone and the Craft tools.

use std::path::{Path, PathBuf};

/// `name` of `vendor/<dir>`: shipped with the app, else an installed add-on
/// (`addons`), else built from source; the source tree's path when none is.
pub fn path(dir: &str, name: impl AsRef<Path>) -> PathBuf {
    let name = name.as_ref();
    shipped(dir, name)
        .or_else(|| crate::addons::file(dir, name))
        .unwrap_or_else(|| source(dir, name))
}

/// Shipped with the app or built from source, not an add-on.
pub fn builtin(dir: &str, name: impl AsRef<Path>) -> Option<PathBuf> {
    let name = name.as_ref();
    shipped(dir, name).or_else(|| Some(source(dir, name)).filter(|path| path.exists()))
}

/// A macOS app has libraries in `Contents/Frameworks` and programs beside the
/// executable; other releases have `<dir>/` beside it.
fn shipped(dir: &str, name: &Path) -> Option<PathBuf> {
    let library = name.extension().is_some_and(|extension| extension == "dylib");
    let exe = std::env::current_exe().ok()?;
    let beside = exe.parent()?;
    let path = match app_contents(beside) {
        Some(contents) if library => contents.join("Frameworks").join(name),
        Some(_) => beside.join(name),
        None => beside.join(dir).join(name),
    };
    path.exists().then_some(path)
}

/// Built from source: `vendor/<dir>/<os>-<arch>`.
fn source(dir: &str, name: &Path) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(dir)
        .join(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
        .join(name)
}

/// `….app/Contents`, when `dir` is its `MacOS`.
fn app_contents(dir: &Path) -> Option<&Path> {
    let contents = dir.parent()?;
    let app = contents.parent()?;
    let bundled = cfg!(target_os = "macos")
        && dir.ends_with("MacOS")
        && contents.ends_with("Contents")
        && app.extension().is_some_and(|extension| extension == "app");
    bundled.then_some(contents)
}
