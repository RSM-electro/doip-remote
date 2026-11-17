//! Application state for the ratatui dashboard.
//!
//! The state is deliberately plain data: the renderer reads it and the event
//! loop mutates it. No terminal or networking types leak in here, which keeps
//! the state easy to unit-test.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Which pane currently owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The main menu is active.
    Menu,
    /// The log panel is active (PageUp/PageDown scroll it).
    Logs,
}

/// State of the diagnostic tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `Connected` is used by the `--mock` mode and later by the relay.
pub enum ConnectionStatus {
    /// No tunnel has been established.
    Disconnected,
    /// A tunnel to the relay is established.
    Connected,
}

/// Entries of the main menu, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Download and install the L2 drivers.
    Install,
    /// Act as the vehicle-side peer and generate a session code.
    Car,
    /// Act as the diagnostic peer and accept a session code.
    Diag,
    /// Stop diagnostics, remove drivers and delete logs.
    Cleanup,
    /// Stop diagnostics and quit.
    Exit,
}

impl MenuItem {
    /// All menu entries in display order.
    pub const ALL: [Self; 5] = [
        Self::Install,
        Self::Car,
        Self::Diag,
        Self::Cleanup,
        Self::Exit,
    ];

    /// Human-readable menu label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "Установить драйверы",
            Self::Car => "Я рядом с машиной (Car)",
            Self::Diag => "Я диагност (Diag)",
            Self::Cleanup => "Завершить диагностику с удалением драйверов и логов",
            Self::Exit => "Завершить диагностику",
        }
    }

    /// Whether the entry is destructive and must be rendered in red.
    #[must_use]
    pub fn is_dangerous(self) -> bool {
        matches!(self, Self::Cleanup)
    }
}

/// Top-level UI state.
#[derive(Debug)]
pub struct App {
    /// Index of the highlighted menu entry.
    pub menu_index: usize,
    /// Pane that currently receives key presses.
    pub focus: Focus,
    /// Driver installation is running; the menu is locked meanwhile.
    pub is_installing: bool,
    /// The delete-everything confirmation modal is open.
    pub show_confirm_delete: bool,
    /// Whether the process has administrator rights.
    pub admin_status: bool,
    /// Whether Npcap is installed.
    pub npcap_status: bool,
    /// Whether the Loopback adapter is present.
    pub loopback_status: bool,
    /// Tunnel state shown in the status panel.
    pub connection_status: ConnectionStatus,
    /// Shared log buffer, filled by the `tracing` UI layer.
    pub logs: Arc<Mutex<VecDeque<String>>>,
    /// How many lines the log panel is scrolled up from the bottom.
    pub log_scroll: u16,
    /// Set when the event loop should exit.
    pub should_quit: bool,
}

impl App {
    /// Creates the initial state.
    #[must_use]
    pub fn new(logs: Arc<Mutex<VecDeque<String>>>) -> Self {
        Self {
            menu_index: 0,
            focus: Focus::Menu,
            is_installing: false,
            show_confirm_delete: false,
            admin_status: false,
            npcap_status: false,
            loopback_status: false,
            connection_status: ConnectionStatus::Disconnected,
            logs,
            log_scroll: 0,
            should_quit: false,
        }
    }

    /// Moves the highlight to the next entry, wrapping around.
    pub fn next(&mut self) {
        self.menu_index = (self.menu_index + 1) % MenuItem::ALL.len();
    }

    /// Moves the highlight to the previous entry, wrapping around.
    pub fn previous(&mut self) {
        let last = MenuItem::ALL.len() - 1;
        self.menu_index = if self.menu_index == 0 {
            last
        } else {
            self.menu_index - 1
        };
    }

    /// Currently highlighted menu entry.
    #[must_use]
    pub fn selected(&self) -> MenuItem {
        MenuItem::ALL[self.menu_index.min(MenuItem::ALL.len() - 1)]
    }

    /// Switches focus between the menu and the log panel.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Menu => Focus::Logs,
            Focus::Logs => Focus::Menu,
        };
    }

    /// Scrolls the log panel up by `lines`, clamping to the buffered content.
    pub fn scroll_logs_up(&mut self, lines: u16) {
        let buffered = self.logs.lock().map(|logs| logs.len()).unwrap_or(0);
        let max = buffered.saturating_sub(1).min(u16::MAX as usize) as u16;
        self.log_scroll = self.log_scroll.saturating_add(lines).min(max);
    }

    /// Scrolls the log panel down by `lines`, stopping at the newest entry.
    pub fn scroll_logs_down(&mut self, lines: u16) {
        self.log_scroll = self.log_scroll.saturating_sub(lines);
    }

    /// Updates the driver flags from a fresh probe.
    pub fn set_driver_status(&mut self, npcap: bool, loopback: bool) {
        self.npcap_status = npcap;
        self.loopback_status = loopback;
    }

    /// Whether every driver is available.
    #[must_use]
    pub fn drivers_ready(&self) -> bool {
        self.npcap_status && self.loopback_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(Arc::new(Mutex::new(VecDeque::new())))
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let mut app = app();

        assert_eq!(app.selected(), MenuItem::Install);

        for expected in [
            MenuItem::Car,
            MenuItem::Diag,
            MenuItem::Cleanup,
            MenuItem::Exit,
            MenuItem::Install,
        ] {
            app.next();
            assert_eq!(app.selected(), expected);
        }

        app.previous();
        assert_eq!(app.selected(), MenuItem::Exit);
    }

    #[test]
    fn focus_toggles_between_menu_and_logs() {
        let mut app = app();
        assert_eq!(app.focus, Focus::Menu);

        app.toggle_focus();
        assert_eq!(app.focus, Focus::Logs);

        app.toggle_focus();
        assert_eq!(app.focus, Focus::Menu);
    }

    #[test]
    fn only_cleanup_is_dangerous() {
        for item in MenuItem::ALL {
            assert_eq!(item.is_dangerous(), item == MenuItem::Cleanup);
        }
    }

    #[test]
    fn log_scroll_is_clamped_to_the_buffer() {
        let logs = Arc::new(Mutex::new(
            (0..10).map(|index| format!("line {index}")).collect(),
        ));
        let mut app = App::new(Arc::clone(&logs));

        app.scroll_logs_up(100);
        assert_eq!(app.log_scroll, 9);

        app.scroll_logs_down(4);
        assert_eq!(app.log_scroll, 5);

        app.scroll_logs_down(100);
        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn driver_status_combines_both_flags() {
        let mut app = app();

        app.set_driver_status(true, false);
        assert!(!app.drivers_ready());
        assert!(app.npcap_status && !app.loopback_status);

        app.set_driver_status(true, true);
        assert!(app.drivers_ready());
    }
}
