//! What Power Commander ships with (vendor/): 7-Zip, PDFium and, on macOS,
//! libmpv.

use std::path::{Path, PathBuf};

/// `name` of `vendor/<dir>`. A macOS app has libraries in
/// `Contents/Frameworks` and programs beside the executable; other releases
/// have `<dir>/` beside it; built from source, `vendor/<dir>/<os>-<arch>`.
pub fn path(dir: &str, name: impl AsRef<Path>) -> PathBuf {
    let name = name.as_ref();
    let library = name.extension().is_some_and(|extension| extension == "dylib");
    let exe = std::env::current_exe().ok();
    let shipped = exe.as_deref().and_then(Path::parent).map(|beside| match app_contents(beside) {
        Some(contents) if library => contents.join("Frameworks").join(name),
        Some(_) => beside.join(name),
        None => beside.join(dir).join(name),
    });
    match shipped {
        Some(path) if path.exists() => path,
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor")
            .join(dir)
            .join(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
            .join(name),
    }
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
