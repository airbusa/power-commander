//! File icons by a VS Code file icon theme: the bundled Material Icon Theme.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

pub struct IconTheme {
    /// Icon id to its SVG's file name.
    files: HashMap<String, String>,
    dark: Names,
    /// Overrides for light themes.
    light: Names,
    file: String,
    folder: String,
}

/// Lowercase names to icon ids.
#[derive(Default)]
struct Names {
    file_names: HashMap<String, String>,
    file_extensions: HashMap<String, String>,
    folder_names: HashMap<String, String>,
}

pub fn material() -> &'static IconTheme {
    static THEME: OnceLock<IconTheme> = OnceLock::new();
    THEME.get_or_init(|| {
        let json = include_str!("../../../vendor/material-icon-theme/material-icons.json");
        IconTheme::parse(&serde_json::from_str(json).expect("bundled icon theme parses"))
    })
}

impl IconTheme {
    // shortcut: no `languageIds` (no language detection) or `highContrast` overrides.
    fn parse(json: &Value) -> Self {
        let files = json["iconDefinitions"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, def)| {
                let path = def["iconPath"].as_str()?;
                Some((id.clone(), path.rsplit('/').next()?.to_owned()))
            })
            .collect();
        let text = |key: &str| json[key].as_str().unwrap_or_default().to_owned();
        Self {
            files,
            dark: Names::parse(json),
            light: Names::parse(&json["light"]),
            file: text("file"),
            folder: text("folder"),
        }
    }

    /// The SVG file name of an entry's icon, as VS Code picks it: file
    /// name, then extensions longest first (`tar.gz` before `gz`).
    pub fn icon(&self, name: &str, is_dir: bool, light: bool) -> &str {
        let name = name.to_lowercase();
        let find = |pick: &dyn Fn(&Names) -> Option<&String>| {
            light
                .then(|| pick(&self.light))
                .flatten()
                .or_else(|| pick(&self.dark))
        };
        let id = if is_dir {
            find(&|n| n.folder_names.get(&name)).unwrap_or(&self.folder)
        } else {
            find(&|n| n.file_names.get(&name))
                .or_else(|| {
                    name.match_indices('.')
                        .find_map(|(dot, _)| find(&|n| n.file_extensions.get(&name[dot + 1..])))
                })
                .unwrap_or(&self.file)
        };
        self.files.get(id).map_or("", String::as_str)
    }
}

impl Names {
    fn parse(json: &Value) -> Self {
        let map = |key: &str| -> HashMap<String, String> {
            json[key]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, id)| Some((name.to_lowercase(), id.as_str()?.to_owned())))
                .collect()
        };
        Self {
            file_names: map("fileNames"),
            file_extensions: map("fileExtensions"),
            folder_names: map("folderNames"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_material_icons() {
        let theme = material();
        let icon = |name, is_dir| theme.icon(name, is_dir, false);
        assert_eq!(
            icon("README.md", false),
            "readme.svg",
            "file name before extension"
        );
        assert_eq!(
            icon("app.spec.ts", false),
            "test-ts.svg",
            "longest extension first"
        );
        assert_eq!(icon("backup.tar.gz", false), "zip.svg");
        assert_eq!(icon("photo.JPG", false), "image.svg");
        assert_eq!(icon("no-such-kind.qqq", false), "file.svg");
        assert_eq!(icon("Downloads", true), "folder-download.svg");
        assert_eq!(icon("whatever", true), "folder.svg");
        assert_eq!(theme.icon("page.jinja", false, true), "jinja_light.svg");
        assert_eq!(theme.icon("page.jinja", false, false), "jinja.svg");
        for name in [theme.file.as_str(), theme.folder.as_str()] {
            assert!(theme.files.contains_key(name));
        }
    }
}
