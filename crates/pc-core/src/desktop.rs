//! Linux and the BSDs: what the desktop's file managers put on a file's
//! right-click menu, read from the files every desktop shares: the
//! applications that open a type (shared-mime-info, .desktop files,
//! mimeapps.list), the desktop's own actions (KDE's service menus,
//! Nautilus's, Nemo's and Caja's scripts, Thunar's custom actions, Nemo's
//! actions, FileManager-Actions' for Caja), the templates for new
//! files, the terminal, and the desktop's app chooser.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use crate::associations::wildcard_match;
use crate::clipboard::file_uri;

/// An application that opens the chosen files.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub icon: Option<PathBuf>,
    exec: String,
    terminal: bool,
    file: PathBuf,
}

impl App {
    pub fn open(&self, paths: &[PathBuf], folder: &Path) -> io::Result<()> {
        launch(&self.exec, self.terminal, &self.name, &self.file, paths, folder)
    }
}

/// One of the desktop's own commands for the chosen files.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub name: String,
    pub icon: Option<PathBuf>,
    /// The submenu it goes in, at its level.
    pub submenu: Option<String>,
    /// On the menu itself, rather than in its Actions submenu.
    pub top_level: bool,
    run: Run,
}

#[derive(Clone, Debug, PartialEq)]
enum Run {
    /// A desktop entry's Exec line (KDE's service menus).
    Exec { line: String, terminal: bool, file: PathBuf },
    /// A Nautilus, Nemo or Caja script, with that file manager's variables (`NEMO`).
    Script(PathBuf, &'static str),
    /// A Thunar custom action's command line.
    Thunar(String),
    /// A Nemo action's: its folder for `<program>`, for the folder's menu or the files'.
    Nemo { line: String, dir: PathBuf, terminal: bool, separator: Option<String>, background: bool },
    /// A FileManager-Actions profile's, and its working folder.
    FileManager { line: String, terminal: bool, path: Option<String> },
}

impl Action {
    /// On `paths`, from `folder`; the folder itself when the menu is the folder's.
    pub fn run(&self, paths: &[PathBuf], folder: &Path) -> io::Result<()> {
        match &self.run {
            Run::Exec { line, terminal, file } => launch(line, *terminal, &self.name, file, paths, folder),
            Run::Script(script, manager) => {
                let joined = |each: &dyn Fn(&PathBuf) -> String| paths.iter().map(|p| each(p) + "\n").collect::<String>();
                // As Nautilus: names relative to the folder, the rest in its variables.
                let names = paths.iter().map(|path| path.strip_prefix(folder).ok().filter(|p| !p.as_os_str().is_empty()).unwrap_or(path.as_path()));
                spawn(
                    Command::new(script)
                        .args(names)
                        .env(format!("{manager}_SCRIPT_SELECTED_FILE_PATHS"), joined(&|p| p.to_string_lossy().into_owned()))
                        .env(format!("{manager}_SCRIPT_SELECTED_URIS"), joined(&|p| file_uri(p)))
                        .env(format!("{manager}_SCRIPT_CURRENT_URI"), file_uri(folder))
                        .current_dir(folder),
                )
            }
            Run::Nemo { line, dir, terminal, separator, background } => {
                // `<program …>`: a program of the action's folder.
                let line = match line.strip_prefix('<').and_then(|l| l.strip_suffix('>')) {
                    Some(own) => format!("{}/{own}", dir.display()),
                    None => line.clone(),
                };
                let chosen = if *background { &[][..] } else { paths };
                let argv = command_line(&line, |code| nemo_value(code, chosen, folder, separator.as_deref()));
                start(argv, *terminal, &self.name, folder)
            }
            Run::FileManager { line, terminal, path } => {
                // A singular code first (%f, not %F) runs it once per file.
                let first = codes_in(line).find(|c| "bdfmuwxBDFMUWX".contains(*c));
                if first.is_some_and(|c| c.is_lowercase()) && paths.len() > 1 {
                    return paths.iter().try_for_each(|p| self.run(std::slice::from_ref(p), folder));
                }
                let argv = command_line(line, |code| fma_value(code, paths));
                let dir = path.as_deref().map(|p| PathBuf::from(codes(p, |code| fma_value(code, paths).map(|v| v.join(" ")))));
                start(argv, *terminal, &self.name, dir.as_deref().filter(|d| d.is_dir()).unwrap_or(folder))
            }
            Run::Thunar(line) => {
                let first = paths.first().map(PathBuf::as_path).unwrap_or(folder);
                let parent = |p: &Path| p.parent().unwrap_or(p).to_path_buf();
                let name = |p: &Path| p.file_name().map(PathBuf::from).unwrap_or_default();
                let all = |each: &dyn Fn(&Path) -> String| paths.iter().map(|p| each(p)).collect::<Vec<_>>().join(" ");
                let quoted = |p: PathBuf| quote(&p.to_string_lossy());
                let expanded = codes(line, |code| {
                    Some(match code {
                        'f' => quoted(first.to_path_buf()),
                        'F' => all(&|p| quoted(p.to_path_buf())),
                        'u' => quote(&file_uri(first)),
                        'U' => all(&|p| quote(&file_uri(p))),
                        'd' => quoted(parent(first)),
                        'D' => all(&|p| quoted(parent(p))),
                        'n' => quoted(name(first)),
                        'N' => all(&|p| quoted(name(p))),
                        '%' => "%".to_owned(),
                        _ => return None,
                    })
                });
                spawn(Command::new("sh").arg("-c").arg(expanded).current_dir(folder))
            }
        }
    }
}

/// A chosen file or folder, with its type and the types it is one of.
struct Item {
    name: String,
    is_dir: bool,
    types: Vec<String>,
}

impl Item {
    fn new(path: &Path, is_dir: bool) -> Self {
        let db = mime_db();
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Self { types: db.ancestry(&db.type_of(path, &name, is_dir)), name, is_dir }
    }

    /// KDE's types: `all/all`, `all/allfiles`, `image/*`, or one this item is.
    fn is(&self, kind: &str) -> bool {
        match kind {
            "all/all" | "*" | "*/*" => true,
            "all/allfiles" => !self.is_dir,
            _ => match kind.strip_suffix('*') {
                Some(prefix) => self.types.iter().any(|t| t.starts_with(prefix)),
                None => self.types.iter().any(|t| t == kind),
            },
        }
    }
}

/// The applications that open all of `chosen` (path, is a folder), the
/// desktop's default first.
pub fn open_with(chosen: &[(PathBuf, bool)]) -> Vec<App> {
    let items: Vec<Item> = chosen.iter().map(|(path, is_dir)| Item::new(path, *is_dir)).collect();
    let lists = mimeapps_lists();
    let caches: Vec<_> = data_dirs().iter().filter_map(|dir| read_desktop(&dir.join("applications/mimeinfo.cache"))).collect();
    let ids_of = |item: &Item| -> Vec<String> {
        let removed: HashSet<&str> = item
            .types
            .iter()
            .flat_map(|t| lists.iter().filter_map(move |list| list.get("Removed Associations")?.get(t)))
            .flat_map(|ids| split_list(ids))
            .collect();
        let group = |groups: &'_ Groups, group: &str, t: &str| groups.get(group).and_then(|g| g.get(t)).cloned();
        let mut ids = Vec::new();
        // The defaults first, the closest type's first; then what each type is added and cached with.
        for t in &item.types {
            ids.extend(lists.iter().filter_map(|list| group(list, "Default Applications", t)));
        }
        for t in &item.types {
            ids.extend(lists.iter().filter_map(|list| group(list, "Added Associations", t)));
            ids.extend(caches.iter().filter_map(|cache| group(cache, "MIME Cache", t)));
        }
        let mut seen = HashSet::new();
        ids.iter().flat_map(|ids| split_list(ids)).filter(|id| !removed.contains(id) && seen.insert(*id)).map(str::to_owned).collect()
    };
    let mut lists_of = items.iter().map(ids_of);
    let Some(mut ids) = lists_of.next() else {
        return Vec::new();
    };
    for other in lists_of {
        ids.retain(|id| other.contains(id));
    }
    let index = desktop_index();
    ids.iter().filter_map(|id| app(index.get(id.as_str())?)).collect()
}

/// The desktop's own actions for `chosen` (path, is a folder), or for the
/// folder shown when `background`: KDE's service menus, GNOME's Nautilus
/// scripts, Xfce's Thunar actions, Cinnamon's Nemo actions and scripts,
/// MATE's Caja scripts and FileManager-Actions, by the desktop running;
/// all of them on others.
pub fn actions(chosen: &[(PathBuf, bool)], background: bool) -> Vec<Action> {
    let items: Vec<Item> = chosen.iter().map(|(path, is_dir)| Item::new(path, *is_dir)).collect();
    let folder = match (background, chosen.first()) {
        (true, Some((path, _))) => path.clone(),
        (_, first) => first.and_then(|(path, _)| path.parent()).map(Path::to_path_buf).unwrap_or_default(),
    };
    let desktops = desktops();
    let on = |names: &[&str]| desktops.iter().any(|d| names.contains(&d.as_str()));
    let known = on(&["kde", "gnome", "xfce", "x-cinnamon", "mate"]);
    let mut found = Vec::new();
    if on(&["kde"]) || !known {
        service_menus(&items, &mut found);
    }
    if on(&["gnome"]) || !known {
        scripts(dirs::data_dir().map(|d| d.join("nautilus/scripts")), "NAUTILUS", &mut found);
    }
    if on(&["xfce"]) || !known {
        thunar_actions(&items, &mut found);
    }
    if on(&["x-cinnamon"]) || !known {
        nemo_actions(&items, background, &folder, &mut found);
        scripts(dirs::data_dir().map(|d| d.join("nemo/scripts")), "NEMO", &mut found);
    }
    if on(&["mate"]) || !known {
        file_manager_actions(&items, background, &folder, &mut found);
        scripts(dirs::config_dir().map(|d| d.join("caja/scripts")), "CAJA", &mut found);
    }
    found
}

/// The files in the user's Templates folder (`XDG_TEMPLATES_DIR`), which
/// GNOME's, KDE's and Xfce's "new document" menus offer.
pub fn templates() -> Vec<PathBuf> {
    let Some(dir) = dirs::template_dir() else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.path().is_file())
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

/// The desktop's terminal, in `dir`.
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    let (program, folder_flag, _) = terminal().ok_or_else(|| io::Error::other("no terminal found"))?;
    let mut command = Command::new(&program);
    match folder_flag.strip_suffix('=') {
        Some(flag) => command.arg(format!("{flag}={}", dir.display())),
        None if !folder_flag.is_empty() => command.arg(folder_flag).arg(dir),
        None => &mut command,
    };
    spawn(command.current_dir(dir))
}

/// The desktop's own "Open With" dialog (the portal's), which then opens
/// the file in what is picked there.
pub fn choose_app(path: &Path) -> io::Result<()> {
    use std::os::fd::AsFd;
    use zbus::zvariant::{Fd, Value};
    let file = fs::File::open(path)?;
    let bus = zbus::blocking::Connection::session().map_err(io::Error::other)?;
    let options: HashMap<&str, Value> = HashMap::from([("ask", Value::from(true))]);
    let portal = "org.freedesktop.portal.Desktop";
    let call = ("", Fd::from(file.as_fd()), options);
    bus.call_method(Some(portal), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.OpenURI"), "OpenFile", &call)
        .map_err(|err| io::Error::other(format!("the desktop has no app chooser ({err})")))?;
    Ok(())
}

/// The desktop's file manager, with these selected in their folder.
pub fn reveal(paths: &[PathBuf]) -> io::Result<()> {
    let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
    let bus = zbus::blocking::Connection::session().map_err(io::Error::other)?;
    let name = "org.freedesktop.FileManager1";
    bus.call_method(Some(name), "/org/freedesktop/FileManager1", Some(name), "ShowItems", &(uris, ""))
        .map_err(io::Error::other)?;
    Ok(())
}

// --- Launching -------------------------------------------------------------

/// A desktop entry's Exec line on `paths`, as the spec has it: `%f` and
/// `%u` run it once per file, `%F` and `%U` once for all; without either
/// the files go last, as KDE does. A line using the shell runs in `sh`.
fn launch(line: &str, terminal: bool, name: &str, file: &Path, paths: &[PathBuf], folder: &Path) -> io::Result<()> {
    let has = |code: &str| line.contains(code);
    if !has("%F") && !has("%U") && (has("%f") || has("%u")) && paths.len() > 1 {
        return paths.iter().try_for_each(|path| launch(line, terminal, name, file, std::slice::from_ref(path), folder));
    }
    let takes_files = ["%f", "%F", "%u", "%U"].iter().any(|code| has(code));
    let line = if takes_files { line.to_owned() } else { format!("{line} %F") };
    let argv = command_line(&line, |code| {
        Some(match code {
            'F' => paths.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
            'U' => paths.iter().map(|p| file_uri(p)).collect(),
            'f' => paths.first().map(|p| p.to_string_lossy().into_owned()).into_iter().collect(),
            'u' => paths.first().map(|p| file_uri(p)).into_iter().collect(),
            'c' => vec![name.to_owned()],
            'k' => vec![file.to_string_lossy().into_owned()],
            '%' => vec!["%".to_owned()],
            // %i and the deprecated ones go.
            _ => Vec::new(),
        })
    });
    start(argv, terminal, name, folder)
}

/// A command line's arguments, its `%x` codes replaced by `value`'s: a
/// code alone gives an argument per value, one inside an argument its
/// values joined. A line using the shell runs in `sh`, each value quoted.
fn command_line(line: &str, value: impl Fn(char) -> Option<Vec<String>>) -> Vec<String> {
    let Some(args) = split_exec(line) else {
        let script = codes(line, |code| value(code).map(|v| v.iter().map(|v| quote(v)).collect::<Vec<_>>().join(" ")));
        return vec!["sh".into(), "-c".into(), script];
    };
    let lone = |arg: &str| match arg.as_bytes() {
        [b'%', code] if *code != b'%' => value(*code as char),
        _ => None,
    };
    args.iter().flat_map(|arg| lone(arg).unwrap_or_else(|| vec![codes(arg, |code| value(code).map(|v| v.join(" ")))])).collect()
}

/// The `%x` codes a line has, in order.
fn codes_in(line: &str) -> impl Iterator<Item = char> + '_ {
    line.split('%').skip(1).filter_map(|after| after.chars().next())
}

/// Runs `argv` from `folder`, in the desktop's terminal when asked.
fn start(mut argv: Vec<String>, terminal: bool, name: &str, folder: &Path) -> io::Result<()> {
    if argv.is_empty() {
        return Err(io::Error::other(format!("{name}: nothing to run")));
    }
    if terminal {
        let (program, _, run_flag) = terminal_or_err()?;
        let mut wrapped = vec![program.to_string_lossy().into_owned()];
        wrapped.extend(run_flag.map(str::to_owned));
        wrapped.append(&mut argv);
        argv = wrapped;
    }
    spawn(Command::new(&argv[0]).args(&argv[1..]).current_dir(folder))
}

/// Runs on its own: out of the app's process group, reaped when it ends.
fn spawn(command: &mut Command) -> io::Result<()> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// `%x` codes replaced by `value`; a code it has no value for stays as written.
fn codes(text: &str, mut value: impl FnMut(char) -> Option<String>) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('%', Some(code)) => {
                chars.next();
                match value(code) {
                    Some(v) => out.push_str(&v),
                    None => out.extend(['%', code]),
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The shell's single quotes.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// An Exec line's arguments by the Desktop Entry spec's quoting; `None`
/// when it needs the shell (redirections, pipes, variables).
fn split_exec(line: &str) -> Option<Vec<String>> {
    let (mut args, mut arg, mut quoted, mut started) = (Vec::new(), String::new(), false, false);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => arg.push(chars.next()?),
            c if c.is_whitespace() && !quoted => {
                if started || !arg.is_empty() {
                    args.push(std::mem::take(&mut arg));
                    started = false;
                }
            }
            '>' | '<' | '|' | ';' | '&' | '`' | '$' | '(' | ')' if !quoted => return None,
            c => arg.push(c),
        }
    }
    if quoted {
        return None;
    }
    if started || !arg.is_empty() {
        args.push(arg);
    }
    Some(args)
}

/// The desktop's terminal: its name, how it is told a folder (`--flag=`
/// or `--flag`), and how a command is given to it.
fn terminal() -> Option<(OsString, &'static str, Option<&'static str>)> {
    const KNOWN: &[(&str, &str, Option<&str>)] = &[
        ("konsole", "--workdir", Some("-e")),
        ("ptyxis", "--working-directory=", Some("--")),
        ("gnome-terminal", "--working-directory=", Some("--")),
        ("kgx", "--working-directory=", Some("-e")),
        ("xfce4-terminal", "--working-directory=", Some("-x")),
        ("mate-terminal", "--working-directory=", Some("-x")),
        ("tilix", "--working-directory=", Some("-e")),
        ("alacritty", "--working-directory", Some("-e")),
        ("kitty", "--directory", None),
        ("foot", "--working-directory=", None),
        ("x-terminal-emulator", "", Some("-e")),
        ("xterm", "", Some("-e")),
    ];
    let desktops = desktops();
    let on = |name: &str| desktops.iter().any(|d| d == name);
    // The desktop's own first: KDE's, Cinnamon's and MATE's setting, Xfce's and GNOME's usual ones.
    let mut wanted: Vec<String> = std::env::var("TERMINAL").into_iter().collect();
    if on("kde") {
        wanted.push(kde_setting("General", "TerminalApplication").unwrap_or_else(|| "konsole".into()));
    }
    if on("xfce") {
        wanted.push("xfce4-terminal".into());
    }
    if on("gnome") {
        wanted.extend(["ptyxis", "gnome-terminal", "kgx"].map(String::from));
    }
    if on("x-cinnamon") {
        wanted.extend(crate::theme::gsetting("org.cinnamon.desktop.default-applications.terminal", "exec"));
    }
    if on("mate") {
        wanted.extend(crate::theme::gsetting("org.mate.applications-terminal", "exec"));
    }
    wanted.extend(KNOWN.iter().map(|(name, ..)| name.to_string()));
    wanted.into_iter().find_map(|name| {
        let program = on_path(name.split_whitespace().next()?)?;
        let base = program.file_name()?.to_string_lossy().into_owned();
        let (folder, run) = KNOWN.iter().find(|(known, ..)| *known == base).map_or(("", Some("-e")), |(_, f, r)| (*f, *r));
        Some((program.into_os_string(), folder, run))
    })
}

fn terminal_or_err() -> io::Result<(OsString, &'static str, Option<&'static str>)> {
    terminal().ok_or_else(|| io::Error::other("no terminal found"))
}

/// A program's path: as given when absolute, else found on `PATH`.
fn on_path(program: &str) -> Option<PathBuf> {
    let runnable = |path: &Path| path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        return Some(PathBuf::from(program)).filter(|p| runnable(p));
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(program)).find(|p| runnable(p))
}

// --- Applications ----------------------------------------------------------

/// An application's entry, unless it hides, belongs to another desktop or
/// its program is missing.
fn app(file: &Path) -> Option<App> {
    let groups = read_desktop(file)?;
    let entry = groups.get("Desktop Entry")?;
    let flag = |key: &str| entry.get(key).is_some_and(|v| v == "true");
    if entry.get("Type").is_some_and(|t| t != "Application") || flag("Hidden") || flag("NoDisplay") || !shown_here(entry) {
        return None;
    }
    if entry.get("TryExec").is_some_and(|program| on_path(program).is_none()) {
        return None;
    }
    Some(App {
        name: entry.get("Name")?.clone(),
        icon: entry.get("Icon").and_then(|name| icon(name)),
        exec: entry.get("Exec")?.clone(),
        terminal: flag("Terminal"),
        file: file.to_owned(),
    })
}

fn shown_here(entry: &HashMap<String, String>) -> bool {
    let desktops = desktops();
    let any = |key: &str| entry.get(key).map(|v| split_list(v).any(|d| desktops.contains(&d.to_lowercase())));
    any("OnlyShowIn").unwrap_or(true) && !any("NotShowIn").unwrap_or(false)
}

/// Desktop file ids (`org.kde.kate.desktop`) and their files, the first
/// data folder's winning.
fn desktop_index() -> HashMap<String, PathBuf> {
    fn walk(root: &Path, dir: &Path, depth: u8, index: &mut HashMap<String, PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth > 0 {
                walk(root, &path, depth - 1, index);
            } else if path.extension().is_some_and(|e| e == "desktop") {
                let id = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('/', "-");
                index.entry(id).or_insert(path);
            }
        }
    }
    let mut index = HashMap::new();
    for dir in data_dirs() {
        let root = dir.join("applications");
        walk(&root, &root, 3, &mut index);
    }
    index
}

/// mimeapps.list files, the strongest first, each desktop's own before the common one.
fn mimeapps_lists() -> Vec<Groups> {
    let names: Vec<String> = desktops().iter().map(|d| format!("{d}-mimeapps.list")).chain(["mimeapps.list".into()]).collect();
    let config = config_dirs();
    let data = data_dirs().into_iter().map(|dir| dir.join("applications"));
    config
        .into_iter()
        .chain(data)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .filter_map(|path| read_desktop(&path))
        .collect()
}

// --- Desktop actions -------------------------------------------------------

/// KDE's service menus (Dolphin's Actions), in KF6's folder and KF5's.
fn service_menus(items: &[Item], found: &mut Vec<Action>) {
    let home = dirs::data_dir();
    let mut seen = HashSet::new();
    for dir in data_dirs() {
        for sub in ["kio/servicemenus", "kservices5/ServiceMenus"] {
            for file in desktop_files(&dir.join(sub), 2) {
                // The user's own file of a name hides the system's.
                if !seen.insert(file.file_name().map(ToOwned::to_owned)) {
                    continue;
                }
                // KDE runs a user's own only once made executable, as downloaded ones must be.
                let own = home.as_ref().is_some_and(|home| file.starts_with(home));
                if own && !file.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
                    continue;
                }
                let Some(groups) = read_desktop(&file) else { continue };
                let Some(entry) = groups.get("Desktop Entry") else { continue };
                if !service_applies(entry, items) {
                    continue;
                }
                let top_level = entry.get("X-KDE-Priority").is_some_and(|p| p == "TopLevel");
                let submenu = entry.get("X-KDE-Submenu").filter(|s| !s.is_empty()).cloned();
                for id in entry.get("Actions").map(|a| split_list(a).collect::<Vec<_>>()).unwrap_or_default() {
                    let Some(action) = groups.get(&format!("Desktop Action {id}")) else { continue };
                    let (Some(name), Some(line)) = (action.get("Name"), action.get("Exec")) else { continue };
                    if action.get("NoDisplay").is_some_and(|v| v == "true") {
                        continue;
                    }
                    found.push(Action {
                        name: name.clone(),
                        icon: action.get("Icon").and_then(|name| icon(name)),
                        submenu: submenu.clone(),
                        top_level,
                        run: Run::Exec { line: line.clone(), terminal: action.get("Terminal").is_some_and(|v| v == "true"), file: file.clone() },
                    });
                }
            }
        }
    }
}

/// A service menu's conditions: the types, how many, local files, its program, a running service.
fn service_applies(entry: &HashMap<String, String>, items: &[Item]) -> bool {
    let list = |key: &str| entry.get(key).map(|v| split_list(v).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    // For local files: `file` among the protocols named, and not `!file`.
    let protocols = [list("X-KDE-Protocol"), list("X-KDE-Protocols")].concat();
    let named: Vec<&String> = protocols.iter().filter(|p| !p.starts_with('!')).collect();
    if protocols.iter().any(|p| p == "!file") || !named.is_empty() && !named.iter().any(|p| *p == "file") {
        return false;
    }
    if entry.get("TryExec").is_some_and(|program| on_path(program).is_none()) {
        return false;
    }
    // shortcut: a menu shown only when a D-Bus call says so is left out; call it when one matters.
    if entry.contains_key("X-KDE-ShowIfDBusCall") {
        return false;
    }
    if let Some(service) = entry.get("X-KDE-ShowIfRunning")
        && !bus_name_taken(service)
    {
        return false;
    }
    let count = items.len();
    let number = |key: &str| entry.get(key).and_then(|v| v.trim().parse::<usize>().ok());
    let required = list("X-KDE-RequiredNumberOfUrls");
    if !required.is_empty() && !required.iter().any(|n| n.parse::<usize>() == Ok(count)) {
        return false;
    }
    if number("X-KDE-MinNumberOfUrls").is_some_and(|min| count < min) || number("X-KDE-MaxNumberOfUrls").is_some_and(|max| count > max) {
        return false;
    }
    let mut types = list("MimeType");
    for key in ["ServiceTypes", "X-KDE-ServiceTypes"] {
        types.extend(list(key).into_iter().filter(|t| !t.starts_with("KonqPopupMenu")));
    }
    let excluded = [list("ExcludeServiceTypes"), list("X-KDE-ExcludeServiceTypes")].concat();
    !types.is_empty()
        && items.iter().all(|item| types.iter().any(|t| item.is(t)))
        && !items.iter().any(|item| excluded.iter().any(|t| item.is(t)))
}

fn bus_name_taken(name: &str) -> bool {
    let Ok(bus) = zbus::blocking::Connection::session() else { return false };
    let dbus = "org.freedesktop.DBus";
    bus.call_method(Some(dbus), "/org/freedesktop/DBus", Some(dbus), "NameHasOwner", &(name,))
        .ok()
        .and_then(|reply| reply.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// Nautilus's, Nemo's or Caja's scripts: every program in its scripts
/// folder, a subfolder's under its name.
fn scripts(root: Option<PathBuf>, manager: &'static str, found: &mut Vec<Action>) {
    let Some(root) = root else { return };
    let mut scripts: Vec<(PathBuf, String)> = Vec::new();
    for entry in fs::read_dir(&root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            let folder = entry.file_name().to_string_lossy().into_owned();
            scripts.extend(fs::read_dir(&path).into_iter().flatten().flatten().map(|e| (e.path(), format!("{folder} › "))));
        } else {
            scripts.push((path, String::new()));
        }
    }
    scripts.retain(|(path, _)| {
        let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
        !hidden && path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    });
    scripts.sort();
    found.extend(scripts.into_iter().map(|(path, folder)| Action {
        name: folder + &path.file_name().unwrap_or_default().to_string_lossy(),
        icon: None,
        submenu: Some("Scripts".into()),
        top_level: true,
        run: Run::Script(path, manager),
    }));
}

/// Thunar's custom actions (uca.xml): the user's, else the system's defaults.
fn thunar_actions(items: &[Item], found: &mut Vec<Action>) {
    let Some(text) = config_dirs().iter().find_map(|dir| fs::read_to_string(dir.join("Thunar/uca.xml")).ok()) else {
        return;
    };
    let Ok(doc) = roxmltree::Document::parse(&text) else { return };
    let langs = languages();
    for action in doc.root_element().children().filter(|n| n.has_tag_name("action")) {
        let tagged = |tag: &'static str| tagged(action, tag);
        let text_of = |tag: &'static str| tagged(tag).next().and_then(|n| n.text()).map(str::trim).filter(|t| !t.is_empty());
        // The name in the user's language, else the plain one.
        let localized = langs.iter().find_map(|wanted| tagged("name").find(|n| lang(n) == Some(wanted.as_str())));
        let name = localized.or_else(|| tagged("name").find(|n| lang(n).is_none())).and_then(|n| n.text());
        let (Some(name), Some(command)) = (name, text_of("command")) else { continue };
        let patterns = text_of("patterns").unwrap_or("*");
        let has = |tag: &'static str| tagged(tag).next().is_some();
        let kind_fits = |item: &Item| match item.is_dir {
            true => has("directories"),
            false => {
                let of = |prefix: &str| item.types.iter().any(|t| t.starts_with(prefix));
                match () {
                    _ if of("audio/") => has("audio-files"),
                    _ if of("image/") => has("image-files"),
                    _ if of("video/") => has("video-files"),
                    _ if of("text/") => has("text-files"),
                    _ => has("other-files"),
                }
            }
        };
        let fits = |item: &Item| kind_fits(item) && patterns.split(';').any(|p| wildcard_match(p.trim(), &item.name, false));
        if !in_range(text_of("range").unwrap_or("*"), items.len()) || !items.iter().all(fits) {
            continue;
        }
        found.push(Action {
            name: name.trim().to_owned(),
            icon: text_of("icon").and_then(icon),
            submenu: text_of("submenu").map(str::to_owned),
            top_level: true,
            run: Run::Thunar(command.to_owned()),
        });
    }
}

fn tagged<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &'static str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> {
    node.children().filter(move |n| n.has_tag_name(tag))
}

fn lang<'a>(node: &roxmltree::Node<'a, '_>) -> Option<&'a str> {
    node.attribute(("http://www.w3.org/XML/1998/namespace", "lang"))
}

/// Cinnamon's Nemo actions (`.nemo_action`), the user's hiding the
/// system's of a name, in their files' order, as Nemo lists them.
fn nemo_actions(items: &[Item], background: bool, folder: &Path, found: &mut Vec<Action>) {
    let mut files: Vec<(OsString, PathBuf)> = Vec::new();
    for dir in data_dirs() {
        for entry in fs::read_dir(dir.join("nemo/actions")).into_iter().flatten().flatten() {
            let name = entry.file_name();
            if entry.path().extension().is_some_and(|e| e == "nemo_action") && !files.iter().any(|(known, _)| *known == name) {
                files.push((name, entry.path()));
            }
        }
    }
    files.sort();
    // On the folder's menu nothing is selected.
    let chosen = if background { &[][..] } else { items };
    let paths: Vec<PathBuf> = chosen.iter().map(|item| folder.join(&item.name)).collect();
    for (_, file) in files {
        let dir = file.parent().unwrap_or(folder).to_owned();
        let Some(groups) = read_desktop(&file) else { continue };
        let Some(entry) = groups.get("Nemo Action") else { continue };
        let (Some(name), Some(line)) = (entry.get("Name"), entry.get("Exec")) else { continue };
        if entry.get("Active").is_some_and(|v| v == "false") || !nemo_applies(entry, chosen, folder, &dir) {
            continue;
        }
        // Its codes filled in, its mnemonic gone: "Mount %N", "Change _Background".
        let label = codes(name, |code| nemo_value(code, &paths, folder, None).map(|v| v.join(" ")));
        found.push(Action {
            name: label.replace("__", "\0").replace('_', "").replace('\0', "_"),
            icon: entry.get("Icon-Name").and_then(|name| icon(name)),
            submenu: None,
            top_level: true,
            run: Run::Nemo {
                line: line.clone(),
                dir,
                terminal: entry.get("Terminal").is_some_and(|v| v == "true"),
                separator: entry.get("Separator").cloned(),
                background,
            },
        });
    }
}

/// A Nemo action's conditions: how many, the types, programs, the scheme and Nemo's others.
fn nemo_applies(entry: &HashMap<String, String>, items: &[Item], folder: &Path, dir: &Path) -> bool {
    let list = |key: &str| entry.get(key).map(|v| split_list(v).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    let count = items.len();
    let selection = entry.get("Selection").map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let count_fits = match selection.as_str() {
        "s" => count == 1,
        "m" => count > 1,
        "any" => true,
        "notnone" => count > 0,
        "none" => count == 0,
        number => number.parse() == Ok(count),
    };
    let present = |program: &str| match program.starts_with('/') {
        true => Path::new(program).exists(),
        false => on_path(program).is_some(),
    };
    // `!program`: one that must be missing.
    let programs = list("Dependencies").iter().all(|program| match program.strip_prefix('!') {
        Some(program) => !present(program),
        None => present(program),
    });
    let conditions = list("Conditions").iter().all(|condition| {
        let mut words = condition.split_whitespace();
        match words.next() {
            Some("desktop") => dirs::desktop_dir().is_some_and(|desktop| desktop == folder),
            Some("dbus") => words.next().is_some_and(bus_name_taken),
            Some("gsettings") => gsettings_condition(&words.collect::<Vec<_>>()),
            Some("exec") => {
                let program = condition["exec".len()..].trim();
                let program = match program.strip_prefix('<').and_then(|p| p.strip_suffix('>')) {
                    Some(own) => format!("{}/{own}", dir.display()),
                    None => program.to_owned(),
                };
                let quiet = |command: &mut Command| command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
                quiet(Command::new("sh").arg("-c").arg(program).current_dir(folder)).is_ok_and(|s| s.success())
            }
            // shortcut: "removable" (on a removable drive) is never met; ask udisks when an action needs it.
            _ => false,
        }
    });
    let extensions: Vec<String> = list("Extensions").iter().map(|e| e.to_lowercase()).collect();
    let types = list("Mimetypes");
    let type_fits = |item: &Item| {
        let name = item.name.to_lowercase();
        let by_name = extensions.iter().any(|ext| match ext.as_str() {
            "any" => true,
            "dir" => item.is_dir,
            "nodirs" => !item.is_dir,
            "none" => !item.is_dir && !name.contains('.'),
            ext => !item.is_dir && name.ends_with(&format!(".{ext}")),
        });
        by_name || types.iter().any(|t| item.is(t))
    };
    count_fits
        && programs
        && entry.get("UriScheme").is_none_or(|scheme| scheme == "file")
        && conditions
        && items.iter().all(type_fits)
}

/// Nemo's `gsettings <schema> <key>` (a true boolean) and `gsettings <schema> <key> <type> <eq|ne|gt|lt> <value>`.
fn gsettings_condition(words: &[&str]) -> bool {
    use std::cmp::Ordering::*;
    match words {
        [schema, key] => crate::theme::gsetting(schema, key).as_deref() == Some("true"),
        [schema, key, _, op, wanted] => {
            let Some(value) = crate::theme::gsetting(schema, key) else { return false };
            let wanted = wanted.trim_matches('\'');
            let order = match (value.parse::<f64>(), wanted.parse::<f64>()) {
                (Ok(a), Ok(b)) => a.partial_cmp(&b),
                _ => Some(value.as_str().cmp(wanted)),
            };
            match *op {
                "eq" => order == Some(Equal),
                "ne" => order != Some(Equal),
                "gt" => order == Some(Greater),
                "lt" => order == Some(Less),
                _ => false,
            }
        }
        _ => false,
    }
}

/// A Nemo code's values for the chosen `paths` in `folder`; Separator joins a list into one.
fn nemo_value(code: char, paths: &[PathBuf], folder: &Path, separator: Option<&str>) -> Option<Vec<String>> {
    let first = paths.first().map(PathBuf::as_path).unwrap_or(folder);
    let list = |values: Vec<String>| match separator {
        Some(separator) => vec![values.join(separator)],
        None => values,
    };
    Some(match code {
        'F' => list(paths.iter().map(|p| p.to_string_lossy().into_owned()).collect()),
        'U' => list(paths.iter().map(|p| file_uri(p)).collect()),
        'P' => vec![folder.to_string_lossy().into_owned()],
        'f' | 'N' => vec![base_name(first)],
        'p' => vec![base_name(folder)],
        'e' => vec![first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()],
        '%' => vec!["%".into()],
        // shortcut: %D (the drive's device) and %X (the window) give nothing; fill them when an action needs them.
        'D' | 'X' => Vec::new(),
        _ => return None,
    })
}

fn base_name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

/// FileManager-Actions (`file-manager/actions`, Caja's caja-actions): each
/// action by its first profile that fits, a menu's in its submenu, by name.
fn file_manager_actions(items: &[Item], background: bool, folder: &Path, found: &mut Vec<Action>) {
    let mut entries: Vec<(String, Groups)> = Vec::new();
    for dir in data_dirs() {
        for file in desktop_files(&dir.join("file-manager/actions"), 0) {
            let id = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if !entries.iter().any(|(known, _)| *known == id)
                && let Some(groups) = read_desktop(&file)
            {
                entries.push((id, groups));
            }
        }
    }
    let flag = |entry: &HashMap<String, String>, key: &str, default: bool| entry.get(key).map_or(default, |v| v == "true");
    let shown = |entry: &HashMap<String, String>| flag(entry, "Enabled", true) && !flag(entry, "Hidden", false) && fma_applies(entry, items, folder);
    // shortcut: one level of menus; a menu in a menu puts its actions in the outer one's.
    let mut submenus: HashMap<String, String> = HashMap::new();
    for (_, groups) in &entries {
        let Some(entry) = groups.get("Desktop Entry").filter(|e| e.get("Type").is_some_and(|t| t == "Menu") && shown(e)) else { continue };
        let (Some(name), Some(ids)) = (entry.get("Name"), entry.get("ItemsList")) else { continue };
        for id in split_list(ids) {
            submenus.entry(id.to_owned()).or_insert_with(|| name.clone());
        }
    }
    let mut actions = Vec::new();
    for (id, groups) in &entries {
        let Some(entry) = groups.get("Desktop Entry") else { continue };
        let target = match background {
            true => flag(entry, "TargetLocation", false),
            false => flag(entry, "TargetContext", true),
        };
        if entry.get("Type").is_some_and(|t| t != "Action") || !target || !shown(entry) {
            continue;
        }
        let profiles = entry.get("Profiles").map(|p| split_list(p).collect::<Vec<_>>()).unwrap_or_default();
        let profiles = profiles.iter().filter_map(|p| groups.get(&format!("X-Action-Profile {p}")));
        let Some(profile) = profiles.into_iter().find(|p| fma_applies(p, items, folder)) else { continue };
        let (Some(name), Some(line)) = (entry.get("Name"), profile.get("Exec")) else { continue };
        actions.push(Action {
            name: name.clone(),
            icon: entry.get("Icon").and_then(|name| icon(name)),
            submenu: submenus.get(id).cloned(),
            top_level: true,
            run: Run::FileManager {
                line: line.clone(),
                terminal: profile.get("ExecutionMode").is_some_and(|m| m == "Terminal"),
                path: profile.get("Path").cloned(),
            },
        });
    }
    actions.sort_by(|a, b| a.name.cmp(&b.name));
    found.extend(actions);
}

/// An action's or a profile's conditions: the desktop, programs, services,
/// how many, the scheme, the folder, the types and the names.
fn fma_applies(entry: &HashMap<String, String>, items: &[Item], folder: &Path) -> bool {
    let list = |key: &str, default: &str| -> Vec<String> {
        split_list(entry.get(key).map_or(default, String::as_str)).map(str::to_owned).collect()
    };
    // Some positive pattern fits (or there is none), and no `!` one.
    let fits = |patterns: &[String], fit: &dyn Fn(&str) -> bool| {
        let (not, yes): (Vec<&String>, Vec<&String>) = patterns.iter().partition(|p| p.starts_with('!'));
        (yes.is_empty() || yes.iter().any(|p| fit(p))) && !not.iter().any(|p| fit(&p[1..]))
    };
    let count = items.len();
    let count_fits = match entry.get("SelectionCount").map(|c| c.trim()).filter(|c| !c.is_empty()) {
        Some(relation) => {
            let (op, number) = relation.split_at(1);
            match (op, number.trim().parse::<usize>()) {
                ("=", Ok(n)) => count == n,
                ("<", Ok(n)) => count < n,
                (">", Ok(n)) => count > n,
                _ => true,
            }
        }
        None => true,
    };
    let case = entry.get("Matchcase").is_none_or(|v| v != "false");
    let says_true = |command: &str| {
        let output = Command::new("sh").arg("-c").arg(command).stdin(Stdio::null()).stderr(Stdio::null()).output();
        output.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
    };
    // shortcut: Capabilities (Owner, Readable, Writable, Executable) go unchecked; check them when an action needs it.
    shown_here(entry)
        && entry.get("TryExec").is_none_or(|program| on_path(program).is_some())
        && entry.get("ShowIfRegistered").is_none_or(|name| bus_name_taken(name))
        && entry.get("ShowIfRunning").is_none_or(|name| running(name))
        && entry.get("ShowIfTrue").is_none_or(|command| says_true(command))
        && count_fits
        && fits(&list("Schemes", "file"), &|scheme| scheme == "file" || scheme == "*")
        && fits(&list("Folders", "/"), &|dir| folder.starts_with(dir.trim_end_matches('*')))
        && items.iter().all(|item| fits(&list("MimeTypes", "*"), &|t| item.is(t)))
        && items.iter().all(|item| fits(&list("Basenames", "*"), &|name| wildcard_match(name, &item.name, case)))
}

/// A program of this name running, by the name /proc gives it (15 bytes).
fn running(program: &str) -> bool {
    let name = base_name(Path::new(program));
    let short = &name.as_bytes()[..name.len().min(15)];
    let named = |dir: fs::DirEntry| fs::read(dir.path().join("comm")).is_ok_and(|comm| comm.trim_ascii_end() == short);
    fs::read_dir("/proc").into_iter().flatten().flatten().any(named)
}

/// A FileManager-Actions code's values for `paths`: the capital one for all, the small for the first.
fn fma_value(code: char, paths: &[PathBuf]) -> Option<Vec<String>> {
    let of: fn(&Path) -> String = match code.to_ascii_lowercase() {
        'b' => base_name,
        'd' => |p| p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        'f' => |p| p.to_string_lossy().into_owned(),
        'm' => |p| mime_db().type_of(p, &base_name(p), p.is_dir()),
        'u' => |p| file_uri(p),
        'w' => |p| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        'x' => |p| p.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        _ => {
            return Some(match code {
                'c' => vec![paths.len().to_string()],
                'n' => vec![std::env::var("USER").unwrap_or_default()],
                's' => vec!["file".into()],
                '%' => vec!["%".into()],
                // The host, the port, and the no-op codes.
                'h' | 'p' | 'o' | 'O' => Vec::new(),
                _ => return None,
            });
        }
    };
    Some(match code.is_ascii_uppercase() {
        true => paths.iter().map(|p| of(p)).collect(),
        false => paths.first().map(|p| of(p)).into_iter().collect(),
    })
}

/// Thunar's range of how many: `*`, `2`, `1-3`, `2-*`.
fn in_range(range: &str, count: usize) -> bool {
    let bound = |text: &str, open: usize| match text.trim() {
        "" | "*" => Some(open),
        number => number.parse().ok(),
    };
    match range.split_once('-') {
        Some((low, high)) => bound(low, 0).zip(bound(high, usize::MAX)).is_some_and(|(low, high)| (low..=high).contains(&count)),
        None => bound(range, count) == Some(count),
    }
}

// --- Types (shared-mime-info) ----------------------------------------------

struct MimeDb {
    /// Weight, type, pattern, case-sensitive.
    globs: Vec<(u32, String, String, bool)>,
    aliases: HashMap<String, String>,
    parents: HashMap<String, Vec<String>>,
}

fn mime_db() -> &'static MimeDb {
    static DB: OnceLock<MimeDb> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = MimeDb { globs: Vec::new(), aliases: HashMap::new(), parents: HashMap::new() };
        for dir in data_dirs().iter().map(|dir| dir.join("mime")) {
            for line in fs::read_to_string(dir.join("globs2")).unwrap_or_default().lines().filter(|l| !l.starts_with('#')) {
                let mut fields = line.split(':');
                if let (Some(weight), Some(kind), Some(pattern)) = (fields.next(), fields.next(), fields.next()) {
                    let sensitive = fields.next().is_some_and(|flags| flags.contains("cs"));
                    db.globs.push((weight.parse().unwrap_or(50), kind.into(), pattern.into(), sensitive));
                }
            }
            let pairs = |file: &str| fs::read_to_string(dir.join(file)).unwrap_or_default();
            for line in pairs("aliases").lines() {
                if let Some((alias, kind)) = line.split_once(' ') {
                    db.aliases.entry(alias.into()).or_insert_with(|| kind.into());
                }
            }
            for line in pairs("subclasses").lines() {
                if let Some((kind, parent)) = line.split_once(' ') {
                    let parents = db.parents.entry(kind.into()).or_default();
                    if !parents.iter().any(|p| p == parent) {
                        parents.push(parent.into());
                    }
                }
            }
        }
        db
    })
}

impl MimeDb {
    /// By name, the heaviest and then longest pattern winning; else by a look inside.
    fn type_of(&self, path: &Path, name: &str, is_dir: bool) -> String {
        if is_dir {
            return "inode/directory".into();
        }
        let matched = self.globs.iter().filter(|(_, _, pattern, sensitive)| wildcard_match(pattern, name, *sensitive));
        match matched.max_by_key(|(weight, _, pattern, _)| (*weight, pattern.len())) {
            Some((_, kind, ..)) => self.aliases.get(kind).unwrap_or(kind).clone(),
            None => sniff(path).into(),
        }
    }

    /// The type, then what it is a kind of: text/plain for any text, and
    /// application/octet-stream for any file.
    fn ancestry(&self, kind: &str) -> Vec<String> {
        let mut all = vec![kind.to_owned()];
        let mut at = 0;
        while at < all.len() {
            for parent in self.parents.get(&all[at]).into_iter().flatten() {
                if !all.contains(parent) {
                    all.push(parent.clone());
                }
            }
            at += 1;
        }
        let mut add = |kind: &str| {
            if !all.iter().any(|k| k == kind) {
                all.push(kind.into());
            }
        };
        if kind.starts_with("text/") {
            add("text/plain");
        }
        if !kind.starts_with("inode/") {
            add("application/octet-stream");
        }
        all
    }
}

/// A file no pattern names: empty, a program, text, or just data.
// shortcut: no magic beyond these; read shared-mime-info's magic file if names fall short.
fn sniff(path: &Path) -> &'static str {
    let mut head = [0u8; 1024];
    let read = match path.metadata() {
        Ok(meta) if meta.is_file() => fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0),
        _ => return "application/octet-stream",
    };
    let head = &head[..read];
    let text = !head.contains(&0) && std::str::from_utf8(head).map_or_else(|e| e.error_len().is_none(), |_| true);
    match head {
        [] => "application/x-zerosize",
        [0x7f, b'E', b'L', b'F', ..] => "application/x-executable",
        _ if text => "text/plain",
        _ => "application/octet-stream",
    }
}

// --- Icons -----------------------------------------------------------------

/// An icon's file by its name in the desktop's icon theme (or hicolor),
/// the size nearest 32 px or scalable; or the path given.
pub fn icon(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        return Some(PathBuf::from(name)).filter(|p| p.is_file());
    }
    static FOUND: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let found = FOUND.get_or_init(Default::default);
    if let Some(known) = found.lock().ok()?.get(name) {
        return known.clone();
    }
    let bases: Vec<PathBuf> = dirs::home_dir().map(|home| home.join(".icons")).into_iter().chain(data_dirs().into_iter().map(|d| d.join("icons"))).collect();
    let mut themes = vec![icon_theme()];
    let mut path = None;
    let mut at = 0;
    while path.is_none() && at < themes.len() && at < 8 {
        let theme = themes[at].clone();
        let index = bases.iter().find_map(|base| fs::read_to_string(base.join(&theme).join("index.theme")).ok()).unwrap_or_default();
        let value = |key: &str| index.lines().find_map(|line| line.strip_prefix(key)).unwrap_or_default().to_owned();
        let mut folders: Vec<String> = value("Directories=").split(',').map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()).collect();
        // Scalable first, then the sizes nearest 32.
        folders.sort_by_key(|dir| match dir.contains("scalable") {
            true => 0,
            false => 1 + dir.split(|c: char| !c.is_ascii_digit()).find_map(|n| n.parse::<i32>().ok()).map_or(999, |n| (n - 32).abs()),
        });
        let theme = &theme;
        path = folders.iter().flat_map(|dir| bases.iter().map(move |base| base.join(theme).join(dir))).find_map(|dir| {
            ["svg", "png"].iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|p| p.is_file())
        });
        for parent in value("Inherits=").split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if !themes.iter().any(|t| t == parent) {
                themes.push(parent.into());
            }
        }
        if at == themes.len() - 1 && !themes.iter().any(|t| t == "hicolor") {
            themes.push("hicolor".into());
        }
        at += 1;
    }
    let path = path.or_else(|| {
        let pixmaps = data_dirs().into_iter().map(|d| d.join("pixmaps"));
        pixmaps.flat_map(|dir| ["svg", "png"].map(|ext| dir.join(format!("{name}.{ext}")))).find(|p| p.is_file())
    });
    found.lock().ok()?.insert(name.to_owned(), path.clone());
    path
}

/// KDE's icon theme, Xfce's, GNOME's, Cinnamon's, MATE's; hicolor where none is set.
fn icon_theme() -> String {
    static THEME: OnceLock<String> = OnceLock::new();
    THEME
        .get_or_init(|| {
            let desktops = desktops();
            let on = |name: &str| desktops.iter().any(|d| d == name);
            let found = match () {
                _ if on("kde") => Some(kde_setting("Icons", "Theme").unwrap_or_else(|| "breeze".into())),
                _ if on("xfce") => xfce_setting("IconThemeName"),
                _ => crate::theme::interface_setting("icon-theme"),
            };
            found.unwrap_or_else(|| "hicolor".into())
        })
        .clone()
}

/// A key of kdeglobals' group.
pub(crate) fn kde_setting(group: &str, key: &str) -> Option<String> {
    let path = dirs::config_dir()?.join("kdeglobals");
    read_desktop(&path)?.get(group)?.get(key).cloned().filter(|v| !v.is_empty())
}

/// One of Xfce's look settings (xsettings): the user's, else the distribution's defaults.
pub(crate) fn xfce_setting(name: &str) -> Option<String> {
    config_dirs().iter().find_map(|dir| {
        let text = fs::read_to_string(dir.join("xfce4/xfconf/xfce-perchannel-xml/xsettings.xml")).ok()?;
        let doc = roxmltree::Document::parse(&text).ok()?;
        let node = doc.descendants().find(|n| n.attribute("name") == Some(name))?;
        node.attribute("value").filter(|v| !v.is_empty()).map(str::to_owned)
    })
}

// --- Files every desktop shares --------------------------------------------

/// A .desktop-style file's groups: key → value, unescaped, a key's value
/// in the user's language where it has one.
type Groups = HashMap<String, HashMap<String, String>>;

fn read_desktop(path: &Path) -> Option<Groups> {
    let text = fs::read_to_string(path).ok()?;
    let langs = languages();
    let mut groups = Groups::new();
    // How well a value's language fits: lower is better, the plain key last.
    let mut fit: HashMap<(String, String), usize> = HashMap::new();
    let mut group = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name.to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let (base, rank) = match key.split_once('[') {
            Some((base, lang)) => match langs.iter().position(|l| l == lang.trim_end_matches(']')) {
                Some(at) => (base, at),
                // FileManager-Actions' files have only `Name[C]`.
                None if lang == "C]" => (base, langs.len() + 1),
                None => continue,
            },
            None => (key, langs.len()),
        };
        let slot = fit.entry((group.clone(), base.to_owned())).or_insert(usize::MAX);
        if rank <= *slot {
            *slot = rank;
            groups.entry(group.clone()).or_default().insert(base.to_owned(), unescape(value.trim()));
        }
    }
    Some(groups)
}

/// The spec's `\s \n \t \r \\`; other escapes stay for the Exec quoting.
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some(code @ ('s' | 'n' | 't' | 'r' | '\\'))) => {
                chars.next();
                out.push(match code {
                    's' => ' ',
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    _ => '\\',
                });
            }
            _ => out.push(c),
        }
    }
    out
}

/// `a;b;` and KDE 5's `a,b`.
fn split_list(value: &str) -> impl Iterator<Item = &str> {
    value.split([';', ',']).map(str::trim).filter(|v| !v.is_empty())
}

/// The user's language, as desktop files tag values: `hu_HU`, then `hu`.
fn languages() -> Vec<String> {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|var| std::env::var(var).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or_default();
    let mut found = vec![lang.to_owned()];
    if let Some((language, _)) = lang.split_once('_') {
        found.push(language.to_owned());
    }
    found.retain(|l| !l.is_empty() && l != "C" && l != "POSIX");
    found
}

/// XDG_CURRENT_DESKTOP's names, lower case: `ubuntu:GNOME` is ubuntu and gnome.
fn desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').map(str::to_lowercase).filter(|d| !d.is_empty()).collect()
}

fn data_dirs() -> Vec<PathBuf> {
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    unique(dirs::data_dir().into_iter().chain(std::env::split_paths(&system)))
}

fn config_dirs() -> Vec<PathBuf> {
    let system = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    unique(dirs::config_dir().into_iter().chain(std::env::split_paths(&system)))
}

fn unique(dirs: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    dirs.filter(|dir| !dir.as_os_str().is_empty() && seen.insert(dir.clone())).collect()
}

/// The .desktop files in `dir` and its subfolders, `depth` deep.
fn desktop_files(dir: &Path, depth: u8) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && depth > 0 {
            found.extend(desktop_files(&path, depth - 1));
        } else if path.extension().is_some_and(|e| e == "desktop") {
            found.push(path);
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exec_lines() {
        assert_eq!(split_exec(r#"kate -b %U"#).unwrap(), ["kate", "-b", "%U"]);
        assert_eq!(split_exec(r#"sh -c "echo \"a b\" \$HOME" %f"#).unwrap(), ["sh", "-c", r#"echo "a b" $HOME"#, "%f"]);
        assert_eq!(split_exec("kompare -c %U >/dev/null"), None, "a redirection needs the shell");
        assert_eq!(codes("--name=%c %% %k %x", |c| matches!(c, 'c' | '%' | 'k').then(|| c.to_string())), "--name=c % k %x");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert!(in_range("*", 3) && in_range("2", 2) && !in_range("2", 1));
        assert!(in_range("1-3", 3) && !in_range("1-3", 4) && in_range("2-*", 9) && !in_range("2-*", 1));
        assert_eq!(unescape(r"a\sb\\c\;"), r"a b\c\;");
    }

    #[test]
    fn reads_desktop_files_in_the_users_language() {
        let dir = std::env::temp_dir().join(format!("pc-desktop-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.desktop");
        fs::write(&file, "[Desktop Entry]\nName=Edit\nName[xx]=Other\nExec=ed %f\n# c\n[Desktop Action x]\nName=X\n").unwrap();
        let groups = read_desktop(&file).unwrap();
        assert_eq!(groups["Desktop Entry"]["Exec"], "ed %f");
        assert_eq!(groups["Desktop Action x"]["Name"], "X");
        if !languages().iter().any(|l| l == "xx") {
            assert_eq!(groups["Desktop Entry"]["Name"], "Edit");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn matches_kde_types() {
        let item = Item { name: "a.rs".into(), is_dir: false, types: vec!["text/rust".into(), "text/plain".into(), "application/octet-stream".into()] };
        assert!(item.is("all/allfiles") && item.is("text/*") && item.is("text/plain") && item.is("application/octet-stream"));
        assert!(!item.is("inode/directory") && !item.is("image/*"));
        let folder = Item { name: "d".into(), is_dir: true, types: vec!["inode/directory".into()] };
        assert!(folder.is("all/all") && !folder.is("all/allfiles") && folder.is("inode/directory"));
    }
    #[test]
    fn builds_command_lines() {
        let value = |code: char| match code {
            'F' => Some(vec!["/a b".into(), "/c".into()]),
            'f' => Some(vec!["/a b".into()]),
            'x' => Some(Vec::new()),
            _ => None,
        };
        assert_eq!(command_line("wc -l %F", value), ["wc", "-l", "/a b", "/c"]);
        assert_eq!(command_line("ed --file=%f %x %y", value), ["ed", "--file=/a b", "%y"]);
        assert_eq!(command_line("wc %F | less", value), ["sh", "-c", "wc '/a b' '/c' | less"]);
        assert_eq!(codes_in("run %f then %U").collect::<String>(), "fU");
        let paths = [PathBuf::from("/d/a.tar.gz"), PathBuf::from("/d/b")];
        assert_eq!(fma_value('B', &paths).unwrap(), ["a.tar.gz", "b"]);
        assert_eq!(fma_value('d', &paths).unwrap(), ["/d"]);
        assert_eq!(fma_value('c', &paths).unwrap(), ["2"]);
        assert_eq!(nemo_value('F', &paths, Path::new("/d"), Some(",")).unwrap(), ["/d/a.tar.gz,/d/b"]);
        assert_eq!(nemo_value('p', &[], Path::new("/d"), None).unwrap(), ["d"]);
    }

    #[test]
    fn matches_nemo_and_file_manager_actions() {
        let item = |name: &str, is_dir: bool, types: &[&str]| Item { name: name.into(), is_dir, types: types.iter().map(|t| t.to_string()).collect() };
        let entry = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let text = [item("a.TXT", false, &["text/plain", "application/octet-stream"])];
        let two = [item("a.txt", false, &["text/plain"]), item("d", true, &["inode/directory"])];
        let here = Path::new("/tmp");
        let nemo = |pairs: &[(&str, &str)], items: &[Item]| nemo_applies(&entry(pairs), items, here, here);
        assert!(nemo(&[("Selection", "s"), ("Extensions", "txt;")], &text));
        assert!(nemo(&[("Selection", "S"), ("Mimetypes", "text/*;")], &text));
        assert!(!nemo(&[("Selection", "m"), ("Extensions", "any;")], &text));
        assert!(!nemo(&[("Selection", "notnone"), ("Extensions", "nodirs;")], &two));
        assert!(nemo(&[("Selection", "none"), ("Extensions", "any;")], &[]));
        assert!(!nemo(&[("Selection", "any"), ("Extensions", "any;"), ("Dependencies", "!sh;")], &text));
        assert!(!nemo(&[("Selection", "any"), ("Extensions", "any;"), ("UriScheme", "sftp")], &text));
        let fma = |pairs: &[(&str, &str)], items: &[Item]| fma_applies(&entry(pairs), items, here);
        assert!(fma(&[], &two));
        assert!(fma(&[("MimeTypes", "text/*;"), ("SelectionCount", "=1")], &text));
        assert!(!fma(&[("MimeTypes", "*;!text/plain;")], &text));
        assert!(!fma(&[("Basenames", "*.txt;")], &text), "case matters by default");
        assert!(fma(&[("Basenames", "*.txt;"), ("Matchcase", "false")], &text));
        assert!(!fma(&[("SelectionCount", "<2")], &two));
        assert!(!fma(&[("Schemes", "sftp;")], &text));
        assert!(fma(&[("Folders", "/tmp/;")], &text) && !fma(&[("Folders", "/home;")], &text));
    }
}

#[cfg(test)]
mod system {
    use super::*;

    /// Prints what this desktop offers (it reads the system's): `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn lists_this_desktops_menu() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cases: Vec<Vec<(PathBuf, bool)>> = vec![
            vec![(repo.join("README.md"), false)],
            vec![(repo.join("crates/pc-core/src/lib.rs"), false)],
            vec![(repo.join("packaging/powercommander-1024.png"), false)],
            vec![(repo.join("crates"), true)],
            vec![(repo.join("Cargo.toml"), false), (repo.join("Cargo.lock"), false)],
        ];
        for chosen in cases {
            let started = std::time::Instant::now();
            let apps = open_with(&chosen);
            let actions = actions(&chosen, false);
            let item = Item::new(&chosen[0].0, chosen[0].1);
            println!("{:?} {:?} in {:?}", chosen.iter().map(|(p, _)| p.file_name().unwrap()).collect::<Vec<_>>(), item.types.first(), started.elapsed());
            for app in &apps {
                println!("  app    {} {:?}", app.name, app.icon.as_ref().map(|i| i.file_name().unwrap()));
            }
            for action in &actions {
                println!("  action {} [{:?} top:{}] {:?}", action.name, action.submenu, action.top_level, action.icon.as_ref().map(|i| i.file_name().unwrap()));
            }
        }
        println!("templates {:?}, terminal {:?}", templates(), terminal().map(|t| t.0));
    }
}
