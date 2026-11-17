//! Interactive terminal client for `doip-remote`.

mod drivers;

use std::{fmt, process::ExitCode};

use colored::Colorize;
use doip_core::{Role, SessionCode};
use drivers::{DriverError, DriverStatus};
use inquire::{InquireError, Select, Text};

/// Placeholder relay address until the networking layer is implemented.
const RELAY_SERVER: &str = "auto";

/// Main menu entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Car,
    Diag,
    Install,
    Cleanup,
    Exit,
}

impl MenuAction {
    const ALL: [Self; 5] = [
        Self::Car,
        Self::Diag,
        Self::Install,
        Self::Cleanup,
        Self::Exit,
    ];
}

impl fmt::Display for MenuAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Car => "Я рядом с машиной (Car)",
            Self::Diag => "Я диагност (Diag)",
            Self::Install => "Установить драйверы (Install)",
            Self::Cleanup => "Удалить драйверы (Очистка)",
            Self::Exit => "Выход",
        })
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    print_banner();

    let mut driver_status = drivers::check_drivers();
    print_driver_status(&driver_status);
    println!();

    let status = run_menu(&mut driver_status).await;

    println!();
    println!("{}", "До встречи!".bright_black());
    status
}

async fn run_menu(driver_status: &mut DriverStatus) -> ExitCode {
    loop {
        let action = match Select::new("Главное меню:", MenuAction::ALL.to_vec())
            .with_help_message("↑/↓ — навигация, Enter — выбор, Esc — выход")
            .with_page_size(MenuAction::ALL.len())
            .prompt()
        {
            Ok(action) => action,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("{} {error}", "[ERROR]".red().bold());
                return ExitCode::FAILURE;
            }
        };

        let result = match action {
            MenuAction::Car => handle_car(),
            MenuAction::Diag => handle_diag(),
            MenuAction::Install => {
                handle_install(driver_status).await;
                Ok(())
            }
            MenuAction::Cleanup => {
                handle_cleanup();
                Ok(())
            }
            MenuAction::Exit => return ExitCode::SUCCESS,
        };

        match result {
            Ok(()) => {}
            Err(InquireError::OperationInterrupted) => return ExitCode::SUCCESS,
            Err(InquireError::OperationCanceled) => {}
            Err(error) => {
                eprintln!("{} {error}", "[ERROR]".red().bold());
                return ExitCode::FAILURE;
            }
        }

        println!();
    }
}

/// Generates a fresh session code for the vehicle side.
fn handle_car() -> Result<(), InquireError> {
    let input = Text::new("Введите IP автомобиля (Enter для автоопределения): ")
        .with_help_message("Оставьте поле пустым, чтобы определить IP автоматически")
        .prompt()?;

    let vehicle_ip = {
        let input = input.trim();
        (!input.is_empty()).then(|| input.to_owned())
    };

    let session = SessionCode::new(Role::Car, RELAY_SERVER, vehicle_ip);
    let code = session.generate();

    println!();
    println!(
        "{}",
        "Сессия создана. Передайте код диагносту:".green().bold()
    );
    println!();
    print_code_box(&code);
    println!();
    println!(
        "{}",
        "Ожидание подключения к серверу (заглушка)... Нажмите Ctrl+C для выхода".yellow()
    );

    wait_for_enter()
}

/// Accepts a session code on the diagnostic side.
fn handle_diag() -> Result<(), InquireError> {
    let input = Text::new("Введите код подключения DT1-...: ")
        .with_help_message("Вставьте код, полученный от владельца автомобиля")
        .prompt()?;

    println!();

    match SessionCode::parse(&input) {
        Ok(session) => {
            println!("{}", "Код принят.".green().bold());
            println!(
                "Целевой IP авто: {:?}, Порт DoIP: {}",
                session.vehicle_ip, session.doip_port
            );
            println!("Сервер: {}", session.server);
            println!();
            println!("{}", "Ожидание подключения...".yellow());

            wait_for_enter()
        }
        Err(error) => {
            eprintln!("{} {}", "[ERROR]".red().bold(), error.to_string().red());
            wait_for_enter()
        }
    }
}

/// Downloads and installs missing drivers, then refreshes the status.
async fn handle_install(driver_status: &mut DriverStatus) {
    println!();
    println!("Скачивание и установка драйверов...");

    // On non-Windows builds `UnsupportedPlatform` is the only error variant,
    // so the catch-all arm is unreachable there.
    #[allow(unreachable_patterns)]
    match tokio::task::spawn_blocking(drivers::install_drivers).await {
        Ok(Ok(())) => println!("{}", "[OK] Установка завершена. Проверьте статус.".green()),
        Ok(Err(DriverError::UnsupportedPlatform)) => {
            println!(
                "{}",
                "[STUB] Установка драйверов поддерживается только на Windows.".yellow()
            );
        }
        Ok(Err(error)) => eprintln!("{} {error}", "[ERROR]".red().bold()),
        Err(error) => eprintln!("{} install task failed: {error}", "[ERROR]".red().bold()),
    }

    *driver_status = drivers::check_drivers();
    println!();
    print_driver_status(driver_status);
}

/// Driver cleanup placeholder.
fn handle_cleanup() {
    println!();
    println!("{} Removing drivers...", "[STUB]".yellow().bold());
    println!("{}", "Заглушка: драйверы не были изменены.".bright_black());
}

fn print_banner() {
    println!(
        "{}",
        format!("=== Rust DoIP Remote v{} ===", env!("CARGO_PKG_VERSION"))
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics".bright_black()
    );
    println!();
}

/// Prints the current driver status above the main menu.
fn print_driver_status(status: &DriverStatus) {
    if status.is_all_ok() {
        println!(
            "{}",
            "[OK] Все драйверы установлены (Npcap, Loopback).".green()
        );
        return;
    }

    let mut missing = Vec::with_capacity(2);
    if !status.npcap_installed {
        missing.push("Npcap");
    }
    if !status.loopback_installed {
        missing.push("Loopback");
    }

    println!(
        "{}",
        format!("[!] ВНИМАНИЕ: Отсутствуют драйверы: {}", missing.join(", "))
            .red()
            .bold()
    );

    if !status.npcap_installed {
        println!("    - Npcap (скачать с npcap.com)");
    }
    if !status.loopback_installed {
        println!("    - Microsoft KM-TEST Loopback Adapter (устанавливается через меню)");
    }

    println!(
        "    Вы можете установить их через меню (пункт 3 — «Установить драйверы») или вручную из интернета."
    );
}

/// Prints a session code inside a green frame.
///
/// Long codes are wrapped at 72 characters so the frame fits a standard
/// 80-column terminal. The parser ignores whitespace, so a wrapped code can
/// still be pasted back.
fn print_code_box(code: &str) {
    const TITLE: &str = "Код подключения (передайте его диагносту):";
    const CODE_WRAP_WIDTH: usize = 72;

    let characters: Vec<char> = code.chars().collect();
    let code_lines: Vec<String> = if characters.is_empty() {
        vec![String::new()]
    } else {
        characters
            .chunks(CODE_WRAP_WIDTH)
            .map(|chunk| chunk.iter().collect())
            .collect()
    };

    let content_width = code_lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default()
        .max(TITLE.chars().count());

    let inner_width = content_width + 2;
    let border = "═".repeat(inner_width);

    println!("{}", format!("╔{border}╗").green());
    println!("{}", format!("║ {TITLE:<content_width$} ║").green());
    for line in code_lines {
        println!(
            "{}",
            format!("║ {line:<content_width$} ║").bright_green().bold()
        );
    }
    println!("{}", format!("╚{border}╝").green());
}

fn wait_for_enter() -> Result<(), InquireError> {
    Text::new("Нажмите Enter, чтобы вернуться в меню...")
        .prompt()
        .map(|_| ())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_target(false)
        .without_time()
        .try_init();
}
