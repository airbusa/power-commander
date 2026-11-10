use gpui_kit::component::{ActiveTheme, Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Hsla, rgba};
use pc_core::theme::{Kind, Theme};

#[derive(Clone, Copy)]
pub struct Cursor {
    pub background: Option<Hsla>,
    pub foreground: Hsla,
    /// High contrast only.
    pub border: Option<Hsla>,
}

#[derive(Clone, Copy)]
pub struct PanelColors {
    pub active: Cursor,
    pub inactive: Cursor,
    /// High contrast only.
    pub active_title_border: Option<Hsla>,
    /// Marked files' names.
    pub marked: Hsla,
}

pub fn apply(theme: &Theme, cx: &mut App) -> PanelColors {
    let color = |key: &str| theme.color(key).map(|c| Hsla::from(rgba(c)));
    let mode = match theme.kind.is_dark() {
        true => ThemeMode::Dark,
        false => ThemeMode::Light,
    };
    KitTheme::change(mode, None, cx);
    KitTheme::update(cx, |kit| {
        let kit = &mut kit.colors;
        let set = |field: &mut Hsla, key: &str| {
            if let Some(c) = color(key) {
                *field = c;
            }
        };
        set(&mut kit.background, "editor.background");
        set(&mut kit.foreground, "foreground");
        set(&mut kit.border, "panel.border");
        // Active panel title.
        set(&mut kit.primary, "button.background");
        set(&mut kit.primary_foreground, "button.foreground");
        // Inactive title, column titles.
        set(&mut kit.muted, "sideBar.background");
        set(&mut kit.muted_foreground, "descriptionForeground");
        set(&mut kit.popover, "menu.background");
        set(&mut kit.popover_foreground, "menu.foreground");
        set(&mut kit.accent, "menu.selectionBackground");
        set(&mut kit.accent_foreground, "menu.selectionForeground");
        set(&mut kit.list_active, "list.activeSelectionBackground");
        // Column titles.
        set(&mut kit.list_head, "sideBarSectionHeader.background");
        set(&mut kit.tab_bar, "editorGroupHeader.tabsBackground");
        set(&mut kit.tab_active, "tab.activeBackground");
        set(&mut kit.tab_foreground, "tab.inactiveForeground");
        set(&mut kit.tab_active_foreground, "tab.activeForeground");
    });
    let foreground = cx.theme().foreground;
    let high_contrast = matches!(theme.kind, Kind::HighContrastDark | Kind::HighContrastLight);
    let cursor = |background, foreground_key, border_key| Cursor {
        background: color(background),
        foreground: color(foreground_key).unwrap_or(foreground),
        border: high_contrast.then(|| color(border_key)).flatten(),
    };
    PanelColors {
        active: cursor(
            "list.activeSelectionBackground",
            "list.activeSelectionForeground",
            "list.focusOutline",
        ),
        inactive: cursor(
            "list.inactiveSelectionBackground",
            "list.inactiveSelectionForeground",
            "contrastBorder",
        ),
        active_title_border: high_contrast
            .then(|| color("contrastActiveBorder"))
            .flatten(),
        marked: color("errorForeground").unwrap_or(foreground),
    }
}
