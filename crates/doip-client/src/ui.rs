//! Dashboard rendering.
//!
//! Layout: a header, a middle row split into the menu (left) and the status
//! panel (right), and the log console at the bottom. Modals are rendered last,
//! on top of everything else.

use crate::app::{App, ConnectionStatus, Focus, MenuItem};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;

/// Keys shown in the header together with the action they perform.
const HOTKEYS: [(&str, &str); 5] = [
    ("↑/↓", "меню"),
    ("Enter", "выбор"),
    ("Tab", "логи"),
    ("PgUp/PgDn", "прокрутка"),
    ("Ctrl+C", "выход"),
];

/// Draws the whole dashboard.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.size();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(50),
            Constraint::Percentage(40),
        ])
        .split(area);

    draw_header(frame, rows[0]);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(rows[1]);

    draw_menu(frame, columns[0], app);
    draw_statuses(frame, columns[1], app);
    draw_logs(frame, rows[2], app);

    if app.show_confirm_delete {
        draw_confirm_delete(frame, area);
    }

    if let Some(input) = &app.diag_input {
        draw_diag_input(frame, area, input);
    }

    if let Some(code) = &app.session_code {
        draw_session_code(frame, area, code);
    }
}

fn header_title() -> Line<'static> {
    Line::from(Span::styled(
        "=== DoIP Remote ===",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    ))
}

fn draw_header(frame: &mut Frame, area: Rect) {
    let hints = hotkeys_line();

    let header = Paragraph::new(vec![header_title(), hints]).alignment(Alignment::Center);
    frame.render_widget(header, area);

    // Version in the top-right corner, from the workspace package version.
    let version = version_label();
    let version_width = version.chars().count() as u16;
    if area.width > version_width + 1 && area.height > 0 {
        let rect = Rect {
            x: area.right() - version_width - 1,
            y: area.y,
            width: version_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(version).style(Style::default().fg(MUTED)),
            rect,
        );
    }
}

fn version_label() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// Builds the hotkey hint row: yellow key caps, cyan separators and actions.
fn hotkeys_line() -> Line<'static> {
    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(Color::Cyan);

    let mut spans = Vec::with_capacity(HOTKEYS.len() * 3);

    for (index, (key, action)) in HOTKEYS.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" • ", text_style));
        }
        spans.push(Span::styled(format!("[{key}]"), key_style));
        spans.push(Span::styled(format!(" - {action}"), text_style));
    }

    Line::from(spans)
}

fn draw_menu(frame: &mut Frame, area: Rect, app: &App) {
    let border = if app.focus == Focus::Menu {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };

    let title = if app.is_installing {
        Line::from(Span::styled(
            " Меню — установка идёт... ",
            Style::default().fg(Color::Yellow),
        ))
    } else {
        Line::from(" Меню ")
    };

    let lines = MenuItem::ALL
        .iter()
        .enumerate()
        .map(|(index, item)| menu_line(*item, index == app.menu_index, app))
        .collect::<Vec<_>>();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border);

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn menu_line(item: MenuItem, selected: bool, app: &App) -> Line<'static> {
    let prefix = if selected { "> " } else { "  " };
    let mut style = Style::default();

    if item.is_dangerous() {
        style = style.fg(Color::Red);
    }

    if selected {
        style = style.add_modifier(Modifier::BOLD);
        if app.focus == Focus::Menu {
            style = style.bg(Color::DarkGray).fg(Color::White);
            if item.is_dangerous() {
                style = style.fg(Color::LightRed);
            }
        }
    }

    if app.is_installing && !selected {
        style = style.fg(MUTED);
    }

    Line::from(Span::styled(format!("{prefix}{}", item.label()), style))
}

fn draw_statuses(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Статусы ");
    let width = block.inner(area).width as usize;

    let admin = ok_line("Права Админа", app.admin_status, width);
    let npcap = ok_line("Npcap", app.npcap_status, width);
    let loopback = ok_line("Loopback", app.loopback_status, width);
    let log_file = presence_line(
        "Log File",
        app.log_file_exists,
        "[EXISTS]",
        "[MISSING]",
        width,
    );

    let connection = match app.connection_status {
        ConnectionStatus::Connected => status_line(
            "Соединение",
            "[ЕСТЬ КОНТАКТ ✓]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            width,
        ),
        ConnectionStatus::Disconnected => status_line(
            "Соединение",
            "[ОТКЛЮЧЕНО]",
            Style::default().fg(Color::Red),
            width,
        ),
    };

    let lines = vec![admin, npcap, loopback, log_file, connection];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Builds an `[OK]` / `[НЕТ]` row. Anything that is not present is **red**:
/// a missing driver (or missing admin rights) must never look healthy.
fn ok_line(label: &str, present: bool, width: usize) -> Line<'static> {
    presence_line(label, present, "[OK]", "[НЕТ]", width)
}

fn presence_line(
    label: &str,
    present: bool,
    present_text: &str,
    missing_text: &str,
    width: usize,
) -> Line<'static> {
    if present {
        status_line(
            label,
            present_text,
            Style::default().fg(Color::Green),
            width,
        )
    } else {
        status_line(label, missing_text, Style::default().fg(Color::Red), width)
    }
}

/// Builds a status row aligned with dots: `Название........ [OK]`.
fn status_line(label: &str, value: &str, value_style: Style, width: usize) -> Line<'static> {
    let label_width = label.chars().count();
    let value_width = value.chars().count();
    let dots = width.saturating_sub(label_width + value_width + 2).max(1);

    Line::from(vec![
        Span::raw(label.to_owned()),
        Span::styled(".".repeat(dots), Style::default().fg(MUTED)),
        Span::raw(" "),
        Span::styled(value.to_owned(), value_style),
    ])
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App) {
    let border = if app.focus == Focus::Logs {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(MUTED)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title("[ ЛОГИ ]")
        .border_style(border);
    let height = block.inner(area).height as usize;

    let logs = app
        .logs
        .lock()
        .map(|logs| logs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    let lines = visible_logs(&logs, app.log_scroll as usize, height);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Picks the log lines to show, bottom-aligned, honouring the scroll offset.
fn visible_logs(logs: &[String], scroll: usize, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }

    let scroll = scroll.min(logs.len().saturating_sub(1));
    let end = logs.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);

    let mut lines: Vec<Line<'static>> =
        logs[start..end].iter().map(|line| log_line(line)).collect();

    // Push short logs to the bottom so the newest entry sits right above the
    // border, like a console.
    while lines.len() < height {
        lines.insert(0, Line::from(""));
    }

    lines
}

fn log_style(line: &str) -> Style {
    if line.contains("[ERROR]") {
        Style::default().fg(Color::Red)
    } else if line.contains("[WARN]") {
        Style::default().fg(Color::Yellow)
    } else if line.contains("[OK]") {
        Style::default().fg(Color::Green)
    } else if line.contains("[INFO]") {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().fg(Color::White)
    }
}

/// Renders one log line, highlighting the `[!]` admin warning marker in bold red.
fn log_line(line: &str) -> Line<'static> {
    let style = log_style(line);
    let Some(rest) = line.strip_prefix("[WARN] [!]") else {
        return Line::from(Span::styled(line.to_owned(), style));
    };

    Line::from(vec![
        Span::styled("[WARN]", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled(
            "[!]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(rest.to_owned(), Style::default().fg(Color::Yellow)),
    ])
}

fn draw_confirm_delete(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(64, 24, area);
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from(Span::styled(
            "Удалить драйверы и логи? (Y/N)",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Y — удалить, N — отмена",
            Style::default().fg(MUTED),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Подтверждение ")
        .border_style(Style::default().fg(Color::Red));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, popup);
}

/// Green line used for the generated session code.
fn code_line(code: &str) -> Line<'static> {
    Line::from(Span::styled(
        code.to_owned(),
        Style::default().fg(Color::Green),
    ))
}

fn draw_session_code(frame: &mut Frame, area: Rect, code: &str) {
    let popup = centered_rect(82, 32, area);
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from(Span::styled(
            "Код подключения Car:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        code_line(code),
        Line::from(""),
        Line::from(Span::styled(
            "Enter — копировать, Esc — закрыть",
            Style::default().fg(MUTED),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Сессия Car ")
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, popup);
}

fn draw_diag_input(frame: &mut Frame, area: Rect, input: &str) {
    let popup = centered_rect(74, 24, area);
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from(vec![
            Span::styled("Код: ", Style::default().fg(ACCENT)),
            Span::raw(input.to_owned()),
            Span::styled("█", Style::default().fg(ACCENT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter — принять, Esc — отмена",
            Style::default().fg(MUTED),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Код подключения ")
        .border_style(Style::default().fg(ACCENT));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, popup);
}

/// Returns a rectangle of the given percentage size, centered in `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_use_yellow_keys_and_cyan_descriptions() {
        let line = hotkeys_line();

        assert_eq!(line.spans[0].content, "[↑/↓]");
        assert_eq!(line.spans[0].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[1].content, " - меню");
        assert_eq!(line.spans[1].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[2].content, " • ");
        assert_eq!(line.spans[2].style.fg, Some(Color::Cyan));

        let plain: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(
            plain,
            "[↑/↓] - меню • [Enter] - выбор • [Tab] - логи • [PgUp/PgDn] - прокрутка • [Ctrl+C] - выход"
        );
    }

    #[test]
    fn visible_logs_are_bottom_aligned_and_padded() {
        let logs = vec!["one".to_owned(), "two".to_owned()];
        let lines = visible_logs(&logs, 0, 4);

        assert_eq!(lines.len(), 4);
        assert!(lines[0].spans.iter().all(|span| span.content.is_empty()));
        assert_eq!(lines[2].spans[0].content, "one");
        assert_eq!(lines[3].spans[0].content, "two");
    }

    #[test]
    fn scroll_moves_the_visible_window_up() {
        let logs = (0..10)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>();
        let lines = visible_logs(&logs, 2, 3);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content, "line 5");
        assert_eq!(lines[2].spans[0].content, "line 7");
    }

    #[test]
    fn log_styles_follow_the_level_tag() {
        assert_eq!(log_style("[ERROR] boom").fg, Some(Color::Red));
        assert_eq!(log_style("[WARN] hmm").fg, Some(Color::Yellow));
        assert_eq!(log_style("[OK] yes").fg, Some(Color::Green));
    }

    #[test]
    fn header_is_magenta_and_bold() {
        let title = header_title();

        assert_eq!(title.spans[0].content, "=== DoIP Remote ===");
        assert_eq!(title.spans[0].style.fg, Some(Color::Magenta));
        assert!(title.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn version_label_comes_from_cargo() {
        assert_eq!(version_label(), format!("v{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn missing_statuses_are_always_red() {
        for line in [
            ok_line("Права Админа", false, 30),
            ok_line("Npcap", false, 30),
            ok_line("Loopback", false, 30),
            presence_line("Log File", false, "[EXISTS]", "[MISSING]", 30),
        ] {
            let value = line.spans.last().expect("status value");
            assert_eq!(value.style.fg, Some(Color::Red));
        }

        let healthy = ok_line("Npcap", true, 30);
        assert_eq!(
            healthy.spans.last().expect("status value").style.fg,
            Some(Color::Green)
        );
    }

    #[test]
    fn log_file_row_uses_exists_and_missing_tags() {
        let missing = presence_line("Log File", false, "[EXISTS]", "[MISSING]", 30);
        assert_eq!(
            missing.spans.last().expect("value").content.as_ref(),
            "[MISSING]"
        );

        let present = presence_line("Log File", true, "[EXISTS]", "[MISSING]", 30);
        assert_eq!(
            present.spans.last().expect("value").content.as_ref(),
            "[EXISTS]"
        );
    }

    #[test]
    fn admin_warning_marker_is_bold_red() {
        let line = log_line("[WARN] [!] Нет прав администратора");
        let marker = &line.spans[2];

        assert_eq!(marker.content, "[!]");
        assert_eq!(marker.style.fg, Some(Color::Red));
        assert!(marker.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn generated_code_is_green() {
        let line = code_line("DT1-example");

        assert_eq!(line.spans[0].content, "DT1-example");
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn centered_rect_stays_inside_the_area() {
        let area = Rect::new(0, 0, 100, 40);
        let popup = centered_rect(60, 20, area);

        assert!(popup.x > 0 && popup.y > 0);
        assert!(popup.right() <= area.right());
        assert!(popup.bottom() <= area.bottom());
    }
}
