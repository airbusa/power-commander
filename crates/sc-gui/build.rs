// Embeds the Material Icon Theme's SVGs, sorted by name for a binary search.
use std::path::Path;
use std::{env, fs};

fn main() {
    // Windows: the app's icon in the .exe, resource 1, which GPUI loads for its windows.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../packaging/windows/sparkcommander.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/windows/sparkcommander.ico");
        resource.compile().expect("the Windows icon resource");
    }
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let dir = Path::new(&manifest).join("../../vendor/material-icon-theme/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("vendor/material-icon-theme/icons")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".svg"))
        .collect();
    names.sort();
    let mut code = String::from("pub const MATERIAL_ICONS: &[(&str, &[u8])] = &[\n");
    for name in names {
        let path = dir.join(&name);
        code += &format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            path.to_str().unwrap()
        );
    }
    code += "];\n";
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("material_icons.rs"),
        code,
    )
    .unwrap();
}
