use gpui_kit::component::GlobalState;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, App, Context, KeyBinding, Menu, MenuItem, SharedString, Window};
use pc_core::menu::{self, Command, Item, MENUS};
use pc_core::theme::Theme;

#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct Run(pub Command);

/// None follows the system.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct SetTheme(pub Option<SharedString>);

/// A tab of the active side.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct SelectTab(pub usize);

/// One of the active side's recently closed tabs.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct ReopenTab(pub usize);

pub fn init(cx: &mut App) {
    cx.bind_keys(
        menu::key_bindings()
            .into_iter()
            .map(|(keys, command)| KeyBinding::new(keys, Run(command), Some(crate::CONTEXT))),
    );
    cx.on_action(|Run(command): &Run, cx| {
        if *command == Command::Quit {
            cx.quit()
        }
    });
}

/// Call `AppMenuBar::reload` after.
pub fn set(themes: &[Theme], chosen: Option<&str>, cx: &mut App) {
    let menus = MENUS
        .iter()
        .map(|menu| {
            Menu::new(menu.label)
                .items(items(menu.items, themes, chosen))
                .owned()
        })
        .collect();
    // AppMenuBar draws these, on macOS too.
    GlobalState::global_mut(cx).set_app_menus(menus);
}

fn items(list: &[Item], themes: &[Theme], chosen: Option<&str>) -> Vec<MenuItem> {
    list.iter()
        .filter_map(|item| match *item {
            Item::Separator => Some(MenuItem::separator()),
            Item::Submenu { label, items: sub } => Some(MenuItem::submenu(
                Menu::new(label).items(items(sub, themes, chosen)),
            )),
            Item::Command { label, command, .. } => Some(MenuItem::action(label, Run(command))),
            Item::Themes { label } => {
                let follow =
                    MenuItem::action("Follow System", SetTheme(None)).checked(chosen.is_none());
                let each = themes.iter().map(|theme| {
                    let name = SharedString::from(theme.name.clone());
                    MenuItem::action(name.clone(), SetTheme(Some(name)))
                        .checked(chosen == Some(theme.name.as_str()))
                });
                let list = [follow, MenuItem::separator()].into_iter().chain(each);
                Some(MenuItem::submenu(Menu::new(label).items(list)))
            }
            Item::ClosedTabs { .. } => None,
        })
        .collect()
}

/// `checked` puts a check mark on a command; `closed` lists the recently closed tabs.
pub fn context_menu(
    mut menu: PopupMenu,
    items: &[Item],
    checked: impl Fn(Command) -> bool,
    closed: Vec<SharedString>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        menu = match *item {
            Item::Command { label, command, .. } => {
                menu.menu_with_check(label, checked(command), Box::new(Run(command)))
            }
            Item::Separator => menu.separator(),
            Item::ClosedTabs { label } => {
                let closed = closed.clone();
                menu.submenu(label, window, cx, move |mut menu, _, _| {
                    if closed.is_empty() {
                        return menu.menu_with_check_and_disabled(
                            "(none)",
                            false,
                            Box::new(ReopenTab(0)),
                            true,
                        );
                    }
                    for (ix, path) in closed.iter().enumerate() {
                        menu = menu.menu(path.clone(), Box::new(ReopenTab(ix)));
                    }
                    menu
                })
            }
            // shortcut: no submenus or themes in context menus yet; add them when one has them.
            Item::Submenu { .. } | Item::Themes { .. } => menu,
        };
    }
    menu
}
