//! Interactive terminal dashboard for `doip-remote`.
//!
//! The event loop lives here: it polls key presses from a dedicated reader
//! thread, drains background driver events and redraws the UI. All rendering is
//! delegated to [`ui`] and all terminal setup to [`tui`].

mod app;
mod drivers;
mod logging;
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
use tracing::{error, info, warn};

/// Placeholder relay address until the networking layer is implemented.
const RELAY_SERVER: &str = "auto";

#[tokio::main]
async fn main() -> ExitCode {
    let mock = std::env::args().skip(1).any(|arg| arg == "--mock");

    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let _log_guard = logging::init(Arc::clone(&logs));

    let mut app = App::new(logs);
    let drivers_ok = if mock {
        app.enable_mock();
        info!("[OK] Mock-режим: все статусы эмулируются зелёными.");
        true
    } else {
        app.admin_status = drivers::is_admin();
        let status = drivers::check_all();
        app.set_driver_status(status.npcap_installed, status.loopback_installed);
        status.is_all_ok()
    };

    log_startup_status(&app, drivers_ok);

    let mut terminal = match tui::init() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("Не удалось инициализировать терминал: {error}");
            return ExitCode::FAILURE;
        }
    };

    let result = run(&mut terminal, &mut app).await;
    tui::restore();

    // The log writer must be closed before the file can be removed on Windows.
    let delete_log = app.delete_log_on_exit;
    drop(_log_guard);

    if delete_log {
        let path = logging::log_file_path();
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Не удалось удалить лог {}: {error}", path.display());
            }
        }
    }

    match result {
        Ok(()) => {
            println!("{}", "До встречи!".bright_black());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Ошибка TUI: {error}");
            ExitCode::FAILURE
        }
    }
}

fn log_startup_status(app: &App, drivers_ok: bool) {
    info!(
        "Логирование запущено, файл: {}",
        logging::log_file_path().display()
    );

    if app.admin_status {
        info!("[OK] Права администратора есть.");
    } else {
        warn!("Нет прав администратора: установка драйверов недоступна.");
    }

    info!(
        "Проверка драйверов: Npcap — {}, Loopback — {}",
        ready_tag(app.npcap_status),
        ready_tag(app.loopback_status)
    );

    if drivers_ok {
        info!("[OK] Все драйверы установлены.");
    } else {
        warn!("Не все драйверы установлены: установите их из меню.");
    }

    info!("Tab — переключить фокус на логи, PageUp/PageDown — прокрутка.");
}

async fn run(terminal: &mut tui::Tui, app: &mut App) -> io::Result<()> {
    let keys = tui::spawn_key_reader();
    let (install_tx, mut install_rx) = tokio_mpsc::unbounded_channel();

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;

        while let Ok(key) = keys.try_recv() {
            handle_key(app, key, &install_tx);
        }

        while let Ok(event) = install_rx.try_recv() {
            handle_install_event(app, event);
        }

        tokio::time::sleep(Duration::from_millis(16)).await;
    }

    Ok(())
}

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>,
) {
    // Ctrl+C must work in every mode, including the modals.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }

    if app.show_confirm_delete {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => confirm_cleanup(app),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.show_confirm_delete = false;
                info!("Удаление отменено.");
            }
            _ => {}
        }
        return;
    }

    if app.diag_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.diag_input = None;
                info!("Ввод кода отменён.");
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
                if app.is_installing {
                    warn!("Дождитесь завершения установки драйверов.");
                } else {
                    activate(app, install_tx);
                }
            }
        }
        KeyCode::Esc => {
            if app.focus == Focus::Logs {
                app.focus = Focus::Menu;
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::Char('q') if app.focus == Focus::Menu => app.should_quit = true,
        _ => {}
    }
}

fn activate(app: &mut App, install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>) {
    match app.selected() {
        MenuItem::Install => begin_install(app, install_tx),
        MenuItem::Car => begin_car(),
        MenuItem::Diag => {
            app.diag_input = Some(String::new());
            info!("Введите код подключения DT1-...");
        }
        MenuItem::Cleanup => {
            app.show_confirm_delete = true;
        }
        MenuItem::Exit => app.should_quit = true,
    }
}

fn begin_install(app: &mut App, install_tx: &tokio_mpsc::UnboundedSender<InstallEvent>) {
    if app.is_installing {
        info!("Установка уже выполняется, дождитесь завершения.");
        return;
    }

    if !app.admin_status {
        warn!("Требуются права Администратора. Перезапустите программу от имени Администратора.");
        return;
    }

    if app.drivers_ready() {
        info!("[OK] Все драйверы уже установлены.");
        return;
    }

    app.is_installing = true;
    info!("Скачивание и установка драйверов...");

    drivers::install_all_async(install_tx.clone());
}

fn handle_install_event(app: &mut App, event: InstallEvent) {
    match event {
        InstallEvent::Finished(result) => {
            app.is_installing = false;

            let status = drivers::check_all();
            app.set_driver_status(status.npcap_installed, status.loopback_installed);

            match result {
                Ok(()) => info!("[OK] Установка завершена. Проверьте статус."),
                Err(error) => error!("Установка не удалась: {error}"),
            }

            info!(
                "Npcap — {}, Loopback — {}",
                ready_tag(app.npcap_status),
                ready_tag(app.loopback_status)
            );
        }
    }
}

fn begin_car() {
    let session = SessionCode::new(Role::Car, RELAY_SERVER, None);
    let code = session.generate();

    info!("Сессия Car создана. Передайте код диагносту:");
    info!("{code}");
    info!("Ожидание подключения к серверу (заглушка)...");
}

fn accept_diag_code(input: &str) {
    match SessionCode::parse(input) {
        Ok(session) => {
            info!("[OK] Код принят. Сервер: {}", session.server);
            info!(
                "Целевой IP авто: {:?}, порт DoIP: {}",
                session.vehicle_ip, session.doip_port
            );
            info!("Ожидание подключения...");
        }
        Err(error) => error!("Не удалось разобрать код: {error}"),
    }
}

/// Handles the destructive menu entry: removes the drivers and the log file,
/// then stops the application.
///
/// On Windows the log file cannot be deleted while `tracing-appender` still has
/// it open, so the removal is retried after the writer guard is dropped in
/// `main`.
fn confirm_cleanup(app: &mut App) {
    app.show_confirm_delete = false;

    drivers::remove_all();

    let path = logging::log_file_path();
    match std::fs::remove_file(&path) {
        Ok(()) => info!("[OK] Лог удалён: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            info!("Лог уже отсутствует: {}", path.display());
        }
        Err(error) => {
            warn!("Не удалось удалить лог сейчас: {error}. Он будет удалён при выходе.");
            app.delete_log_on_exit = true;
        }
    }

    app.should_quit = true;
}

fn ready_tag(ready: bool) -> &'static str {
    if ready {
        "[OK]"
    } else {
        "[НЕТ]"
    }
}
