//! Interactive terminal UI for `doip-remote`.
//!
//! Stage 1 of the ratatui migration: logging is already routed to the file and
//! to the in-memory buffer. The dashboard itself is added in the next stages.

mod logging;

use std::{
    collections::VecDeque,
    process::ExitCode,
    sync::{Arc, Mutex},
};

fn main() -> ExitCode {
    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let _log_guard = logging::init(Arc::clone(&logs));

    tracing::info!(
        "Логирование запущено, файл: {}",
        logging::log_file_path().display()
    );

    println!("TUI-дашборд находится в разработке (этап 1/6: логирование).");
    ExitCode::SUCCESS
}
