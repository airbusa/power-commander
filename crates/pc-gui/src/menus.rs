use gpui_kit::component::GlobalState;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, App, Context, KeyBinding, Menu, MenuItem, SharedString, Window};
use pc_core::menu::{self, Command, Item, MENUS};
use pc_core::theme::Appearance;

#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct Run(pub Command);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = pc, no_json)]
pub struct SetAppearance(pub Appearance);

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
pub fn set(chosen: Appearance, cx: &mut App) {
    let menus = MENUS
        .iter()
        .map(|menu| {
            Menu::new(menu.label)
                .items(items(menu.items, chosen))
                .owned()
        })
        .collect();
    // AppMenuBar draws these, on macOS too.
    GlobalState::global_mut(cx).set_app_menus(menus);
}

fn items(list: &[Item], chosen: Appearance) -> Vec<MenuItem> {
    list.iter()
        .filter_map(|item| match *item {
            Item::Separator => Some(MenuItem::separator()),
            Item::Submenu { label, items: sub } => Some(MenuItem::submenu(
                Menu::new(label).items(items(sub, chosen)),
            )),
            Item::Command { label, command, .. } => Some(MenuItem::action(label, Run(command))),
            Item::Appearance { label } => {
                let each = [
                    ("As the system", Appearance::System),
                    ("Light", Appearance::Light),
                    ("Dark", Appearance::Dark),
                ]
                .map(|(name, appearance)| {
                    MenuItem::action(name, SetAppearance(appearance)).checked(chosen == appearance)
                });
                Some(MenuItem::submenu(Menu::new(label).items(each)))
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
            // shortcut: no submenus or appearances in context menus yet; add them when one has them.
            Item::Submenu { .. } | Item::Appearance { .. } => menu,
        };
    }
    menu
}
