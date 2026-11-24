//! Interactive terminal dashboard for `doip-remote`.
//!
//! The event loop lives here: it polls key presses from a dedicated reader
//! thread, drains background driver events and redraws the UI. All rendering is
//! delegated to [`ui`] and all terminal setup to [`tui`].

mod app;
mod drivers;
mod logging;
mod net;
mod tui;
mod ui;

use std::{
    collections::VecDeque,
    io,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::Duration,
};

use app::{App, Focus, MenuItem};
use colored::Colorize;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use doip_core::{Role, SessionCode};
use drivers::InstallEvent;
use tokio::sync::mpsc as tokio_mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Placeholder relay address until the networking layer is implemented.
const RELAY_SERVER: &str = "auto";

fn install_panic_log_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("panic.log")))
            .unwrap_or_else(|| std::path::PathBuf::from("panic.log"));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(file, "=== PANIC ===");
            let _ = writeln!(file, "{info}");
            if let Some(location) = info.location() {
                let _ = writeln!(
                    file,
                    "at {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }
            let backtrace = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(file, "{backtrace}");
            let _ = writeln!(file);
            let _ = file.flush();
        }
        previous(info);
    }));
}

#[tokio::main]
async fn main() -> ExitCode {
    install_panic_log_hook();
    let mock = std::env::args().skip(1).any(|arg| arg == "--mock");

    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let mut logger = logging::init(Arc::clone(&logs));

    let mut app = App::new(logs);
    let drivers_ok = if mock {
        app.enable_mock();
        info!("[OK] Mock mode: administrator rights and drivers emulated, network is not.");
        true
    } else {
        app.admin_status = drivers::is_admin();
        let status = drivers::check_all();
        app.set_driver_status(status.npcap_installed, status.loopback_installed);
        status.is_all_ok()
    };

    log_startup_status(&app, drivers_ok);
    app.set_log_file_exists(logging::log_file_path().is_file());

    let mut terminal = match tui::init() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("Failed to initialize terminal: {error}");
            return ExitCode::FAILURE;
        }
    };

    let cancellation = CancellationToken::new();
    let network = net::NetworkService::start(cancellation.clone(), None);

    let result = run(&mut terminal, &mut app, &mut logger, &cancellation).await;
    tui::restore();

    // Any exit path cancels the network tasks; shutdown waits for the tasks and
    // drops their sockets before the process ends.
    network.shutdown().await;

    // The log writer must be closed before the file can be removed on Windows.
    let delete_log = app.delete_log_on_exit;
    drop(logger);

    if delete_log {
        let path = logging::log_file_path();
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Failed to delete log {}: {error}", path.display());
            }
        }
    }

    match result {
        Ok(()) => {
            println!("{}", "Goodbye!".bright_black());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("TUI error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn log_startup_status(app: &App, drivers_ok: bool) {
    info!(
        "Logging started, file: {}",
        logging::log_file_path().display()
    );

    if app.admin_status {
        info!("[OK] Administrator rights available.");
    } else {
        warn!("[!] No administrator rights: driver installation is unavailable.");
    }

    info!(
        "Driver check: Npcap — {}, Loopback — {}",
        ready_tag(app.npcap_status),
        ready_tag(app.loopback_status)
    );

    if drivers_ok {
        info!("[OK] All drivers are installed.");
    } else {
        warn!("Not all drivers are installed: install them from the menu.");
    }

    info!("Tab — switch focus to logs, PageUp/PageDown — scroll.");
}

async fn run(
    terminal: &mut tui::Tui,
    app: &mut App,
    logger: &mut Option<logging::Logger>,
    cancellation: &CancellationToken,
) -> io::Result<()> {
    let keys = tui::spawn_key_reader();
    let (install_tx, mut install_rx) = tokio_mpsc::unbounded_channel();

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;

        while let Ok(key) = keys.try_recv() {
            handle_key(app, key, &install_tx, logger, cancellation);
        }

        while let Ok(event) = install_rx.try_recv() {
            handle_install_event(app, event);
        }

        tokio::time::sleep(Duration::from_millis(16)).await;
    }

    // Safety net for early returns and any quit path that forgot to cancel.
    cancellation.cancel();

    Ok(())
}

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>,
    logger: &mut Option<logging::Logger>,
    cancellation: &CancellationToken,
) {
    // Ctrl+C must work in every mode, including the modals.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        if app.show_confirm_exit {
            app.show_confirm_exit = false;
            perform_exit(app, cancellation);
        } else {
            request_exit(app, cancellation);
        }
        return;
    }

    if app.show_confirm_delete {
        match key.code {
            KeyCode::Enter => confirm_cleanup(app, logger, cancellation),
            KeyCode::Esc => {
                app.show_confirm_delete = false;
                info!("Cleanup canceled.");
            }
            _ => {}
        }
        return;
    }

    if app.show_confirm_exit {
        match key.code {
            KeyCode::Enter => {
                app.show_confirm_exit = false;
                perform_exit(app, cancellation);
            }
            KeyCode::Esc => {
                app.show_confirm_exit = false;
                info!("Exit canceled.");
            }
            _ => {}
        }
        return;
    }

    if app.session_code.is_some() {
        match key.code {
            KeyCode::Enter => {
                if let Some(code) = app.session_code.as_deref() {
                    copy_session_code(code);
                }
                app.session_code = None;
            }
            KeyCode::Esc => app.session_code = None,
            _ => {}
        }
        return;
    }

    if app.diag_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.diag_input = None;
                info!("Code entry canceled.");
            }
            KeyCode::Enter => {
                if let Some(code) = app.diag_input.take() {
                    accept_diag_code(&code);
                }
            }
            KeyCode::Backspace => {
                if let Some(input) = app.diag_input.as_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = app.diag_input.as_mut() {
                    input.push(character);
                }
            }
            _ => {}
        }
        return;
    }

    if let KeyCode::Char(character @ '1'..='5') = key.code {
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(item) = MenuItem::from_hotkey(character) {
                if app.is_cleaned_up && item != MenuItem::Exit {
                    return;
                }
                if app.is_installing {
                    warn!("Wait for driver installation to finish.");
                } else {
                    app.menu_index = MenuItem::ALL
                        .iter()
                        .position(|candidate| *candidate == item)
                        .unwrap_or(app.menu_index);
                    activate(app, install_tx, cancellation);
                }
            }
            return;
        }
    }

    match key.code {
        KeyCode::Tab => app.toggle_focus(),
        KeyCode::Up => match app.focus {
            Focus::Menu => app.previous(),
            Focus::Logs => app.scroll_logs_up(1),
        },
        KeyCode::Down => match app.focus {
            Focus::Menu => app.next(),
            Focus::Logs => app.scroll_logs_down(1),
        },
        KeyCode::PageUp => {
            if app.focus == Focus::Logs {
                app.scroll_logs_up(5);
            }
        }
        KeyCode::PageDown => {
            if app.focus == Focus::Logs {
                app.scroll_logs_down(5);
            }
        }
        KeyCode::Enter => {
            if app.focus == Focus::Menu {
                if app.is_cleaned_up && app.selected() != MenuItem::Exit {
                    // Menu items 1-4 are disabled after cleanup.
                } else if app.is_installing {
                    warn!("Wait for driver installation to finish.");
                } else {
                    activate(app, install_tx, cancellation);
                }
            }
        }
        KeyCode::Esc => {
            if app.focus == Focus::Logs {
                app.focus = Focus::Menu;
            } else {
                request_exit(app, cancellation);
            }
        }
        KeyCode::Char('q') if app.focus == Focus::Menu => {
            request_exit(app, cancellation);
        }
        _ => {}
    }
}

fn request_exit(app: &mut App, cancellation: &CancellationToken) {
    if app.connection_status == app::ConnectionStatus::Connected {
        app.show_confirm_exit = true;
        return;
    }
    perform_exit(app, cancellation);
}

fn perform_exit(app: &mut App, cancellation: &CancellationToken) {
    cancellation.cancel();
    app.should_quit = true;
}

fn activate(
    app: &mut App,
    install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>,
    cancellation: &CancellationToken,
) {
    match app.selected() {
        MenuItem::Install => begin_install(app, install_tx),
        MenuItem::Car => begin_car(app),
        MenuItem::Diag => {
            app.diag_input = Some(String::new());
            info!("Enter the DT1-... session code:");
        }
        MenuItem::Cleanup => {
            app.show_confirm_delete = true;
        }
        MenuItem::Exit => {
            request_exit(app, cancellation);
        }
    }
}

fn begin_install(app: &mut App, install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>) {
    if app.is_installing {
        info!("Installation is already running, please wait.");
        return;
    }

    if !app.admin_status {
        warn!("[!] Administrator rights are required. Restart the program as Administrator.");
        return;
    }

    if app.drivers_ready() {
        info!("[OK] All drivers are already installed.");
        return;
    }

    app.is_installing = true;
    info!("Downloading and installing drivers...");

    drivers::install_all_async(install_tx.clone());
}

fn handle_install_event(app: &mut App, event: InstallEvent) {
    match event {
        InstallEvent::Finished(result) => {
            app.is_installing = false;

            let status = drivers::check_all();
            app.set_driver_status(status.npcap_installed, status.loopback_installed);

            match result {
                Ok(()) => info!("[OK] Installation complete. Check the status panel."),
                Err(error) => error!("Installation failed: {error}"),
            }

            info!(
                "Npcap — {}, Loopback — {}",
                ready_tag(app.npcap_status),
                ready_tag(app.loopback_status)
            );
        }
    }
}

fn begin_car(app: &mut App) {
    let session = SessionCode::new(Role::Car, RELAY_SERVER, None);
    let code = session.generate();

    info!("Car session created. Share this code with the diagnostician:");
    info!("{code}");
    info!("Waiting for server connection (stub)...");

    app.set_session_code(code);
}

/// Copies the generated session code to the system clipboard.
fn copy_session_code(code: &str) {
    let copied =
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(code.to_owned()));

    match copied {
        Ok(()) => info!("Code copied to clipboard!"),
        Err(error) => warn!("Failed to copy code to clipboard: {error}"),
    }
}

fn accept_diag_code(input: &str) {
    match SessionCode::parse(input) {
        Ok(session) => {
            info!("[OK] Code accepted. Server: {}", session.server);
            info!(
                "Target vehicle IP: {:?}, DoIP port: {}",
                session.vehicle_ip, session.doip_port
            );
            info!("Waiting for connection...");
        }
        Err(error) => error!("Failed to parse code: {error}"),
    }
}

/// Handles the destructive menu entry: disconnects sockets, removes drivers,
/// deletes the log file, then **returns to the event loop** so the refreshed
/// statuses stay on screen until the user chooses "5. Exit" or presses Ctrl+C.
///
/// This function never sets `app.should_quit`, never exits the process, and never
/// drops the tokio runtime. It is panic-safe: any unexpected panic is caught and
/// converted into a UI log entry while keeping the app alive in cleaned-up mode.
fn confirm_cleanup(
    app: &mut App,
    logger: &mut Option<logging::Logger>,
    cancellation: &CancellationToken,
) {
    app.show_confirm_delete = false;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cleanup_sequence(app, logger, cancellation);
    }));

    if let Err(payload) = result {
        let message = panic_message(&payload);
        error!("Cleanup panicked: {message}");
    }

    // Always force the cleaned-up UI state, even if a step panicked.
    // IMPORTANT: keep the menu index on the disabled cleanup item. Moving it to
    // Exit here would let a repeated Enter (key auto-repeat or a second press)
    // immediately quit the app right after cleanup, which looks like a crash.
    // Only an explicit numeric `5` or navigating to Exit can quit from here.
    let status = drivers::check_all();
    app.apply_cleanup_status(
        status.npcap_installed,
        status.loopback_installed,
        logging::log_file_path().is_file(),
    );
    app.is_cleaned_up = true;
}

fn cleanup_sequence(
    app: &mut App,
    logger: &mut Option<logging::Logger>,
    cancellation: &CancellationToken,
) {
    // Step 1: disconnect active sockets.
    cancellation.cancel();
    app.connection_status = app::ConnectionStatus::Disconnected;
    info!("[OK] Disconnect requested (CancellationToken triggered).");

    // Step 2: remove Npcap and Loopback.
    match drivers::remove_all() {
        Ok(()) => info!("[OK] Drivers removed."),
        Err(error) => warn!("Driver removal: {error}"),
    }

    // Step 3: stop file logger, delete log file, switch to dummy/no-op file
    // writer. The UI log layer remains active so this event is still visible.
    let deletion = match logger.as_mut() {
        Some(logger) => logger.delete_log_file(),
        None => std::fs::remove_file(logging::log_file_path()).or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        }),
    };

    match deletion {
        Ok(()) => {
            app.delete_log_on_exit = false;
            info!("Logging disabled, log file deleted");
        }
        Err(error) => {
            warn!("Failed to delete log: {error}. Will retry on exit.");
            app.delete_log_on_exit = true;
        }
    }

    // Step 4: force driver/log status refresh for the UI.
    let status = drivers::check_all();
    app.apply_cleanup_status(
        status.npcap_installed,
        status.loopback_installed,
        logging::log_file_path().is_file(),
    );

    info!("Cleanup complete. The application keeps running.");
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_owned()
    }
}

fn ready_tag(ready: bool) -> &'static str {
    if ready {
        "[OK]"
    } else {
        "[MISSING]"
    }
}
