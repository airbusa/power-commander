use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{BreezeControls, Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Hsla, hsla, rgb};
use pc_core::theme::Palette;

#[derive(Clone, Copy)]
pub struct PanelColors {
    /// The active panel's cursor: its fill and its dotted frame.
    pub cursor: Hsla,
    pub cursor_frame: Hsla,
    /// While another window is in front.
    pub cursor_inactive: Hsla,
    /// macOS: the cursor's names, on the accent.
    pub cursor_text: Option<Hsla>,
    pub sidebar_selection: Hsla,
    pub alternate_row: Hsla,
    pub header_border: Hsla,
    pub active_path: Hsla,
    pub inactive_path: Hsla,
    /// The menu bar.
    pub bar: Hsla,
    /// The sidebar, under its tint.
    pub window: Hsla,
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
        // A focused field's border takes the accent, as Breeze's; the ring
        // around it was cut off by the dialogs.
        kit.focus_ring = false;
        let kit = &mut kit.colors;
        kit.background = color(palette.view);
        kit.foreground = color(palette.text);
        kit.border = color(palette.border);
        // Dialogs' main buttons, the active tab's line.
        kit.primary = color(palette.selection);
        kit.primary_foreground = color(palette.selection_text);
        // Quiet fills, such as group titles.
        kit.muted = color(list.header);
        kit.muted_foreground = color(palette.dim);
        // Menus and popups; `ring` frames the highlighted item, and focus.
        kit.popover = color(menu.popup);
        kit.popover_foreground = color(palette.text);
        kit.accent = color(menu.hover);
        kit.accent_foreground = color(palette.text);
        kit.ring = color(menu.hover_frame);
        kit.list_active = color(palette.selection);
        // Column titles and the button bar.
        kit.list_head = color(list.header);
        kit.tab_bar = color(palette.window);
        kit.tab_active = color(palette.view);
        kit.tab_foreground = color(palette.dim);
        kit.tab_active_foreground = color(palette.text);
    });
    // On KDE the controls take Breeze's look, in the scheme's colours.
    if pc_core::theme::breeze_controls() {
        cx.set_global(BreezeControls {
            button: color(palette.button),
            frame: color(palette.border),
            checked: color(palette.checked_fill()),
            highlight: color(palette.selection),
            mark: color(palette.text),
        });
    }
    // macOS's cursor is the accent, filled, as Finder's; elsewhere a tint, framed.
    let filled = cfg!(target_os = "macos");
    PanelColors {
        cursor: color(if filled { palette.selection } else { list.selected_row }),
        cursor_frame: if filled { gpui_kit::transparent_black() } else { color(palette.text) },
        // AppKit's unemphasized selection.
        cursor_inactive: match (filled, palette.dark) {
            (false, _) => color(list.selected_row),
            (true, true) => color(0x464646),
            (true, false) => color(0xdcdcdc),
        },
        cursor_text: filled.then(|| color(palette.selection_text)),
        // AppKit's sidebar selection: faint white or black over the blur.
        sidebar_selection: match (filled, palette.dark) {
            (false, _) => color(list.selected_row),
            (true, true) => hsla(0., 0., 1., 0.12),
            (true, false) => hsla(0., 0., 0., 0.1),
        },
        alternate_row: color(list.alternate_row),
        header_border: color(list.header_border),
        active_path: color(list.active_path),
        inactive_path: color(list.inactive_path),
        bar: color(palette.bar),
        window: color(palette.window),
        marked: color(palette.marked),
    }
}

/// macOS: the selected rows' colour, the user's accent, as Finder has it.
#[cfg(target_os = "macos")]
pub fn mac_selection(dark: bool) -> Option<u32> {
    use std::cell::Cell;

    use block2::StackBlock;
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorSpace};
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    let rgb = Cell::new(None);
    let resolve = StackBlock::new(|| {
        let color = NSColor::selectedContentBackgroundColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        rgb.set(color.map(|color| [color.redComponent(), color.greenComponent(), color.blueComponent()]));
    });
    NSAppearance::appearanceNamed(name)?.performAsCurrentDrawingAppearance(&resolve);
    let [r, g, b] = rgb.get()?.map(|channel| (channel * 255.).round() as u32);
    Some(r << 16 | g << 8 | b)
}
