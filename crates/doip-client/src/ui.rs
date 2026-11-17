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
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;

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
}

fn draw_header(frame: &mut Frame, area: Rect) {
    let title = Line::from(Span::styled(
        "=== DoIP Remote ===",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    let hints = Line::from(Span::styled(
        "↑/↓ — меню • Enter — выбор • Tab — логи • PgUp/PgDn — прокрутка • Ctrl+C — выход",
        Style::default().fg(MUTED),
    ));

    let header = Paragraph::new(vec![title, hints]).alignment(Alignment::Center);
    frame.render_widget(header, area);
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

    let admin = if app.admin_status {
        status_line(
            "Права Админа",
            "[OK]",
            Style::default().fg(Color::Green),
            width,
        )
    } else {
        status_line(
            "Права Админа",
            "[НЕТ]",
            Style::default().fg(Color::Yellow),
            width,
        )
    };

    let npcap = driver_line("Npcap", app.npcap_status, width);
    let loopback = driver_line("Loopback", app.loopback_status, width);

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

    let lines = vec![admin, npcap, loopback, connection];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn driver_line(label: &str, installed: bool, width: usize) -> Line<'static> {
    if installed {
        status_line(label, "[OK]", Style::default().fg(Color::Green), width)
    } else {
        status_line(label, "[НЕТ]", Style::default().fg(Color::Red), width)
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

    let mut lines: Vec<Line<'static>> = logs[start..end]
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), log_style(line))))
        .collect();

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
            "Y — удалить и выйти, N — отмена",
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
    fn centered_rect_stays_inside_the_area() {
        let area = Rect::new(0, 0, 100, 40);
        let popup = centered_rect(60, 20, area);

        assert!(popup.x > 0 && popup.y > 0);
        assert!(popup.right() <= area.right());
        assert!(popup.bottom() <= area.bottom());
    }
}
