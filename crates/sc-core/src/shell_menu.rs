//! Windows: Explorer's context menu for files, as Total Commander shows it,
//! with the app's own items: the name (what Enter does), View, Pack files.

use std::cell::RefCell;
use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, mpsc};

use windows::Win32::Foundation::{E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CLSID_NewMenu, CMF_CANRENAME, CMF_EXPLORE, CMF_EXTENDEDVERBS, CMIC_MASK_CONTROL_DOWN,
    CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW,
    IContextMenu, IContextMenu2, IContextMenu3, IShellExtInit, IShellFolder, ILCreateFromPathW,
    ILFindLastID, ILFree, SEE_MASK_UNICODE, SHBindToParent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DeleteMenu, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    TranslateMessage, WM_APP, GetMenuDefaultItem, GetMenuItemCount, GetMenuItemID, GetSubMenu, HMENU, InsertMenuW,
    MF_BYCOMMAND, MF_BYPOSITION, MF_STRING, RegisterClassW, SW_SHOWNORMAL, SetForegroundWindow, SetMenuDefaultItem,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WINDOW_EX_STYLE, WM_DRAWITEM, WM_INITMENUPOPUP,
    WM_MEASUREITEM, WM_MENUCHAR, WNDCLASSW, WS_POPUP,
};
use windows::core::{Interface, PCSTR, PCWSTR, PSTR, Result, w};

/// What was picked, when the app does it rather than Explorer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The name on top: what Enter does.
    Open,
    View,
    Pack,
    Rename,
    Delete,
    /// Explorer's Delete with Shift held.
    DeletePermanently,
    /// Explorer did one of its own; the folder may have changed.
    Done,
}

const SHELL_FIRST: u32 = 1;
const SHELL_LAST: u32 = 0x6fff;
const NEW_FIRST: u32 = 0x7000;
const NEW_LAST: u32 = 0x7eff;
const OPEN: u32 = 0x7f00;
const VIEW: u32 = 0x7f01;
const PACK: u32 = 0x7f02;

/// Shows the menu at the pointer for `paths`, all in one folder; with
/// `background` for the folder in `paths` itself, with Explorer's New
/// submenu. `header` is the one entry's name and whether it is a folder.
/// `owner` is the window's HWND; `dark` draws the menu dark. Blocks until the menu closes and the
/// command ran: call it off the app's thread.
pub fn show(owner: isize, paths: &[PathBuf], background: bool, header: Option<(&str, bool)>, dark: bool) -> io::Result<Option<Choice>> {
    let (reply, answer) = mpsc::channel();
    let header = header.map(|(name, is_dir)| (name.to_owned(), is_dir));
    let request = Request { owner, paths: paths.to_vec(), background, header, dark, reply };
    let mut thread = THREAD.lock().unwrap_or_else(PoisonError::into_inner);
    if thread.is_none() {
        *thread = Some(start()?);
    }
    let Some((requests, id)) = thread.as_ref() else { unreachable!() };
    requests.send(request).map_err(io::Error::other)?;
    unsafe { PostThreadMessageW(*id, WM_APP, WPARAM(0), LPARAM(0)) }.map_err(io::Error::other)?;
    drop(thread);
    answer.recv().map_err(io::Error::other)?
}

struct Request {
    owner: isize,
    paths: Vec<PathBuf>,
    background: bool,
    header: Option<(String, bool)>,
    dark: bool,
    reply: mpsc::Sender<io::Result<Option<Choice>>>,
}

/// The thread showing the menus, and its id. It lives on, taking messages,
/// as what Explorer starts from a menu (Properties, Open with) calls back.
static THREAD: Mutex<Option<(mpsc::Sender<Request>, u32)>> = Mutex::new(None);

fn start() -> io::Result<(mpsc::Sender<Request>, u32)> {
    let (sender, requests) = mpsc::channel::<Request>();
    let (started, id) = mpsc::channel();
    std::thread::Builder::new().name("shell menu".into()).spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok();
        let mut message = MSG::default();
        // Makes the thread's message queue before anyone posts to it.
        let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
        started.send(com.map(|()| unsafe { GetCurrentThreadId() })).ok();
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            if message.message == WM_APP && message.hwnd.0.is_null() {
                // Those whose message a menu's own loop took come along too.
                for request in requests.try_iter() {
                    let header = request.header.as_ref().map(|(name, is_dir)| (name.as_str(), *is_dir));
                    let owner = HWND(request.owner as _);
                    let chosen = unsafe { track(owner, &request.paths, request.background, header, request.dark) };
                    request.reply.send(chosen.map_err(io::Error::other)).ok();
                }
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    })?;
    let id = id.recv().map_err(io::Error::other)?.map_err(io::Error::other)?;
    Ok((sender, id))
}

struct Pidl(*mut ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) };
    }
}

struct Menu(HMENU);

impl Drop for Menu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

unsafe fn track(owner: HWND, paths: &[PathBuf], background: bool, header: Option<(&str, bool)>, dark: bool) -> Result<Option<Choice>> {
    let pidls: Vec<Pidl> = paths
        .iter()
        .map(|path| match unsafe { ILCreateFromPathW(PCWSTR(wide(path).as_ptr())) } {
            pidl if pidl.is_null() => Err(E_INVALIDARG.into()),
            pidl => Ok(Pidl(pidl)),
        })
        .collect::<Result<_>>()?;
    let first = pidls.first().ok_or(windows::core::Error::from(E_INVALIDARG))?;
    let parent: IShellFolder = unsafe { SHBindToParent(first.0, None)? };
    // shortcut: items of other folders (search results) are left out; show them with their own folder's menu when asked.
    let parent_path = paths[0].parent();
    let children: Vec<*const ITEMIDLIST> = pidls
        .iter()
        .zip(paths)
        .filter(|(_, path)| path.parent() == parent_path)
        .map(|(pidl, _)| unsafe { ILFindLastID(pidl.0) } as *const ITEMIDLIST)
        .collect();
    let items: IContextMenu = unsafe { parent.GetUIObjectOf(owner, &children, None)? };
    let menu = Menu(unsafe { CreatePopupMenu()? });
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let extended = if shift { CMF_EXTENDEDVERBS } else { 0 };
    unsafe { items.QueryContextMenu(menu.0, 0, SHELL_FIRST, SHELL_LAST, CMF_EXPLORE | CMF_CANRENAME | extended) }.ok()?;
    let mut new = None;
    if background {
        // As Total Commander's: the folder shown is neither deleted nor renamed from here.
        while let Some(at) = position(menu.0, |id| matches!(verb(&items, id).as_deref(), Some("delete" | "rename"))) {
            unsafe { DeleteMenu(menu.0, at - 1, MF_BYPOSITION)? };
        }
        let handler: IContextMenu = unsafe { CoCreateInstance(&CLSID_NewMenu, None, CLSCTX_INPROC_SERVER)? };
        unsafe { handler.cast::<IShellExtInit>()?.Initialize(Some(first.0), None, None)? };
        // Above Properties, as Explorer has it.
        let at = position(menu.0, |id| verb(&items, id).as_deref() == Some("properties"));
        let at = at.map_or(u32::MAX, |at| at.saturating_sub(1));
        let before = submenus(menu.0);
        unsafe { handler.QueryContextMenu(menu.0, at, NEW_FIRST, NEW_LAST, 0) }.ok()?;
        let submenu = submenus(menu.0).into_iter().find(|submenu| !before.contains(submenu));
        new = submenu.map(|submenu| (handler, submenu));
    } else {
        own_items(&menu, &items, header)?;
    }
    let mut at = POINT::default();
    unsafe { GetCursorPos(&mut at)? };
    let helper = Helper::new(owner)?;
    unsafe { theme(helper.0, dark) };
    let shown = Shown { items: items.clone(), new: new.clone() };
    SHOWN.with(|slot| *slot.borrow_mut() = Some(shown));
    // The menu takes the keys only from the foreground window; the app's window gets it back.
    let _ = unsafe { SetForegroundWindow(helper.0) };
    let flags = (TPM_RETURNCMD | TPM_RIGHTBUTTON).0;
    let id = unsafe { TrackPopupMenuEx(menu.0, flags, at.x, at.y, helper.0, None) }.0 as u32;
    let _ = unsafe { SetForegroundWindow(owner) };
    SHOWN.with(|slot| slot.borrow_mut().take());
    let (handler, first_id) = match (id, &new) {
        (0, _) => return Ok(None),
        (OPEN, _) => return Ok(Some(Choice::Open)),
        (VIEW, _) => return Ok(Some(Choice::View)),
        (PACK, _) => return Ok(Some(Choice::Pack)),
        (NEW_FIRST..=NEW_LAST, Some((handler, _))) => (handler, NEW_FIRST),
        _ => (&items, SHELL_FIRST),
    };
    let offset = id - first_id;
    // The app renames and deletes, as Total Commander does, so its panels follow.
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    match verb(handler, offset).as_deref() {
        Some("rename") => return Ok(Some(Choice::Rename)),
        Some("delete") if shift => return Ok(Some(Choice::DeletePermanently)),
        Some("delete") => return Ok(Some(Choice::Delete)),
        _ => {}
    }
    let folder = match background {
        true => paths[0].clone(),
        false => parent_path.map(Path::to_path_buf).unwrap_or_default(),
    };
    let folder = wide(&folder);
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: SEE_MASK_UNICODE
            | CMIC_MASK_PTINVOKE
            | if shift { CMIC_MASK_SHIFT_DOWN } else { 0 }
            | if ctrl { CMIC_MASK_CONTROL_DOWN } else { 0 },
        hwnd: owner,
        // The command by its offset, as MAKEINTRESOURCE.
        lpVerb: PCSTR(offset as usize as *const u8),
        lpVerbW: PCWSTR(offset as usize as *const u16),
        lpDirectoryW: PCWSTR(folder.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: at,
        ..Default::default()
    };
    unsafe { handler.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO)? };
    Ok(Some(Choice::Done))
}

/// Total Commander's: the name on top, bold, for Enter (a file's Open
/// goes, which does the same); View above the first group's end; Pack
/// files below Copy.
fn own_items(menu: &Menu, items: &IContextMenu, header: Option<(&str, bool)>) -> Result<()> {
    let add = |at: u32, id: u32, label: &str| {
        let label: Vec<u16> = label.replace('&', "&&").encode_utf16().chain([0]).collect();
        unsafe { InsertMenuW(menu.0, at, MF_BYPOSITION | MF_STRING, id as usize, PCWSTR(label.as_ptr())) }
    };
    if let Some((name, is_dir)) = header {
        let default = unsafe { GetMenuDefaultItem(menu.0, 0, Default::default()) };
        if !is_dir && default != u32::MAX && verb(items, default.wrapping_sub(SHELL_FIRST)).as_deref() == Some("open") {
            unsafe { DeleteMenu(menu.0, default, MF_BYCOMMAND)? };
        }
        add(0, OPEN, name)?;
        unsafe { SetMenuDefaultItem(menu.0, OPEN, 0)? };
    }
    // Before the first submenu or separator below the top, else below the first separator.
    let top = header.is_some() as i32;
    let count = unsafe { GetMenuItemCount(Some(menu.0)) };
    let ends = |pos: i32| matches!(unsafe { GetMenuItemID(menu.0, pos) }, 0 | u32::MAX);
    let view = match (top..count).find(|&pos| ends(pos)) {
        Some(pos) if pos > top => pos,
        Some(pos) => pos + 1,
        None => count,
    };
    add(view as u32, VIEW, "View")?;
    let pack = position(menu.0, |id| verb(items, id).as_deref() == Some("copy"));
    add(pack.unwrap_or(u32::MAX), PACK, "Pack files")?;
    Ok(())
}

fn submenus(menu: HMENU) -> Vec<HMENU> {
    let count = unsafe { GetMenuItemCount(Some(menu)) };
    (0..count).map(|pos| unsafe { GetSubMenu(menu, pos) }).filter(|submenu| !submenu.0.is_null()).collect()
}

/// The position after the item whose verb (given its offset) passes.
fn position(menu: HMENU, mut wanted: impl FnMut(u32) -> bool) -> Option<u32> {
    let count = unsafe { GetMenuItemCount(Some(menu)) };
    (0..count)
        .find(|&pos| match unsafe { GetMenuItemID(menu, pos) } {
            id @ SHELL_FIRST..=SHELL_LAST => wanted(id - SHELL_FIRST),
            _ => false,
        })
        .map(|pos| pos as u32 + 1)
}

/// A command's language-independent name: "open", "copy", "delete".
fn verb(menu: &IContextMenu, offset: u32) -> Option<String> {
    let mut name = [0u16; 64];
    let buffer = PSTR(name.as_mut_ptr().cast());
    unsafe { menu.GetCommandString(offset as usize, GCS_VERBW, None, buffer, name.len() as u32) }.ok()?;
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..len]))
}

/// The handlers of the menu being shown, which draw and fill their
/// submenus (Send to, New): the items', and New's with its submenu.
struct Shown {
    items: IContextMenu,
    new: Option<(IContextMenu, HMENU)>,
}

thread_local! {
    static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
}

/// Whose a menu message is: New's for its submenu and its items, else the items'.
fn handler(message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<IContextMenu> {
    SHOWN.with(|slot| {
        let slot = slot.borrow();
        let shown = slot.as_ref()?;
        let Some((new, submenu)) = &shown.new else {
            return Some(shown.items.clone());
        };
        let new_ones = match message {
            WM_INITMENUPOPUP => wparam.0 as *mut c_void == submenu.0,
            WM_MENUCHAR => lparam.0 as *mut c_void == submenu.0,
            // MEASUREITEMSTRUCT and DRAWITEMSTRUCT both start with CtlType, CtlID, itemID.
            _ => (NEW_FIRST..=NEW_LAST).contains(&unsafe { *(lparam.0 as *const u32).add(2) }),
        };
        Some(if new_ones { new.clone() } else { shown.items.clone() })
    })
}

/// Light or dark as the app, by uxtheme's unnamed exports: menus have no documented dark mode.
unsafe fn theme(hwnd: HWND, dark: bool) {
    let Ok(uxtheme) = (unsafe { LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }) else {
        return;
    };
    let export = |ordinal: usize| unsafe { GetProcAddress(uxtheme, PCSTR(ordinal as *const u8)) };
    if let Some(allow_dark) = export(133) {
        let allow_dark: unsafe extern "system" fn(HWND, i32) -> i32 = unsafe { std::mem::transmute(allow_dark) };
        unsafe { allow_dark(hwnd, dark as i32) };
    }
    if let Some(set_mode) = export(135) {
        let set_mode: unsafe extern "system" fn(i32) -> i32 = unsafe { std::mem::transmute(set_mode) };
        unsafe { set_mode(if dark { 2 } else { 3 }) };
    }
    if let Some(flush) = export(136) {
        let flush: unsafe extern "system" fn() = unsafe { std::mem::transmute(flush) };
        unsafe { flush() };
    }
}

/// A hidden window of this thread owning the menu, under the app's window:
/// a menu's messages go to a window of the thread showing it.
struct Helper(HWND);

impl Helper {
    fn new(owner: HWND) -> Result<Self> {
        let class = w!("SparkCommanderShellMenu");
        let instance = unsafe { GetModuleHandleW(None)? };
        let wndclass = WNDCLASSW {
            lpfnWndProc: Some(forward),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // Fails once registered, which is fine.
        unsafe { RegisterClassW(&wndclass) };
        let style = WINDOW_EX_STYLE::default();
        let hwnd = unsafe { CreateWindowExW(style, class, w!(""), WS_POPUP, 0, 0, 0, 0, Some(owner), None, Some(instance.into()), None)? };
        Ok(Self(hwnd))
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

unsafe extern "system" fn forward(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if matches!(message, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let mut result = LRESULT(0);
        let handled = handler(message, wparam, lparam).is_some_and(|menu| match menu.cast::<IContextMenu3>() {
            Ok(menu) => unsafe { menu.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }.is_ok(),
            Err(_) => menu
                .cast::<IContextMenu2>()
                .is_ok_and(|menu| unsafe { menu.HandleMenuMsg(message, wparam, lparam) }.is_ok()),
        });
        if handled {
            return match message {
                WM_INITMENUPOPUP => LRESULT(0),
                WM_DRAWITEM | WM_MEASUREITEM => LRESULT(1),
                _ => result,
            };
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
