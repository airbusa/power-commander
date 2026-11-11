//! The sidebar: its tabs on a rail, this computer's folders as a tree that
//! follows the active panel, and the servers by protocol.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::connections::{self, Connection, Connections};
use pc_core::devices::{self, Device};
use pc_core::menu::Command;
use pc_core::remote::{Address, Scheme};
use pc_core::sidebar::{self, FolderTree, SidebarTab};
use pc_core::{icons, theme};

use crate::{Commander, connections_dialog};

pub(crate) const WIDTH: f32 = 260.;
const RAIL: Pixels = px(44.);
const ROW: Pixels = px(22.);

pub(crate) struct Sidebar {
    /// The tab shown; none while hidden.
    pub(crate) tab: Option<SidebarTab>,
    /// Rail and tab together.
    pub(crate) width: f32,
    /// How much of the window colour it paints over the desktop; none: solid.
    tint: Option<f32>,
    tree: FolderTree,
    tree_scroll: UniformListScrollHandle,
    /// The active panel's folder, where the tree last went.
    followed: Option<PathBuf>,
    servers: Vec<Connection>,
    hosts: Vec<String>,
    closed: Vec<Scheme>,
    selected: Option<(Scheme, usize)>,
    /// KDE Connect's paired devices; none hides their tab.
    devices: Vec<Device>,
    /// The folders of the devices mounted, by id.
    device_folders: HashMap<String, Vec<(PathBuf, String)>>,
}

/// The "Opening…" note of a device being mounted.
struct Mounting;

/// Dragging the sidebar's edge.
pub(crate) struct SidebarDrag;

impl Sidebar {
    pub(crate) fn new(tab: Option<SidebarTab>, width: f32, hidden: bool) -> Self {
        Sidebar {
            tab,
            width: if width > 0. { width } else { WIDTH },
            tint: theme::sidebar_tint(),
            tree: FolderTree::new(hidden),
            tree_scroll: UniformListScrollHandle::new(),
            followed: None,
            servers: Connections::load().unwrap_or_default().saved,
            hosts: connections::ssh_hosts(),
            closed: Vec::new(),
            selected: None,
            devices: Vec::new(),
            device_folders: HashMap::new(),
        }
    }

    /// Show > Sidebar's check marks.
    pub(crate) fn shows(&self, command: Command) -> bool {
        match command {
            Command::SidebarTree => self.tab == Some(SidebarTab::Tree),
            Command::SidebarFavourites => self.tab == Some(SidebarTab::Favourites),
            _ => false,
        }
    }

    /// Reads the open folders again; `hidden` shows dot folders.
    pub(crate) fn refresh(&mut self, hidden: bool) {
        self.tree.refresh(hidden);
    }

    pub(crate) fn reload_servers(&mut self) {
        self.servers = Connections::load().unwrap_or_default().saved;
        self.hosts = connections::ssh_hosts();
    }
}

impl Commander {
    /// Show > Sidebar: that tab, or hidden when it shows it already.
    pub(crate) fn toggle_sidebar(&mut self, tab: SidebarTab, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.tab = (self.sidebar.tab != Some(tab)).then_some(tab);
        self.set_backdrop(window, cx);
        self.reload_menus(cx);
        cx.notify();
    }

    /// The desktop shows through, blurred, only while the sidebar is there:
    /// a see-through window loses subpixel text.
    pub(crate) fn set_backdrop(&self, window: &mut Window, cx: &mut App) {
        let see_through = self.sidebar.tab.is_some() && self.sidebar.tint.is_some();
        window.set_background_appearance(match see_through {
            false => WindowBackgroundAppearance::Opaque,
            true if cfg!(windows) => WindowBackgroundAppearance::MicaBackdrop,
            true => WindowBackgroundAppearance::Blurred,
        });
        window.set_see_through(see_through, cx);
    }

    /// The tree opens down to the active panel's folder, once that changed.
    fn follow_active(&mut self) {
        let panel = self.panels[self.active].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        if here == self.sidebar.followed {
            return;
        }
        if let Some(path) = &here
            && self.sidebar.tree.reveal(path)
            && let Some(ix) = self.sidebar.tree.rows().iter().position(|row| row.path == path)
        {
            self.sidebar.tree_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.sidebar.followed = here;
    }

    /// The active panel shows `path`, leaving its server for it.
    fn open_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        let go = move |panel: &mut pc_core::Panel| match remote {
            true => panel.disconnect(path),
            false => panel.go_to_folder(path),
        };
        self.navigate_on(self.active, false, go, window, cx);
    }

    /// The saved servers as on disk, changed, saved and shown.
    fn change_servers(&mut self, change: impl FnOnce(&mut Vec<Connection>), window: &mut Window, cx: &mut Context<Self>) {
        let mut all = Connections::load().unwrap_or_default();
        change(&mut all.saved);
        if let Err(err) = all.save() {
            window.push_notification(Notification::error(format!("Could not save the favourites: {err}")), cx);
        }
        self.sidebar.reload_servers();
        cx.notify();
    }

    /// New (none), edit, or duplicate a saved server.
    fn edit_server(&mut self, ix: Option<usize>, duplicate: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut connection = ix.and_then(|ix| self.sidebar.servers.get(ix).cloned()).unwrap_or_default();
        if duplicate {
            connection.name = format!("{} (copy)", connection.name);
        }
        let replace = ix.filter(|_| !duplicate);
        let this = cx.entity().downgrade();
        connections_dialog::open_edit(connection, window, cx, move |connection, window, cx| {
            let _ = this.update(cx, |this, cx| {
                let change = |saved: &mut Vec<Connection>| match replace.filter(|&ix| ix < saved.len()) {
                    Some(ix) => saved[ix] = connection,
                    None => saved.push(connection),
                };
                this.change_servers(change, window, cx)
            });
        });
    }

    fn delete_server(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let delete = |saved: &mut Vec<Connection>| {
            if ix < saved.len() {
                let gone = saved.remove(ix);
                connections::forget_password(saved, &gone);
            }
        };
        self.sidebar.selected = None;
        self.change_servers(delete, window, cx);
    }

    pub(crate) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let tab = self.sidebar.tab?;
        if tab == SidebarTab::Tree {
            self.follow_active();
        }
        let theme = cx.theme();
        let background = self.colors.window.opacity(self.sidebar.tint.unwrap_or(1.));
        // Material's coloured icons, the tab's name as a tooltip.
        let rail_item = |id: &'static str, icon: &'static str, name: &'static str, which: SidebarTab| {
            let selected = tab == which;
            div()
                .id(id)
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme.radius_lg)
                .cursor_pointer()
                .map(|el| match selected {
                    true => el.bg(theme.primary.opacity(0.18)),
                    false => el.hover(|style| style.bg(theme.foreground.opacity(0.06))),
                })
                .child(img(format!("material/{icon}")).size(px(22.)))
                .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sidebar.tab = Some(which);
                    this.reload_menus(cx);
                    cx.notify();
                }))
        };
        let rail = v_flex()
            .w(RAIL)
            .h_full()
            .flex_none()
            .p_1()
            .gap_1()
            .items_center()
            .child(rail_item("rail-tree", "tree.svg", "Folders", SidebarTab::Tree))
            .child(rail_item("rail-favourites", "folder-server.svg", "Favourites", SidebarTab::Favourites))
            .when(!self.sidebar.devices.is_empty(), |el| {
                el.child(rail_item("rail-devices", "folder-mobile.svg", "Devices", SidebarTab::Devices))
            });
        let content = match tab {
            SidebarTab::Tree => self.render_tree(cx).into_any_element(),
            SidebarTab::Favourites => self.render_favourites(cx).into_any_element(),
            SidebarTab::Devices => self.render_devices(cx).into_any_element(),
        };
        let theme = cx.theme();
        Some(
            h_flex()
                .relative()
                .h_full()
                .flex_none()
                .w(px(self.sidebar.width))
                .bg(background)
                .border_r_1()
                .border_color(theme.border)
                .child(rail)
                .child(v_flex().h_full().flex_1().min_w_0().py_1().child(content))
                .child(
                    div()
                        .id("sidebar-edge")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right(px(-3.))
                        .w(px(6.))
                        .occlude()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
                ),
        )
    }

    /// The sidebar's width, from where its edge is dragged in `row`.
    pub(crate) fn drag_sidebar(&mut self, row: Bounds<Pixels>, x: Pixels, cx: &mut Context<Self>) {
        self.sidebar.width = f32::from(x - row.left()).clamp(f32::from(RAIL) + 100., 640.);
        cx.notify();
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = self.sidebar.tree.rows().len();
        let list = uniform_list(
            "sidebar-tree",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let rows = this.sidebar.tree.rows();
                range.map(|ix| this.render_tree_row(ix, rows[ix], cx)).collect()
            }),
        )
        .track_scroll(&self.sidebar.tree_scroll)
        .size_full()
        .pr(px(12.));
        let rows = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(list)
            .child(div().absolute().inset_0().child(Scrollbar::vertical(&self.sidebar.tree_scroll)));
        v_flex().size_full().child(header("Folders", cx)).child(rows)
    }

    fn render_tree_row(&self, ix: usize, row: sidebar::Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let selected = self.sidebar.followed.as_deref() == Some(row.path);
        let light = !theme.mode.is_dark();
        let icon = match (row.depth, row.path == Path::new("/")) {
            (0, true) => "folder-root.svg",
            (0, false) => "folder-home.svg",
            _ => icons::material().icon(row.name, true, light),
        };
        let icon = img(format!("material/{icon}")).size(px(16.)).flex_none();
        let path = row.path.to_owned();
        let toggled = path.clone();
        let chevron = match row.expanded {
            true => IconName::ChevronDown,
            false => IconName::ChevronRight,
        };
        h_flex()
            .id(("tree-row", ix))
            .w_full()
            .h(ROW)
            .pl(px(4. + 14. * row.depth as f32))
            .pr_1()
            .gap_1()
            .rounded(theme.radius)
            .when(selected, |el| el.bg(self.colors.cursor))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(
                div()
                    .id(("tree-toggle", ix))
                    .size(px(16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!row.leaf, |el| {
                        el.child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.sidebar.tree.toggle(&toggled);
                                    cx.notify();
                                }),
                            )
                    }),
            )
            .child(icon)
            .child(div().flex_1().min_w_0().truncate().child(row.name.to_owned()))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                match event.click_count() {
                    2 => this.sidebar.tree.toggle(&path),
                    _ => this.open_folder(path.clone(), window, cx),
                }
                cx.notify();
            }))
    }

    /// Asks KDE Connect for its devices now and every few seconds after, for
    /// as long as the window is there.
    pub(crate) fn watch_devices(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let found = cx.background_executor().spawn(async { devices::paired() }).await;
                if this.update(cx, |this, cx| this.set_devices(found, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(10)).await;
            }
        })
        .detach();
    }

    fn set_devices(&mut self, devices: Vec<Device>, cx: &mut Context<Self>) {
        if devices == self.sidebar.devices {
            return;
        }
        // KDE Connect unmounts what it can no longer reach.
        let browsable = |id: &String| devices.iter().any(|device| device.id == *id && device.browsable);
        self.sidebar.device_folders.retain(|id, _| browsable(id));
        if devices.is_empty() && self.sidebar.tab == Some(SidebarTab::Devices) {
            self.sidebar.tab = Some(SidebarTab::Tree);
        }
        self.sidebar.devices = devices;
        cx.notify();
    }

    /// Mounts the device in the background, then shows its widest folder.
    fn open_device(&mut self, device: &Device, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(folders) = self.sidebar.device_folders.get(&device.id) {
            let first = folders[0].0.clone();
            return self.open_folder(first, window, cx);
        }
        let (id, name) = (device.id.clone(), device.name.clone());
        let note = Notification::info(format!("Opening {name}...")).id1::<Mounting>(id.clone()).autohide(false);
        window.push_notification(note, cx);
        let mount = {
            let id = id.clone();
            cx.background_executor().spawn(async move { devices::mount(&id) })
        };
        cx.spawn_in(window, async move |this, cx| {
            let mounted = mount.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Mounting>(id.clone(), cx);
                match mounted {
                    Ok(folders) => {
                        let first = folders[0].0.clone();
                        this.sidebar.device_folders.insert(id, folders);
                        this.open_folder(first, window, cx);
                    }
                    Err(err) => {
                        let note = Notification::error(format!("{name}: {err}")).autohide(false);
                        window.push_notification(note, cx);
                    }
                }
            });
        })
        .detach();
    }

    fn render_devices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let panel = self.panels[self.active].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        let mut list = v_flex().id("devices").flex_1().min_h_0().overflow_y_scroll().px_1();
        for device in &self.sidebar.devices {
            let theme = cx.theme();
            let icon = match device.kind.as_str() {
                "tablet" => Lucide::Tablet,
                "laptop" => Lucide::Laptop,
                "desktop" => Lucide::Monitor,
                "tv" => Lucide::Tv,
                _ => Lucide::Smartphone,
            };
            let detail = match (device.reachable, device.browsable) {
                (false, _) => "not connected",
                (true, false) => "shares no files",
                (true, true) => "",
            };
            let opened = device.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("device-{}", device.id)))
                    .h(ROW + px(2.))
                    .px_1()
                    .gap_1p5()
                    .rounded(theme.radius)
                    .map(|el| match device.browsable {
                        true => el
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_device(&opened, window, cx))),
                        false => el.text_color(theme.muted_foreground),
                    })
                    .child(Icon::new(icon).small().text_color(theme.muted_foreground))
                    .child(div().flex_none().max_w(relative(0.7)).truncate().child(device.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(theme.muted_foreground).child(detail)),
            );
            let Some(folders) = self.sidebar.device_folders.get(&device.id) else {
                continue;
            };
            // The deepest of its folders the active panel is in.
            let inside = here.as_ref().and_then(|here| {
                let holding = folders.iter().filter(|(path, _)| here.starts_with(path));
                holding.max_by_key(|(path, _)| path.components().count()).map(|(path, _)| path.clone())
            });
            for (ix, (path, name)) in folders.iter().enumerate() {
                let theme = cx.theme();
                let selected = inside.as_ref() == Some(path);
                let icon = icons::material().icon(name, true, !theme.mode.is_dark());
                let path = path.clone();
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("device-{}-{ix}", device.id)))
                        .h(ROW)
                        .pl(px(26.))
                        .pr_1()
                        .gap_1()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .when(selected, |el| el.bg(self.colors.cursor))
                        .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
                        .child(img(format!("material/{icon}")).size(px(16.)).flex_none())
                        .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_folder(path.clone(), window, cx))),
                );
            }
        }
        v_flex().size_full().child(header("Devices", cx)).child(list)
    }

    fn render_favourites(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let header = header("Favourites", cx)
            .child(
                Button::new("server-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New favourite...")
                    .on_click(cx.listener(|this, _, window, cx| this.edit_server(None, false, window, cx))),
            )
            .child(
                Button::new("server-address")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Globe)
                    .tooltip("Connect to an address...")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let side = this.active;
                        connections_dialog::open(cx.entity().downgrade(), side, window, cx)
                    })),
            );
        let groups = sidebar::favourites(&self.sidebar.servers, &self.sidebar.hosts);
        let mut list = v_flex().id("favourites").flex_1().min_h_0().overflow_y_scroll().px_1();
        if groups.is_empty() {
            list = list.child(
                div()
                    .p_2()
                    .text_color(theme.muted_foreground)
                    .child("No favourites yet: + adds one."),
            );
        }
        for (scheme, servers) in groups {
            let theme = cx.theme();
            let open = !self.sidebar.closed.contains(&scheme);
            let chevron = match open {
                true => IconName::ChevronDown,
                false => IconName::ChevronRight,
            };
            list = list.child(
                h_flex()
                    .id(("server-group", scheme as usize))
                    .h(ROW)
                    .px_1()
                    .gap_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                    .child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground))
                    .child(div().flex_1().font_semibold().child(scheme.name().to_uppercase()))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(servers.len().to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let closed = &mut this.sidebar.closed;
                        match closed.iter().position(|&s| s == scheme) {
                            Some(at) => drop(closed.remove(at)),
                            None => closed.push(scheme),
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for (ix, server) in servers.into_iter().enumerate() {
                list = list.child(self.render_server(scheme, ix, server, cx));
            }
        }
        v_flex().size_full().child(header).child(list)
    }

    fn render_server(&self, scheme: Scheme, ix: usize, server: sidebar::Favourite, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let selected = self.sidebar.selected == Some((scheme, ix));
        let commander = cx.entity().downgrade();
        let (address, saved) = (server.address.clone(), server.saved);
        h_flex()
            .id((scheme.name(), ix))
            .h(ROW + px(2.))
            .pl(px(22.))
            .pr_1()
            .gap_1p5()
            .rounded(theme.radius)
            .when(selected, |el| el.bg(self.colors.cursor))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(Icon::new(Lucide::Server).xsmall().text_color(theme.muted_foreground))
            .child(div().flex_none().max_w(relative(0.6)).truncate().child(server.name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(server.detail),
            )
            .on_click(cx.listener({
                let address = address.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.sidebar.selected = Some((scheme, ix));
                    if event.click_count() == 2 {
                        this.connect(this.active, address.clone(), window, cx);
                    }
                    cx.notify();
                }
            }))
            .context_menu(move |menu, _, _| server_menu(menu, commander.clone(), address.clone(), saved))
    }
}

/// A tab's title line; buttons go after it.
fn header(title: &'static str, cx: &App) -> Div {
    let title = div().flex_1().text_color(cx.theme().muted_foreground).child(title);
    h_flex().h(px(24.)).px_2().pb_1().gap_1().child(title)
}

/// A server's right-click menu; the saved ones can be changed.
fn server_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    commander: WeakEntity<Commander>,
    address: Address,
    saved: Option<usize>,
) -> gpui_kit::component::menu::PopupMenu {
    let act = |label: &'static str, run: fn(&mut Commander, &mut Window, &mut Context<Commander>)| {
        let commander = commander.clone();
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let _ = commander.update(cx, |this, cx| run(this, window, cx));
        })
    };
    let commander_for_connect = commander.clone();
    let connect = PopupMenuItem::new("Connect").on_click(move |_, window, cx| {
        let _ = commander_for_connect.update(cx, |this, cx| this.connect(this.active, address.clone(), window, cx));
    });
    let mut menu = menu.item(connect);
    if let Some(ix) = saved {
        let edit = |label: &'static str, run: fn(&mut Commander, usize, &mut Window, &mut Context<Commander>)| {
            let commander = commander.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = commander.update(cx, |this, cx| run(this, ix, window, cx));
            })
        };
        menu = menu
            .item(edit("Edit...", |this, ix, window, cx| this.edit_server(Some(ix), false, window, cx)))
            .item(edit("Duplicate", |this, ix, window, cx| this.edit_server(Some(ix), true, window, cx)))
            .item(edit("Delete", |this, ix, window, cx| this.delete_server(ix, window, cx)));
    }
    menu.separator()
        .item(act("New favourite...", |this, window, cx| this.edit_server(None, false, window, cx)))
        .item(act("Disconnect", |this, window, cx| this.disconnect(window, cx)))
}
