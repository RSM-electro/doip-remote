//! Detection and installation of the Windows drivers used for L2 access.
//!
//! The L2 path needs **Npcap** (packet capture) and a **Microsoft KM-TEST
//! Loopback Adapter**. Both are Windows-only; other platforms get a stub so the
//! client can still be built and exercised on Linux.
//!
//! Installation is split into a background-friendly API: [`check_all`] probes
//! the system, while [`install_all_async`] spawns the work on a blocking thread
//! and reports completion through an mpsc channel. Progress and installer
//! output are emitted with `tracing`, so they appear both in the log file and in
//! the TUI log panel.

#[cfg(target_os = "windows")]
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use tokio::sync::mpsc::UnboundedSender;

/// Installation status of the drivers required for L2 traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverStatus {
    /// Whether Npcap (`wpcap.dll`) is installed.
    pub npcap_installed: bool,
    /// Whether a Loopback / KM-TEST network adapter is present.
    pub loopback_installed: bool,
}

impl DriverStatus {
    /// Returns `true` when every required driver is available.
    #[must_use]
    pub fn is_all_ok(&self) -> bool {
        self.npcap_installed && self.loopback_installed
    }
}

/// Event sent by the background installer when it finishes.
#[derive(Debug)]
pub enum InstallEvent {
    /// Installation finished; the UI can refresh the statuses and unlock.
    Finished(Result<(), DriverError>),
}

/// Errors that can occur while installing the drivers.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Some variants only exist on specific platforms.
pub enum DriverError {
    /// Driver installation is implemented for Windows only.
    #[error("driver installation is only supported on Windows")]
    UnsupportedPlatform,

    /// The background installation task could not be joined.
    #[error("driver installation task failed: {0}")]
    Task(String),

    /// The Npcap installer could not be downloaded.
    #[cfg(target_os = "windows")]
    #[error("failed to download the Npcap installer: {0}")]
    Download(#[from] reqwest::Error),

    /// A local file operation or process launch failed.
    #[cfg(target_os = "windows")]
    #[error("driver installation failed: {0}")]
    Io(#[from] std::io::Error),

    /// An installer finished with a non-success exit status.
    #[cfg(target_os = "windows")]
    #[error("{name} installer failed with status {status}")]
    InstallerFailed {
        /// Human-readable installer name.
        name: &'static str,
        /// Exit status returned by the installer.
        status: ExitStatus,
    },
}

/// Removes the L2 drivers.
///
/// On Windows this runs the official Npcap uninstaller
/// (`C:\Program Files\Npcap\NPFInstall.exe -u`) and waits for it to finish.
/// The client never opens Npcap files: detection only checks whether `wpcap.dll`
/// exists, so no capture handle can keep the uninstaller from replacing it.
#[cfg(target_os = "windows")]
pub fn remove_all() -> Result<(), DriverError> {
    let uninstaller = PathBuf::from(NPCAP_UNINSTALLER);

    if uninstaller.is_file() {
        tracing::info!("Запуск {} -u ...", uninstaller.display());
        let status = Command::new(&uninstaller).arg("-u").status()?;

        if !status.success() {
            return Err(DriverError::InstallerFailed {
                name: "Npcap uninstall",
                status,
            });
        }

        tracing::info!("[OK] Npcap удалён.");
    } else {
        tracing::info!("Npcap не установлен, удалять нечего.");
    }

    // The KM-TEST adapter lives in the driver store and is harmless without a
    // capture session; its removal path is not wired yet.
    tracing::info!("Адаптер Loopback оставлен без изменений.");
    Ok(())
}

/// Removes the L2 drivers.
///
/// The L2 drivers are Windows-only, so this is a no-op elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn remove_all() -> Result<(), DriverError> {
    tracing::warn!("Удаление драйверов поддерживается только на Windows.");
    Ok(())
}

/// Checks which drivers are currently installed.
#[cfg(target_os = "windows")]
#[must_use]
pub fn check_all() -> DriverStatus {
    DriverStatus {
        npcap_installed: npcap_installed(),
        loopback_installed: loopback_installed(),
    }
}

/// Checks which drivers are currently installed.
///
/// This is a stub on non-Windows platforms: the L2 drivers are Windows-only.
#[cfg(not(target_os = "windows"))]
#[must_use]
pub fn check_all() -> DriverStatus {
    DriverStatus {
        npcap_installed: false,
        loopback_installed: false,
    }
}

/// Spawns a background task that installs the missing drivers.
///
/// Completion (success or failure) is reported through `events`. The caller
/// should keep the menu locked until [`InstallEvent::Finished`] arrives.
pub fn install_all_async(events: UnboundedSender<InstallEvent>) {
    tokio::spawn(async move {
        let result = match tokio::task::spawn_blocking(install_all_blocking).await {
            Ok(result) => result,
            Err(error) => Err(DriverError::Task(error.to_string())),
        };

        let _ = events.send(InstallEvent::Finished(result));
    });
}

/// Blocking installation body; runs on a dedicated thread.
#[cfg(target_os = "windows")]
fn install_all_blocking() -> Result<(), DriverError> {
    let status = check_all();
    if status.is_all_ok() {
        tracing::info!("[OK] Все драйверы уже установлены.");
        return Ok(());
    }

    if status.npcap_installed {
        tracing::info!("Npcap уже установлен, пропускаем.");
    } else {
        tracing::info!("Npcap не найден. Скачивание установщика с npcap.com...");
        install_npcap()?;
    }

    if status.loopback_installed {
        tracing::info!("Loopback-адаптер уже установлен, пропускаем.");
    } else {
        tracing::info!("KM-TEST Loopback Adapter не найден. Запуск установки...");
        install_loopback()?;
    }

    Ok(())
}

/// Blocking installation body; stub outside Windows.
#[cfg(not(target_os = "windows"))]
fn install_all_blocking() -> Result<(), DriverError> {
    tracing::warn!("Установка драйверов поддерживается только на Windows.");
    Err(DriverError::UnsupportedPlatform)
}

/// Returns `true` when the current process has administrator privileges.
///
/// On Windows this runs `net session`, which only succeeds for elevated
/// processes. The console window is suppressed.
#[cfg(target_os = "windows")]
#[must_use]
pub fn is_admin() -> bool {
    use std::os::windows::process::CommandExt;

    /// Do not flash a console window while probing.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new("net")
        .args(["session"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Returns `true` when the current process has administrator privileges.
///
/// Always `false` outside Windows.
#[cfg(not(target_os = "windows"))]
#[must_use]
pub fn is_admin() -> bool {
    false
}

#[cfg(target_os = "windows")]
const NPCAP_DOWNLOAD_URL: &str = "https://npcap.com/dist/npcap-1.80.exe";

/// Official Npcap uninstaller; `-u` removes the driver and the service.
#[cfg(target_os = "windows")]
const NPCAP_UNINSTALLER: &str = r"C:\Program Files\Npcap\NPFInstall.exe";

#[cfg(any(target_os = "windows", test))]
const LOOPBACK_MARKERS: [&str; 2] = ["loopback", "km-test"];

#[cfg(target_os = "windows")]
fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

#[cfg(target_os = "windows")]
fn npcap_installed() -> bool {
    windows_dir()
        .join("System32")
        .join("Npcap")
        .join("wpcap.dll")
        .is_file()
}

#[cfg(target_os = "windows")]
fn loopback_installed() -> bool {
    // Get-NetAdapter exposes both the adapter name and its description, which
    // covers adapters renamed by the user as well as the driver's own name.
    let powershell = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-NetAdapter | Select-Object -Property Name, InterfaceDescription | Out-String -Width 4096",
        ])
        .output();

    if let Ok(output) = powershell {
        if output.status.success() && contains_loopback_marker(&output.stdout) {
            return true;
        }
    }

    // netsh is a lighter fallback and also lists adapters on older systems.
    Command::new("netsh")
        .args(["interface", "show", "interface"])
        .output()
        .map(|output| output.status.success() && contains_loopback_marker(&output.stdout))
        .unwrap_or(false)
}

#[cfg(any(target_os = "windows", test))]
fn contains_loopback_marker(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    LOOPBACK_MARKERS.iter().any(|marker| text.contains(marker))
}

#[cfg(target_os = "windows")]
fn install_npcap() -> Result<(), DriverError> {
    let installer = std::env::temp_dir().join("npcap-1.80.exe");
    download_npcap(&installer)?;

    tracing::info!("Запуск установщика Npcap. Следуйте подсказкам мастера...");
    let status = Command::new(&installer).status();

    // The installer is always a temporary file; do not leave it behind.
    let _ = std::fs::remove_file(&installer);
    let status = status?;

    if status.success() {
        tracing::info!("[OK] Установщик Npcap завершился успешно.");
        Ok(())
    } else {
        Err(DriverError::InstallerFailed {
            name: "Npcap",
            status,
        })
    }
}

#[cfg(target_os = "windows")]
fn download_npcap(destination: &Path) -> Result<(), DriverError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("doip-remote/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut response = client.get(NPCAP_DOWNLOAD_URL).send()?.error_for_status()?;

    let total = response.content_length();
    let mut file = File::create(destination)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut next_percent: u64 = 10;

    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])?;
        downloaded += read as u64;

        match total {
            Some(total) if total > 0 => {
                let percent = downloaded * 100 / total;
                if percent >= next_percent || downloaded == total {
                    tracing::info!("Загрузка Npcap: {percent}% ({downloaded} из {total} байт)");
                    next_percent = (percent / 10 + 1) * 10;
                }
            }
            _ => {}
        }
    }

    file.flush()?;
    tracing::info!(
        "[OK] Npcap скачан ({} байт): {}",
        downloaded,
        destination.display()
    );
    Ok(())
}

/// Installs the Microsoft KM-TEST Loopback Adapter through `newdev.dll`.
///
/// `InfDefaultInstall.exe` only stages the INF; it does not create the device.
/// `UpdateDriverForPlugAndPlayDevicesW` performs the full "have disk" install
/// and binds the `*msloop` hardware ID to `netloop.inf`.
#[cfg(target_os = "windows")]
fn install_loopback() -> Result<(), DriverError> {
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        UpdateDriverForPlugAndPlayDevicesW, INSTALLFLAG_FORCE, INSTALLFLAG_NONINTERACTIVE,
    };

    let netloop_inf = windows_dir().join("INF").join("netloop.inf");
    if !netloop_inf.is_file() {
        return Err(DriverError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} not found", netloop_inf.display()),
        )));
    }

    let inf_path: Vec<u16> = netloop_inf
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let hardware_id: Vec<u16> = "*msloop".encode_utf16().chain(std::iter::once(0)).collect();

    let flags = INSTALLFLAG_FORCE | INSTALLFLAG_NONINTERACTIVE;
    let mut reboot_required = BOOL::default();

    tracing::info!(
        "Установка Loopback через UpdateDriverForPlugAndPlayDevicesW ({}) ...",
        netloop_inf.display()
    );

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the
    // call; `reboot_required` points to a valid BOOL for the duration.
    let result = unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            None,
            PCWSTR::from_raw(hardware_id.as_ptr()),
            PCWSTR::from_raw(inf_path.as_ptr()),
            flags,
            Some(&mut reboot_required),
        )
    };

    match result {
        Ok(()) => {
            if reboot_required.as_bool() {
                tracing::info!("[!] Для завершения установки Loopback нужна перезагрузка.");
            }
            tracing::info!("[OK] KM-TEST Loopback Adapter установлен.");
            Ok(())
        }
        Err(error) => Err(DriverError::Io(std::io::Error::other(format!(
            "UpdateDriverForPlugAndPlayDevicesW failed: {error}"
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ok_requires_both_drivers() {
        assert!(DriverStatus {
            npcap_installed: true,
            loopback_installed: true,
        }
        .is_all_ok());
        assert!(!DriverStatus {
            npcap_installed: true,
            loopback_installed: false,
        }
        .is_all_ok());
        assert!(!DriverStatus {
            npcap_installed: false,
            loopback_installed: true,
        }
        .is_all_ok());
        assert!(!DriverStatus {
            npcap_installed: false,
            loopback_installed: false,
        }
        .is_all_ok());
    }

    #[test]
    fn detects_loopback_markers_case_insensitively() {
        assert!(contains_loopback_marker(
            b"Microsoft KM-TEST Loopback Adapter"
        ));
        assert!(contains_loopback_marker(b"Ethernet 2\r\nLOOPBACK"));
        assert!(!contains_loopback_marker(b"Intel(R) Ethernet Connection"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_check_is_a_stub() {
        let status = check_all();

        assert!(!status.npcap_installed);
        assert!(!status.loopback_installed);
        assert!(!status.is_all_ok());
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn non_windows_install_reports_unsupported_platform() {
        let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        install_all_async(events);

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
            .await
            .expect("install event timed out")
            .expect("install channel closed");

        match event {
            InstallEvent::Finished(Err(DriverError::UnsupportedPlatform)) => {}
            other => panic!("unexpected install event: {other:?}"),
        }
    }
}
