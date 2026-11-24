//! Logging setup for the terminal UI.
//!
//! Every event is written to two sinks:
//!
//! * `doip-remote.log` on the user's Desktop (falling back to the home
//!   directory when `Desktop` is missing);
//! * an in-memory ring buffer that the TUI renders in the `[ LOGS ]` panel.
//!
//! The in-memory mirror is implemented as a custom [`Layer`], so any
//! `tracing` event emitted anywhere in the client shows up in the dashboard
//! without extra plumbing.

use std::{
    collections::VecDeque,
    fmt, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tracing::{field::Field, Event, Level, Subscriber};
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling,
};
use tracing_subscriber::{
    field::Visit,
    filter::LevelFilter,
    fmt::{self as sub_fmt, format::DefaultFields, format::Format, writer::BoxMakeWriter},
    layer::{Context, Layer, Layered},
    prelude::*,
    reload, Registry,
};

/// Name of the log file written to the user's Desktop.
pub const LOG_FILE_NAME: &str = "doip-remote.log";

/// Preferred subdirectory of the home directory for the log file.
const DESKTOP_DIR: &str = "Desktop";

/// Maximum number of lines kept in the in-memory buffer.
const MAX_BUFFERED_LINES: usize = 1000;

/// Subscriber below the file layer: the global level filter on top of the
/// registry. The reload handle is bound to this exact type.
type BaseSubscriber = Layered<LevelFilter, Registry>;

/// File layer whose writer can be swapped at runtime.
type FileLayer = sub_fmt::Layer<BaseSubscriber, DefaultFields, Format, BoxMakeWriter>;

/// Owns the logging pipeline and the file writer.
///
/// Holding this value keeps the background appender alive. [`Logger`] also
/// exposes [`Logger::delete_log_file`], which replaces the file writer with a
/// sink and drops the old writer **before** removing the file, so Windows does
/// not report a sharing violation for the still-open log handle.
#[derive(Debug)]
pub struct Logger {
    reload: reload::Handle<FileLayer, BaseSubscriber>,
    guard: Option<WorkerGuard>,
    sink_guard: Option<WorkerGuard>,
    path: PathBuf,
}

impl Logger {
    /// Disables file logging, deletes the resolved log file and keeps the
    /// in-memory log mirror working so the cleanup itself can still be reported
    /// in the UI.
    ///
    /// Order matters on Windows: the file writer is swapped for a sink and the
    /// old appender guard is dropped (closing the handle) before `remove_file`.
    pub fn delete_log_file(&mut self) -> io::Result<()> {
        // Install the no-op file writer first. Keeping its guard alive for the
        // rest of the session means every later `info!`/`warn!` is accepted by
        // `tracing` without touching the deleted file; the UI mirror stays
        // active regardless of the file-layer state.
        let (writer, sink_guard) = tracing_appender::non_blocking(io::sink());
        self.reload
            .reload(file_layer(writer))
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.sink_guard = Some(sink_guard);

        // Stops the old appender thread and closes the log file handle.
        drop(self.guard.take());

        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Builds the file layer around a non-blocking writer.
///
/// `BoxMakeWriter` erases the concrete writer type so the layer type stays the
/// same when the reload handle swaps the appender for a sink.
fn file_layer(writer: NonBlocking) -> FileLayer {
    sub_fmt::layer()
        .with_writer(BoxMakeWriter::new(writer))
        .with_ansi(false)
        .with_target(false)
}

/// Returns the user's home directory.
///
/// Windows uses `USERPROFILE` (with `HOMEDRIVE` + `HOMEPATH` as a fallback);
/// other platforms use `HOME`.
fn home_directory() -> Option<PathBuf> {
    fn env_path(name: &str) -> Option<PathBuf> {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    #[cfg(target_os = "windows")]
    {
        env_path("USERPROFILE").or_else(|| {
            let mut home = env_path("HOMEDRIVE")?;
            home.push(env_path("HOMEPATH")?);
            Some(home)
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        env_path("HOME")
    }
}

/// Places the log in `home/Desktop` when that directory exists, otherwise
/// directly in `home`.
fn log_path_in(home: &Path, desktop_exists: bool) -> PathBuf {
    if desktop_exists {
        home.join(DESKTOP_DIR).join(LOG_FILE_NAME)
    } else {
        home.join(LOG_FILE_NAME)
    }
}

/// Returns the path of `doip-remote.log`.
///
/// The preferred location is `[USERPROFILE]/Desktop/doip-remote.log` on Windows
/// and `$HOME/Desktop/doip-remote.log` elsewhere. If the Desktop directory does
/// not exist, the log falls back to the home directory itself. Only when no home
/// directory can be resolved does it fall back to the current working directory.
#[must_use]
pub fn log_file_path() -> PathBuf {
    if let Some(home) = home_directory() {
        return log_path_in(&home, home.join(DESKTOP_DIR).is_dir());
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(LOG_FILE_NAME)
}

/// Initializes the global `tracing` subscriber.
///
/// Returns the [`Logger`] that keeps the file writer alive and can delete the
/// log file later. Returns `None` if a global subscriber was already installed
/// (for example by a test harness).
#[must_use]
pub fn init(logs: Arc<Mutex<VecDeque<String>>>) -> Option<Logger> {
    let path = log_file_path();
    let directory = path.parent().unwrap_or_else(|| Path::new("."));

    // Create the file eagerly: the status panel reports its existence and the
    // rolling appender itself opens files lazily on the first write.
    if let Err(error) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        eprintln!("Failed to create log {}: {error}", path.display());
    }

    let appender = rolling::never(directory, LOG_FILE_NAME);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let (layer, reload) = reload::Layer::new(file_layer(writer));

    let installed = tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(layer)
        .with(UiLayer::new(logs))
        .try_init()
        .is_ok();

    installed.then_some(Logger {
        reload,
        guard: Some(guard),
        sink_guard: None,
        path,
    })
}

/// A `tracing` layer that mirrors formatted events into the UI buffer.
struct UiLayer {
    logs: Arc<Mutex<VecDeque<String>>>,
}

impl UiLayer {
    fn new(logs: Arc<Mutex<VecDeque<String>>>) -> Self {
        Self { logs }
    }

    fn push(&self, line: String) {
        let Ok(mut logs) = self.logs.lock() else {
            return;
        };

        if logs.len() >= MAX_BUFFERED_LINES {
            logs.pop_front();
        }
        logs.push_back(line);
    }
}

impl<S: Subscriber> Layer<S> for UiLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let line = format!(
            "{} {}",
            level_tag(*event.metadata().level()),
            visitor.message()
        );
        self.push(line);
    }
}

/// Extracts the `message` field (or the remaining fields) of an event.
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn message(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(message), true) => message,
            (Some(message), false) => format!("{message} ({})", self.fields.join(", ")),
            (None, false) => self.fields.join(", "),
            (None, true) => String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

impl MessageVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

fn level_tag(level: Level) -> &'static str {
    match level {
        Level::ERROR => "[ERROR]",
        Level::WARN => "[WARN]",
        Level::INFO => "[INFO]",
        Level::DEBUG => "[DEBUG]",
        Level::TRACE => "[TRACE]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_prefers_the_desktop() {
        let home = Path::new("home");
        let path = log_path_in(home, true);

        assert_eq!(path, home.join(DESKTOP_DIR).join(LOG_FILE_NAME));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(LOG_FILE_NAME)
        );
    }

    #[test]
    fn log_file_falls_back_to_the_home_directory() {
        let home = Path::new("home");
        let path = log_path_in(home, false);

        assert_eq!(path, home.join(LOG_FILE_NAME));
    }

    #[test]
    fn resolved_log_path_uses_the_log_file_name() {
        let path = log_file_path();

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(LOG_FILE_NAME)
        );
    }

    #[test]
    fn buffer_is_capped() {
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let layer = UiLayer::new(Arc::clone(&logs));

        for index in 0..(MAX_BUFFERED_LINES + 10) {
            layer.push(format!("line {index}"));
        }

        let logs = logs.lock().expect("buffer lock");
        assert_eq!(logs.len(), MAX_BUFFERED_LINES);
        assert!(logs.front().is_some_and(|line| line == "line 10"));
    }

    #[test]
    fn level_tags_are_stable() {
        assert_eq!(level_tag(Level::INFO), "[INFO]");
        assert_eq!(level_tag(Level::WARN), "[WARN]");
        assert_eq!(level_tag(Level::ERROR), "[ERROR]");
    }
}
