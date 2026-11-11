use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Hsla, rgb};
use pc_core::theme::Palette;

#[derive(Clone, Copy)]
pub struct PanelColors {
    /// The active panel's cursor: its fill and its dotted frame.
    pub cursor: Hsla,
    pub cursor_frame: Hsla,
    pub alternate_row: Hsla,
    pub header_border: Hsla,
    /// The sorted column's title and the active panel's path.
    pub sorted_header: Hsla,
    /// The menu bar and the button bar.
    pub bar: Hsla,
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
    let list = palette.list();
    let menu = palette.menu();
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
        kit.muted = color(list.header);
        kit.muted_foreground = color(palette.dim);
        // Menus and popups; `ring` frames the highlighted item, and focus.
        kit.popover = color(menu.popup);
        kit.popover_foreground = color(palette.text);
        kit.accent = color(menu.hover);
        kit.accent_foreground = color(palette.text);
        kit.ring = color(menu.hover_frame);
        kit.list_active = color(palette.selection);
        // Column titles.
        kit.list_head = color(list.header);
        kit.tab_bar = color(palette.window);
        kit.tab_active = color(palette.view);
        kit.tab_foreground = color(palette.dim);
        kit.tab_active_foreground = color(palette.text);
    });
    PanelColors {
        cursor: color(list.selected_row),
        cursor_frame: color(palette.text),
        alternate_row: color(list.alternate_row),
        header_border: color(list.header_border),
        sorted_header: color(list.sorted_header),
        bar: color(palette.bar),
        marked: color(palette.marked),
    }
}
