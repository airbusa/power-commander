use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Hsla, rgb};
use pc_core::theme::Palette;

#[derive(Clone, Copy)]
pub struct PanelColors {
    /// The active panel's cursor: its fill and its dotted frame.
    pub cursor: Hsla,
    pub cursor_frame: Hsla,
    /// The sorted column's title and the active panel's path: the accent, softly.
    pub soft_accent: Hsla,
    /// Marked files' names.
    pub marked: Hsla,
}

/// The system's look on GPUI Kit's colours, and the panels'.
pub fn apply(palette: &Palette, cx: &mut App) -> PanelColors {
    let color = |value: u32| Hsla::from(rgb(value));
    let mode = match palette.dark {
        true => ThemeMode::Dark,
        false => ThemeMode::Light,
    };
    KitTheme::change(mode, None, cx);
    KitTheme::update(cx, |kit| {
        if let Some(font) = &palette.font {
            kit.font_family = font.clone().into();
        }
        kit.scrollbar_mode = ScrollbarMode::Always;
        let kit = &mut kit.colors;
        kit.background = color(palette.view);
        kit.foreground = color(palette.text);
        kit.border = color(palette.border);
        // Dialogs' main buttons, the active tab's line.
        kit.primary = color(palette.selection);
        kit.primary_foreground = color(palette.selection_text);
        // The inactive panel's path.
        kit.muted = color(palette.header);
        kit.muted_foreground = color(palette.dim);
        kit.popover = color(palette.window);
        kit.popover_foreground = color(palette.text);
        kit.accent = color(palette.selection);
        kit.accent_foreground = color(palette.selection_text);
        kit.list_active = color(palette.selection);
        // Column titles.
        kit.list_head = color(palette.header);
        kit.tab_bar = color(palette.window);
        kit.tab_active = color(palette.view);
        kit.tab_foreground = color(palette.dim);
        kit.tab_active_foreground = color(palette.text);
    });
    let faint = color(palette.selection).opacity(0.3);
    PanelColors {
        cursor: faint,
        cursor_frame: color(palette.text),
        soft_accent: color(palette.header).blend(faint),
        marked: color(palette.marked),
    }
}
