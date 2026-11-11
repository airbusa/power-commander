use gpui_kit::component::GlobalState;
use gpui_kit::component::menu::PopupMenu;
use std::rc::Rc;

use gpui_kit::{
    Action, App, Context, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Menu,
    MenuItem, SharedString, Unbind, Window,
};
use pc_core::keymap::KeySettings;
use pc_core::menu::{Command, Item, MENUS};
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

/// The commands' keys. Keys bound `before` are unbound first: the new
/// bindings come after that, so they win.
pub fn bind(keys: &KeySettings, before: Option<&KeySettings>, cx: &mut App) {
    let context = || Some(Rc::new(KeyBindingContextPredicate::parse(crate::CONTEXT).unwrap()));
    let binding = |key: &str, action: Box<dyn Action>| {
        KeyBinding::load(key, action, context(), false, None, &DummyKeyboardMapper).ok()
    };
    let unbound = before.into_iter().flat_map(KeySettings::bindings);
    let unbind = unbound.filter_map(|(key, _)| binding(&key, Box::new(Unbind(Run::name_for_type().into()))));
    cx.bind_keys(unbind.collect::<Vec<_>>());
    // A key the settings file got wrong is left out.
    let bound = keys.bindings().into_iter();
    cx.bind_keys(bound.filter_map(|(key, command)| binding(&key, Box::new(Run(command)))).collect::<Vec<_>>());
}

pub fn init(cx: &mut App) {
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
