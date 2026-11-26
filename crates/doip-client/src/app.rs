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
pub enum ConnectionStatus {
    /// No tunnel has been established.
    Disconnected,
    /// The relay session is being negotiated, or a peer/local client is awaited.
    Connecting,
    /// The relay tunnel is paired and bytes are being forwarded.
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
    /// Remove the drivers and delete the log file without exiting.
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

    /// Human-readable menu label, including its numeric hotkey.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "1. Install Drivers",
            Self::Car => "2. I am near the car (Car)",
            Self::Diag => "3. I am a diagnostician (Diag)",
            Self::Cleanup => "4. Uninstall drivers and logs",
            Self::Exit => "5. Exit",
        }
    }

    /// Returns the menu item bound to a numeric hotkey, if any.
    #[must_use]
    pub fn from_hotkey(key: char) -> Option<Self> {
        match key {
            '1' => Some(Self::Install),
            '2' => Some(Self::Car),
            '3' => Some(Self::Diag),
            '4' => Some(Self::Cleanup),
            '5' => Some(Self::Exit),
            _ => None,
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
    /// Active-session exit confirmation modal is open.
    pub show_confirm_exit: bool,
    /// Whether the process has administrator rights.
    pub admin_status: bool,
    /// Whether Npcap is installed.
    pub npcap_status: bool,
    /// Whether the Loopback adapter is present.
    pub loopback_status: bool,
    /// Tunnel state shown in the status panel.
    pub connection_status: ConnectionStatus,
    /// A Car/Diag network session is starting or running.
    pub session_active: bool,
    /// Shared log buffer, filled by the `tracing` UI layer.
    pub logs: Arc<Mutex<VecDeque<String>>>,
    /// How many lines the log panel is scrolled up from the bottom.
    pub log_scroll: u16,
    /// Pending Diag session-code input, if the input popup is open.
    pub diag_input: Option<String>,
    /// Generated Car session code shown in a popup, if it is open.
    pub session_code: Option<String>,
    /// Whether `doip-remote.log` currently exists on disk.
    pub log_file_exists: bool,
    /// Remove the log file once the writer is closed (needed on Windows, where
    /// an open file cannot be deleted).
    pub delete_log_on_exit: bool,
    /// True after a successful cleanup; menu items 1-4 are disabled.
    pub is_cleaned_up: bool,
    /// Set when the event loop should exit.
    pub should_quit: bool,
    /// Set only by an explicit exit command. After cleanup, a `should_quit`
    /// without this flag is treated as stray state and cleared.
    pub exit_requested: bool,
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
            show_confirm_exit: false,
            admin_status: false,
            npcap_status: false,
            loopback_status: false,
            connection_status: ConnectionStatus::Disconnected,
            session_active: false,
            logs,
            log_scroll: 0,
            diag_input: None,
            session_code: None,
            log_file_exists: false,
            delete_log_on_exit: false,
            is_cleaned_up: false,
            should_quit: false,
            exit_requested: false,
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

    /// Updates the log-file flag shown in the status panel.
    pub fn set_log_file_exists(&mut self, exists: bool) {
        self.log_file_exists = exists;
    }

    /// Marks the start of a Car/Diag session before the network task runs.
    pub fn begin_session(&mut self) {
        self.session_active = true;
        self.connection_status = ConnectionStatus::Connecting;
    }

    /// Marks the end of a session and returns the status to `Disconnected`.
    pub fn finish_session(&mut self) {
        self.session_active = false;
        self.connection_status = ConnectionStatus::Disconnected;
    }

    /// Applies the statuses observed after driver cleanup.
    ///
    /// Cleanup is deliberately non-destructive to the session: this only
    /// refreshes the values shown in the status panel. In particular it never
    /// touches [`App::should_quit`] or the confirmation popup state, so the main
    /// event loop keeps running and the user decides when to exit.
    pub fn apply_cleanup_status(&mut self, npcap: bool, loopback: bool, log_exists: bool) {
        self.set_driver_status(npcap, loopback);
        self.set_log_file_exists(log_exists);
        self.finish_session();

        debug_assert!(
            !self.should_quit,
            "cleanup must never request application shutdown"
        );
    }

    /// Keeps the event loop alive after cleanup unless the user explicitly
    /// asked to exit.
    ///
    /// Cleanup must not be able to terminate the process through stale state
    /// (for example a queued `should_quit` or an old exit modal). Only
    /// [`App::exit_requested`], which is set by an explicit exit command, may
    /// break the loop once [`App::is_cleaned_up`] is true.
    ///
    /// Returns `true` when a stray quit request was blocked.
    pub fn block_stray_quit_after_cleanup(&mut self) -> bool {
        if self.is_cleaned_up && self.should_quit && !self.exit_requested {
            self.should_quit = false;
            self.show_confirm_exit = false;
            return true;
        }

        false
    }

    /// Opens the popup that shows a freshly generated Car session code.
    pub fn set_session_code(&mut self, code: String) {
        self.session_code = Some(code);
    }

    /// Pretends that admin rights and drivers are available.
    ///
    /// Used by the `--mock` command-line flag to preview the dashboard on a
    /// development machine. The network status is deliberately **not** faked:
    /// there is no relay yet, so the status stays `Disconnected`.
    pub fn enable_mock(&mut self) {
        self.admin_status = true;
        self.npcap_status = true;
        self.loopback_status = true;
        self.connection_status = ConnectionStatus::Disconnected;
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
    fn sessions_transition_through_connecting() {
        let mut app = app();

        app.begin_session();
        assert!(app.session_active);
        assert_eq!(app.connection_status, ConnectionStatus::Connecting);

        app.connection_status = ConnectionStatus::Connected;
        app.finish_session();

        assert!(!app.session_active);
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn cleanup_status_refresh_does_not_quit_the_app() {
        let mut app = app();
        app.set_driver_status(true, true);
        app.set_log_file_exists(true);
        app.connection_status = ConnectionStatus::Connected;
        app.session_active = true;
        app.show_confirm_delete = true;

        app.apply_cleanup_status(false, false, false);

        assert!(!app.npcap_status);
        assert!(!app.loopback_status);
        assert!(!app.log_file_exists);
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
        assert!(!app.session_active);
        assert!(!app.should_quit, "cleanup must keep the event loop alive");
        assert!(
            app.show_confirm_delete,
            "closing the modal is the caller's responsibility"
        );
    }

    #[test]
    fn cleanup_fence_blocks_stray_quit_but_allows_explicit_exit() {
        let mut app = app();
        app.is_cleaned_up = true;
        app.should_quit = true;
        app.show_confirm_exit = true;

        assert!(app.block_stray_quit_after_cleanup());
        assert!(!app.should_quit);
        assert!(!app.show_confirm_exit);

        app.exit_requested = true;
        app.should_quit = true;

        assert!(!app.block_stray_quit_after_cleanup());
        assert!(app.should_quit);
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

    #[test]
    fn mock_mode_turns_admin_and_drivers_green_but_not_the_network() {
        let mut app = app();
        app.enable_mock();

        assert!(app.admin_status);
        assert!(app.npcap_status);
        assert!(app.loopback_status);
        assert!(app.drivers_ready());
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn cleanup_and_exit_are_separate_entries() {
        assert_eq!(MenuItem::Cleanup.label(), "4. Uninstall drivers and logs");
        assert_eq!(MenuItem::Exit.label(), "5. Exit");
        assert!(MenuItem::Cleanup.is_dangerous());
        assert!(!MenuItem::Exit.is_dangerous());
    }

    #[test]
    fn numeric_hotkeys_map_to_menu_items() {
        assert_eq!(MenuItem::from_hotkey('1'), Some(MenuItem::Install));
        assert_eq!(MenuItem::from_hotkey('2'), Some(MenuItem::Car));
        assert_eq!(MenuItem::from_hotkey('3'), Some(MenuItem::Diag));
        assert_eq!(MenuItem::from_hotkey('4'), Some(MenuItem::Cleanup));
        assert_eq!(MenuItem::from_hotkey('5'), Some(MenuItem::Exit));
        assert_eq!(MenuItem::from_hotkey('0'), None);
        assert_eq!(MenuItem::from_hotkey('6'), None);
    }

    #[test]
    fn session_code_popup_and_log_file_status_are_tracked() {
        let mut app = app();

        assert!(app.session_code.is_none());
        assert!(!app.log_file_exists);

        app.set_session_code("DT1-test".to_owned());
        app.set_log_file_exists(true);

        assert_eq!(app.session_code.as_deref(), Some("DT1-test"));
        assert!(app.log_file_exists);
    }
}
