//! Terminal lifecycle helpers for the TUI.

use std::{
    io::{self, stdout, Stdout},
    sync::mpsc::{self, Receiver},
    thread,
};

use crossterm::{
    cursor::Show,
    event::{read, Event, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

/// Ratatui terminal used by the dashboard.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enters raw mode and the alternate screen and installs a panic hook.
///
/// The panic hook restores the terminal before the default hook prints the
/// panic message, so a crash never leaves the user in a broken shell.
pub fn init() -> io::Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;

    if let Err(error) = execute!(stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error);
    }

    match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            restore();
            Err(error)
        }
    }
}

/// Restores the terminal: leaves the alternate screen, shows the cursor and
/// disables raw mode. Safe to call more than once.
pub fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, Show);
}

/// Re-asserts the TUI console mode after an external child process ran.
///
/// Windows installers may reset the shared console mode even when they do not
/// close it. Re-enabling raw mode and the alternate screen keeps key handling
/// and rendering working after cleanup.
pub fn reinitialize_after_child_process() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    Ok(())
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Spawns a thread that forwards key presses to the UI loop.
///
/// The thread ends when the receiver is dropped (the application exits) or when
/// the terminal cannot be read anymore.
pub fn spawn_key_reader() -> Receiver<KeyEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || loop {
        match read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if sender.send(key).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });

    receiver
}
