//! Detection and installation of the Windows drivers used for L2 access.
//!
//! The L2 path needs **Npcap** (packet capture) and a **Microsoft KM-TEST
//! Loopback Adapter**. Both are Windows-only; other platforms get a stub so the
//! client can still be built and exercised on Linux.

#[cfg(target_os = "windows")]
use std::process::{Command, ExitStatus};

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

/// Errors that can occur while installing the drivers.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Some variants only exist on specific platforms.
pub enum DriverError {
    /// Driver installation is implemented for Windows only.
    #[error("driver installation is only supported on Windows")]
    UnsupportedPlatform,

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

/// Checks which drivers are currently installed.
#[cfg(target_os = "windows")]
#[must_use]
pub fn check_drivers() -> DriverStatus {
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
pub fn check_drivers() -> DriverStatus {
    DriverStatus {
        npcap_installed: false,
        loopback_installed: false,
    }
}

/// Downloads and installs the missing drivers.
///
/// Npcap is downloaded from the official site and installed silently (`/S`).
/// The KM-TEST Loopback Adapter is installed from `netloop.inf`. Both steps
/// wait for the installer to finish. On non-Windows platforms this returns
/// [`DriverError::UnsupportedPlatform`] without side effects.
#[cfg(target_os = "windows")]
pub fn install_drivers() -> Result<(), DriverError> {
    let status = check_drivers();
    if status.is_all_ok() {
        return Ok(());
    }

    if !status.npcap_installed {
        install_npcap()?;
    }

    if !status.loopback_installed {
        install_loopback()?;
    }

    Ok(())
}

/// Downloads and installs the missing drivers.
///
/// Stub for non-Windows platforms.
#[cfg(not(target_os = "windows"))]
pub fn install_drivers() -> Result<(), DriverError> {
    Err(DriverError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
const NPCAP_DOWNLOAD_URL: &str = "https://npcap.com/dist/npcap-1.80.exe";

#[cfg(any(target_os = "windows", test))]
const LOOPBACK_MARKERS: [&str; 2] = ["loopback", "km-test"];

#[cfg(target_os = "windows")]
fn windows_dir() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
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
    let installer_path = std::env::temp_dir().join("npcap-1.80.exe");

    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("doip-remote/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let bytes = client
        .get(NPCAP_DOWNLOAD_URL)
        .send()?
        .error_for_status()?
        .bytes()?;
    std::fs::write(&installer_path, bytes.as_ref())?;

    // Make sure the downloaded installer is removed even if it cannot start.
    let status = Command::new(&installer_path).arg("/S").status();
    let _ = std::fs::remove_file(&installer_path);
    let status = status?;

    if status.success() {
        Ok(())
    } else {
        Err(DriverError::InstallerFailed {
            name: "Npcap",
            status,
        })
    }
}

#[cfg(target_os = "windows")]
fn install_loopback() -> Result<(), DriverError> {
    let inf_default_install = windows_dir().join("System32").join("InfDefaultInstall.exe");
    let netloop_inf = windows_dir().join("INF").join("netloop.inf");

    let status = Command::new(inf_default_install)
        .arg(netloop_inf)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(DriverError::InstallerFailed {
            name: "Loopback",
            status,
        })
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
        let status = check_drivers();

        assert!(!status.npcap_installed);
        assert!(!status.loopback_installed);
        assert!(!status.is_all_ok());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_install_reports_unsupported_platform() {
        let error = install_drivers().expect_err("installation must be unsupported");

        assert!(matches!(error, DriverError::UnsupportedPlatform));
    }
}
