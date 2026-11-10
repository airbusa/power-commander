//! Power Commander in the terminal: the window's panels, keys and core,
//! drawn as Midnight Commander does.

mod input;
mod keys;
mod ui;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pc_core::command;
use pc_core::menu::{self, Command};
use pc_core::session::{self, Session};
use pc_core::settings::Settings;
use pc_core::{Panel, Sort, SortKey, Tabs, archive};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::{execute, terminal::SetTitle};

use crate::input::Input;
use crate::ui::{KEY_BAR, Places};

pub struct Side {
    pub tabs: Tabs,
    /// The first entry in sight.
    pub top: usize,
}

pub struct Message {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub sides: [Side; 2],
    pub active: usize,
    pub settings: Settings,
    pub command: Input,
    pub message: Option<Message>,
    pub places: Places,
    bindings: HashMap<&'static str, Command>,
    /// For double clicks: when, and on which side's row.
    clicked: Option<(Instant, usize, usize)>,
    /// A command line for the shell, and its folder: the loop runs it.
    shell: Option<(String, PathBuf)>,
    quit: bool,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

fn main() -> io::Result<()> {
    let mut problems = Vec::new();
    let settings = Settings::load().unwrap_or_else(|err| {
        problems.push(format!("Could not read the settings: {err}"));
        Settings::default()
    });
    let session = Session::load_from(session::TERMINAL).unwrap_or_else(|err| {
        problems.push(format!("Could not read the open tabs: {err}"));
        Session::default()
    });
    std::thread::spawn(archive::remove_stale_scratch);
    let mut app = App::new(
        settings,
        &session,
        std::env::args_os().skip(1).map(PathBuf::from),
    );
    if !problems.is_empty() {
        app.say(problems.join("\n"), true);
    }
    let mut terminal = start();
    let ran = app.run(&mut terminal);
    stop();
    app.save_session();
    archive::remove_scratch();
    ran
}

/// Full screen, raw keys, the mouse.
fn start() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(
        io::stdout(),
        EnableMouseCapture,
        SetTitle("Power Commander")
    );
    terminal
}

fn stop() {
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

impl App {
    /// The saved tabs; folders given on the command line open on the left
    /// and right.
    fn new(settings: Settings, session: &Session, folders: impl Iterator<Item = PathBuf>) -> Self {
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let filter = settings.filter();
        let fallback = || {
            Panel::new(&home, filter)
                .or_else(|_| Panel::new("/", filter))
                .expect("list the home folder or /")
        };
        let side = |ix: usize| Side {
            tabs: match session.sides.get(ix) {
                Some(state) => Tabs::restore(state, filter, fallback),
                None => Tabs::new(fallback()),
            },
            top: 0,
        };
        let mut app = App {
            sides: [side(0), side(1)],
            active: session.active.min(1),
            settings,
            command: Input::default(),
            message: None,
            places: Places::default(),
            bindings: menu::key_bindings().into_iter().collect(),
            clicked: None,
            shell: None,
            quit: false,
        };
        for (ix, folder) in folders.take(2).enumerate() {
            let folder = command::resolve(
                &folder.to_string_lossy(),
                &std::env::current_dir().unwrap_or_default(),
            );
            app.navigate_on(ix, move |panel| panel.go_to_folder(folder));
        }
        app
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            terminal.draw(|frame| ui::draw(frame, self))?;
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
                Event::Mouse(mouse) => self.mouse(mouse),
                _ => {}
            }
            if let Some((line, folder)) = self.shell.take() {
                *terminal = self.run_shell(&line, &folder);
            }
        }
        Ok(())
    }

    fn save_session(&self) {
        let session = Session {
            sides: self.sides.iter().map(|side| side.tabs.state()).collect(),
            active: self.active,
            ..Session::default()
        };
        if let Err(err) = session.save_to(session::TERMINAL) {
            eprintln!("Could not save the open tabs: {err}");
        }
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            error,
        });
    }

    fn panel(&self) -> &Panel {
        self.sides[self.active].tabs.current()
    }

    fn panel_mut(&mut self) -> &mut Panel {
        self.sides[self.active].tabs.current_mut()
    }

    /// Rows a page moves: the file rows in sight.
    fn page(&self) -> isize {
        (self.places.rows[self.active].height as isize - 1).max(1)
    }

    fn navigate(&mut self, go: impl FnOnce(&mut Panel) -> io::Result<()>) {
        self.navigate_on(self.active, go);
    }

    fn navigate_on(&mut self, ix: usize, go: impl FnOnce(&mut Panel) -> io::Result<()>) {
        let tabs = &mut self.sides[ix].tabs;
        if let Err(err) = tabs.navigate(go) {
            let text = format!("{}: {err}", tabs.current().location());
            self.say(text, true);
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if self.message.take().is_some() {
            return;
        }
        let typing = !self.command.is_empty();
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Tab if plain => return self.active = 1 - self.active,
            KeyCode::Up if plain => return self.panel_mut().move_cursor(-1),
            KeyCode::Down if plain => return self.panel_mut().move_cursor(1),
            KeyCode::PageUp if plain => return self.move_page(-1),
            KeyCode::PageDown if plain => return self.move_page(1),
            KeyCode::Home if plain && !typing => return self.panel_mut().set_cursor(0),
            KeyCode::End if plain && !typing => return self.panel_mut().set_cursor(usize::MAX),
            KeyCode::Enter if plain && typing => return self.run_line(),
            KeyCode::Enter if plain => return self.navigate(|panel| panel.open().map(drop)),
            KeyCode::Backspace if plain && !typing => return self.navigate(Panel::up),
            KeyCode::PageUp if key.modifiers == KeyModifiers::CONTROL => {
                return self.navigate(Panel::up);
            }
            KeyCode::Esc => return self.command.clear(),
            KeyCode::F(10) if plain => return self.quit = true,
            KeyCode::F(9) if plain => return self.not_yet("The menus"),
            // Alt+1 to Alt+9: that tab.
            KeyCode::Char(c @ '1'..='9') if key.modifiers == KeyModifiers::ALT => {
                let tabs = &mut self.sides[self.active].tabs;
                return tabs.select(c as usize - '1' as usize);
            }
            _ => {}
        }
        // The line's own editing keys come first while there is a line.
        let editing = matches!(
            key.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Backspace
                | KeyCode::Delete
        );
        if typing && editing && self.command.handle(&key) {
            return;
        }
        let space = key.code == KeyCode::Char(' ');
        if let Some(name) = keys::name(&key)
            && (!space || !typing)
            && let Some(&command) = self.bindings.get(name.as_str())
        {
            return self.command(command);
        }
        self.command.handle(&key);
    }

    fn move_page(&mut self, direction: isize) {
        let rows = self.page() * direction;
        self.panel_mut().move_cursor(rows);
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if self.message.is_some() {
            if let MouseEventKind::Down(_) = mouse.kind {
                self.message = None;
            }
            return;
        }
        let at = ratatui::layout::Position::new(mouse.column, mouse.row);
        let side = (0..2).find(|&ix| self.places.rows[ix].contains(at));
        match (mouse.kind, side) {
            (MouseEventKind::ScrollUp, Some(ix)) => {
                self.sides[ix].tabs.current_mut().move_cursor(-3)
            }
            (MouseEventKind::ScrollDown, Some(ix)) => {
                self.sides[ix].tabs.current_mut().move_cursor(3)
            }
            (MouseEventKind::Down(MouseButton::Left), Some(ix)) => {
                self.active = ix;
                let row = self.sides[ix].top + (mouse.row - self.places.rows[ix].y) as usize;
                if row >= self.panel().entries().len() {
                    return;
                }
                self.panel_mut().set_cursor(row);
                let double = self.clicked.is_some_and(|(when, side, at)| {
                    side == ix && at == row && when.elapsed() < DOUBLE_CLICK
                });
                self.clicked = Some((Instant::now(), ix, row));
                if double {
                    self.clicked = None;
                    self.navigate(|panel| panel.open().map(drop));
                }
            }
            (MouseEventKind::Down(MouseButton::Left), None) if self.places.keys.contains(at) => {
                let slot =
                    (mouse.column - self.places.keys.x) / (self.places.keys.width / 10).max(1);
                if let Some((_, Some(command))) = KEY_BAR.get(slot as usize) {
                    self.command(*command);
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, command: Command) {
        let side = self.active;
        match command {
            Command::Quit => self.quit = true,
            Command::ToggleMark => self.panel_mut().toggle_mark(),
            Command::SelectAll => self.panel_mut().mark_all(true),
            Command::UnselectAll => self.panel_mut().mark_all(false),
            Command::InvertSelection => self.panel_mut().invert_marks(),
            Command::SortByName => self.sort(SortKey::Name),
            Command::SortByExtension => self.sort(SortKey::Extension),
            Command::SortByTime => self.sort(SortKey::Modified),
            Command::SortBySize => self.sort(SortKey::Size),
            Command::Unsorted => self.sort(SortKey::Unsorted),
            Command::ReversedOrder => {
                let sort = self.panel().sort();
                self.panel_mut().set_sort(Sort {
                    descending: !sort.descending,
                    ..sort
                });
            }
            Command::RereadSource => self.navigate(Panel::reload),
            Command::DuplicateTab => self.sides[side].tabs.duplicate(),
            Command::LockTab => self.sides[side].tabs.toggle_lock(),
            Command::CloseTab => self.sides[side].tabs.close(),
            Command::CloseAllTabs => self.sides[side].tabs.close_all(),
            Command::CloseDuplicateTabs => self.sides[side].tabs.close_duplicates(),
            Command::View | Command::Edit | Command::EditExternally => {
                self.not_yet("Viewing and editing")
            }
            Command::Copy
            | Command::Move
            | Command::NewFolder
            | Command::Delete
            | Command::DeletePermanently => self.not_yet("File operations"),
            Command::ConnectToServer | Command::Disconnect => self.not_yet("Servers"),
            _ => self.not_yet("This command"),
        }
    }

    /// The same key on the same column turns the order around, as a
    /// column title click does in the window.
    fn sort(&mut self, key: SortKey) {
        let sort = self.panel().sort();
        let sort = match sort.key == key {
            true => Sort {
                descending: !sort.descending,
                ..sort
            },
            false => Sort::by(key),
        };
        self.panel_mut().set_sort(sort);
    }

    // shortcut: the commands the terminal lacks say so; they come step by step.
    fn not_yet(&mut self, what: &str) {
        self.say(format!("{what}: not in the terminal version yet."), false);
    }

    /// `cd` moves the panel; anything else goes to the shell.
    fn run_line(&mut self) {
        let line = self.command.text().trim().to_owned();
        self.command.clear();
        let folder = self.panel().folder().to_owned();
        if let Some(target) = command::change_dir(&line, &folder) {
            return self.navigate(move |panel| panel.go_to_folder(target));
        }
        if self.panel().remote().is_some() {
            return self.say("Commands run on this computer, not on the server.", true);
        }
        self.shell = Some((line, folder));
    }

    /// In sight of the user, the panels hidden until Enter brings them back.
    fn run_shell(&mut self, line: &str, folder: &std::path::Path) -> DefaultTerminal {
        stop();
        println!("{}> {line}", folder.display());
        let ran = command::shell(line).current_dir(folder).status();
        if let Err(err) = ran {
            println!("{err}");
        }
        print!("\nPress Enter to return to Power Commander...");
        let _ = io::stdout().flush();
        let _ = io::stdin().read_line(&mut String::new());
        for side in &mut self.sides {
            side.tabs.reload_folder(folder);
        }
        start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draws_both_panels() {
        let root = std::env::temp_dir().join(format!("pc-tui-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("notes.txt"), "hello").unwrap();
        let folders = [root.clone(), root.join("sub")].into_iter();
        let mut app = App::new(Settings::default(), &Session::default(), folders);
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            screen.contains("/sub") && screen.contains("notes.txt"),
            "{screen}"
        );
        assert!(screen.contains("UP--DIR"));
        assert!(screen.contains("Mkdir") && screen.contains("Quit"));

        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        app.key(key(KeyCode::Down));
        app.key(key(KeyCode::Char(' ')));
        assert!(
            app.panel()
                .entries()
                .iter()
                .any(|e| app.panel().is_marked(e)),
            "space marks"
        );
        for c in "cd sub".chars() {
            app.key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            app.command.text(),
            "cd sub",
            "typing goes to the command line"
        );
        app.key(key(KeyCode::Enter));
        assert_eq!(app.panel().path(), root.join("sub"));
        app.key(key(KeyCode::Backspace));
        assert_eq!(app.panel().path(), root, "back up, the line being empty");
        app.key(key(KeyCode::Char('l')));
        app.key(key(KeyCode::Enter));
        assert_eq!(
            app.shell.take(),
            Some(("l".into(), root.clone())),
            "for the shell"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
