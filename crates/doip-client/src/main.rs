//! Interactive terminal UI for `doip-remote`.
//!
//! Stage 2 of the ratatui migration: the application state lives in [`app`].
//! The dashboard renderer and the event loop are added in stage 3.

mod app;
mod logging;

use std::{
    collections::VecDeque,
    process::ExitCode,
    sync::{Arc, Mutex},
};

use app::App;

fn main() -> ExitCode {
    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let _log_guard = logging::init(Arc::clone(&logs));

    tracing::info!(
        "Логирование запущено, файл: {}",
        logging::log_file_path().display()
    );

    let mut app = App::new(logs);
    app.next();
    app.previous();
    app.toggle_focus();
    app.toggle_focus();
    app.set_driver_status(false, false);
    app.scroll_logs_up(3);
    app.scroll_logs_down(3);

    println!("TUI-дашборд в разработке (этап 2/6: состояние).");
    println!(
        "Выбран пункт: {} (опасный: {})",
        app.selected().label(),
        app.selected().is_dangerous()
    );
    println!("Драйверы готовы: {}", app.drivers_ready());
    println!(
        "admin={} installing={} confirm={} connection={:?} quit={}",
        app.admin_status,
        app.is_installing,
        app.show_confirm_delete,
        app.connection_status,
        app.should_quit
    );
    ExitCode::SUCCESS
}
