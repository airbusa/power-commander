use gpui_kit::component::scroll::ScrollbarMode;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use gpui_kit::component::MacControls;
use gpui_kit::component::{NativeControls, Theme as KitTheme, ThemeMode};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_kit::component::{ActiveTheme, StateImages, ToolkitControls};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_kit::{Global, RenderImage, Window};
use gpui_kit::{App, Hsla, hsla, px, rgb};
use pc_core::theme::{ControlColors, ControlStyle, Palette};

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
        // The app writes in `text_sm`, 0.875 of the base: that is the desktop's size.
        if let Some(size) = palette.font_size {
            kit.font_size = px(size / 0.875);
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
    // The desktop style's controls (KDE's Breeze, Windows 11's), in its colours.
    if let Some(style) = pc_core::theme::control_style() {
        cx.set_global(native_controls(style, &palette.controls(style)));
        // Windows' and GNOME's corners, and their fields' frame, a button's
        // (GNOME's unframed: its fill).
        let radius = match style {
            ControlStyle::Breeze => None,
            ControlStyle::Fluent => Some(px(4.)),
            ControlStyle::Adwaita => Some(px(9.)),
        };
        if let Some(radius) = radius {
            let frame = color(palette.controls(style).button[0][1]);
            KitTheme::update(cx, |kit| {
                kit.radius = radius;
                kit.colors.input = frame;
            });
        }
    }
    // The toolkit's drawings are in the desktop's colours: not for the other look.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    match cx.try_global::<ToolkitDrawn>().filter(|drawn| drawn.dark == palette.dark) {
        Some(drawn) => cx.set_global(drawn.controls.clone()),
        None if cx.has_global::<ToolkitControls>() => drop(cx.remove_global::<ToolkitControls>()),
        None => {}
    }
    #[cfg(target_os = "macos")]
    cx.set_global(mac_controls(palette));
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

/// macOS: the controls as AppKit draws them, measured on macOS 26: fills of
/// the text's colour, white in dark mode, black in light.
#[cfg(target_os = "macos")]
fn mac_controls(palette: &Palette) -> MacControls {
    let dark = palette.dark;
    let ink = |alpha: f32| hsla(0., 0., if dark { 1. } else { 0. }, alpha);
    let accent = mac_color(dark, objc2_app_kit::NSColor::controlAccentColor).unwrap_or(0x007aff);
    MacControls {
        accent: Hsla::from(rgb(accent)),
        on_accent: hsla(0., 0., 1., 1.),
        button: ink(0.075),
        button_pressed: ink(0.16),
        button_disabled: ink(0.035),
        well: ink(0.1),
        groove: ink(0.07),
        groove_rim: ink(0.12),
        knob: if dark { Hsla::from(rgb(0xdfdfdf)) } else { hsla(0., 0., 1., 1.) },
        group: ink(0.03),
        field_frame: ink(0.1),
        text: ink(0.85),
        disabled_text: ink(0.25),
        window_frame: ink(if dark { 0.18 } else { 0.12 }),
        tab_track: ink(if dark { 0.06 } else { 0.1 }),
        tab: if dark { ink(0.19) } else { Hsla::from(rgb(0xf8f8f8)) },
        tab_rim: hsla(0., 0., 1., if dark { 0.2 } else { 1. }),
        tooltip: Hsla::from(rgb(if dark { 0x282828 } else { 0xe9e9e9 })),
        tooltip_frame: hsla(0., 0., 0., if dark { 0.75 } else { 0.2 }),
    }
}

/// The controls in `colors`, at the style's sizes.
fn native_controls(style: ControlStyle, colors: &ControlColors) -> NativeControls {
    let color = |value: u32| Hsla::from(rgb(value));
    let pair = |[fill, frame]: [u32; 2]| (color(fill), color(frame));
    // The sizes: a check box, its corners and frame, a radio's dot; a slider's
    // groove, a progress bar's groove and fill; the rim of the handle's middle.
    let (check, radius, frame, dot, groove, progress, rim) = match style {
        ControlStyle::Breeze => (16., 4., 1., 6., 6., [6., 6.], 1.),
        // WinUI's.
        ControlStyle::Fluent => (20., 4., 1., 12., 4., [1., 3.], 4.),
        // libadwaita's: 14 px and 3 px of padding.
        ControlStyle::Adwaita => (20., 6., 2., 8., 6., [8., 8.], 0.),
    };
    NativeControls {
        check_size: px(check),
        check_radius: px(radius),
        check_border: px(frame),
        radio_dot: px(dot),
        check_frame: color(colors.check_frame),
        check_fill: color(colors.check_fill),
        checked_frame: color(colors.checked_frame),
        checked_fill: color(colors.checked_fill),
        mark: color(colors.mark),
        groove: color(colors.groove),
        fill: color(colors.fill),
        slider_groove: px(groove),
        progress: progress.map(px),
        handle: color(colors.handle),
        handle_middle: color(colors.handle_middle),
        handle_rim: px(rim),
        handle_frame: colors.handle_frame.map(color),
        button: colors.button.map(pair),
        default_button: colors.default_button.map(pair),
        button_text: color(colors.button_text),
        default_button_text: color(colors.default_button_text),
        button_edge: colors.button_edge.map(|edges| edges.map(color)),
        tabs: colors.tabs.map(|tabs| tabs.map(color)),
        field_line: colors.field_line.map(color),
        field_fill: colors.field_fill.map(color),
        combo_button: style != ControlStyle::Breeze,
        bold: style == ControlStyle::Adwaita,
    }
}

/// macOS: the selected rows' colour, the user's accent, as Finder has it.
#[cfg(target_os = "macos")]
pub fn mac_selection(dark: bool) -> Option<u32> {
    mac_color(dark, objc2_app_kit::NSColor::selectedContentBackgroundColor)
}

/// A system colour in light or dark mode.
#[cfg(target_os = "macos")]
fn mac_color(dark: bool, color: fn() -> objc2::rc::Retained<objc2_app_kit::NSColor>) -> Option<u32> {
    use std::cell::Cell;

    use block2::StackBlock;
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColorSpace};
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    let rgb = Cell::new(None);
    let resolve = StackBlock::new(|| {
        let color = color().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        rgb.set(color.map(|color| [color.redComponent(), color.greenComponent(), color.blueComponent()]));
    });
    NSAppearance::appearanceNamed(name)?.performAsCurrentDrawingAppearance(&resolve);
    let [r, g, b] = rgb.get()?.map(|channel| (channel * 255.).round() as u32);
    Some(r << 16 | g << 8 | b)
}

/// The controls as the desktop's toolkit drew them, and whether in dark colours.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
struct ToolkitDrawn {
    dark: bool,
    controls: ToolkitControls,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl Global for ToolkitDrawn {}

/// The controls as the desktop's toolkit draws them (KDE's Qt style, the
/// GTK 3 theme of Xfce, MATE and Cinnamon; `pc_core::toolkit_style`),
/// once drawn; until then, and without it, the app's own.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn draw_toolkit_controls(window: &Window, cx: &mut App) {
    if pc_core::toolkit_style::helper().is_none() {
        return;
    }
    let scale = window.scale_factor();
    // Buttons are 2 rem high.
    let height = (f32::from(cx.theme().font_size) * 2.).round() as u32;
    let dark = cx.theme().mode.is_dark();
    cx.spawn(async move |cx| {
        let drawn = cx.background_executor().spawn(async move { pc_core::toolkit_style::draw(scale, height) }).await;
        let controls = match drawn {
            Ok(mut drawn) => toolkit_controls(&mut drawn.images, scale).map(|controls| (drawn.dark, controls)),
            Err(err) => {
                eprintln!("{err}");
                None
            }
        };
        let Some((drawn_dark, controls)) = controls else {
            return;
        };
        cx.update(|cx| {
            if drawn_dark == dark {
                cx.set_global(controls.clone());
                cx.refresh_windows();
            }
            cx.set_global(ToolkitDrawn { dark: drawn_dark, controls });
        });
    })
    .detach();
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn toolkit_controls(images: &mut std::collections::HashMap<String, pc_core::toolkit_style::Picture>, scale: f32) -> Option<ToolkitControls> {
    let mut states = |name: &str| -> Option<StateImages> {
        let [normal, hovered, pressed, disabled] = ["normal", "hovered", "pressed", "disabled"].map(|state| {
            let picture = images.remove(&name.replace("{}", state))?;
            // GPUI takes images as BGRA.
            let mut bgra = picture.rgba;
            bgra.chunks_exact_mut(4).for_each(|pixel| pixel.swap(0, 2));
            let image = image::RgbaImage::from_raw(picture.width, picture.height, bgra)?;
            Some(Arc::new(RenderImage::new([image::Frame::new(image)])))
        });
        Some(StateImages([normal?, hovered?, pressed?, disabled?]))
    };
    let mut button = |kind: &str| -> Option<[StateImages; 3]> {
        let [left, middle, right] = ["left", "middle", "right"].map(|part| states(&format!("button.{kind}.{{}}.{part}")));
        Some([left?, middle?, right?])
    };
    let button = [button("plain")?, button("default")?];
    let checkbox = [states("checkbox.off.{}")?, states("checkbox.on.{}")?];
    Some(ToolkitControls { scale, checkbox, button })
}
