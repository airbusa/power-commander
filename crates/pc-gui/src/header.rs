//! The window's own title bar, GNOME's header bar: where the window draws its
//! frame (GNOME on Wayland always draws none for it; the ☰ button asks for
//! it), the title, the ☰ button holding the menus, and the window's buttons.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{ActiveTheme, GlobalState, Icon, Sizable, TitleBar, WindowButtonPictures, WindowCornerRadius, h_flex, top_corners};
use gpui_kit::*;

use crate::Commander;

impl Commander {
    /// Asks for our own title bar where the menus go behind the ☰ button.
    pub(crate) fn request_frame(&self, window: &mut Window) {
        let own = self.settings.menu.button();
        window.request_decorations(if own { WindowDecorations::Client } else { WindowDecorations::Server });
        // GPUI keeps telling the window manager of our frame's shadow margin otherwise.
        if !own {
            window.set_client_inset(px(0.));
        }
    }

    /// Whether the menus are behind the ☰ button, not in a bar.
    pub(crate) fn menus_behind_button(&self, window: &Window) -> bool {
        self.settings.menu.button() && own_frame(window)
    }

    /// The title bar, where the window draws its own frame.
    pub(crate) fn render_header(&self, window: &Window, cx: &mut App) -> Option<AnyElement> {
        let bar = title_bar("Power Commander", window, cx)?;
        let focus = self.focus.clone();
        let menu = Button::new("app-menu")
            .ghost()
            .small()
            .icon(Icon::new(Lucide::Menu))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                // The menu bar's menus, as they are now; their commands go to the panels.
                let menus = GlobalState::global(cx).app_menus().to_vec();
                menus.into_iter().fold(menu.action_context(focus.clone()), |menu, top| {
                    let items = top.items;
                    menu.submenu(top.name, window, cx, move |menu, window, cx| {
                        menu.with_menu_items(items.clone(), window, cx)
                    })
                })
            });
        // A press on the button must not start moving the window.
        let menu = div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(menu);
        let bar = bar
            .on_close_window(|_, _, cx| cx.quit())
            .child(h_flex().w_full().justify_end().pr_1().children(self.menus_behind_button(window).then_some(menu)));
        Some(bar.into_any_element())
    }
}

/// The title bar of a window that draws its own frame: any on GNOME on
/// Wayland, which draws none, and the main one for its ☰ button.
pub(crate) fn title_bar(title: impl Into<SharedString>, window: &Window, cx: &mut App) -> Option<TitleBar> {
    if !own_frame(window) {
        return None;
    }
    // The window buttons as GTK draws them, from the desktop's icon theme.
    if !cx.has_global::<WindowButtonPictures>()
        && let Some(pictures) = pc_core::theme::window_buttons()
    {
        let pictures = pictures.clone().map(|path| path.to_string_lossy().into_owned().into());
        cx.set_global(WindowButtonPictures(pictures));
    }
    let title = div().min_w_0().truncate().font_weight(FontWeight::BOLD).child(title.into());
    // Kept clear of the buttons on both sides, as GTK centres it.
    let title = div().absolute().inset_0().px(px(140.)).flex().items_center().justify_center().child(title);
    let mut bar = TitleBar::new().child(title);
    // As tall as GTK's, its top corners the window's.
    if let Some(header) = pc_core::theme::header_bar() {
        let (top_left, top_right) = top_corners(window, cx);
        let color = header.color(cx.theme().mode.is_dark(), window.is_window_active());
        bar = bar.h(px(header.height)).rounded_tl(top_left).rounded_tr(top_right).bg(rgb(color));
    }
    Some(bar)
}

/// The window's top corners as the GTK theme rounds them, before a window opens.
pub(crate) fn round_corners(cx: &mut App) {
    if let Some(header) = pc_core::theme::header_bar() {
        cx.set_global(WindowCornerRadius(px(header.radius)));
    }
}

fn own_frame(window: &Window) -> bool {
    matches!(window.window_decorations(), Decorations::Client { .. })
}
