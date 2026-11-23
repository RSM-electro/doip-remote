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
/// (`C:\Program Files\Npcap\NPFInstall.exe -n -u`) and waits for it to finish,
/// then removes the KM-TEST Loopback devnode that was created from
/// `netloop.inf` with the `*MSLOOP` hardware ID. The client never opens Npcap
/// files: detection only checks whether `wpcap.dll` exists, so no capture
/// handle can keep the uninstaller from replacing it.
///
/// Both removals are attempted independently: a failed Npcap uninstall must
/// never prevent the Loopback adapter from being removed.
#[cfg(target_os = "windows")]
pub fn remove_all() -> Result<(), DriverError> {
    let npcap = uninstall_npcap();
    let loopback = remove_loopback();

    match (npcap, loopback) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(npcap_error), Err(loopback_error)) => {
            tracing::warn!("Loopback removal also failed: {loopback_error}");
            Err(npcap_error)
        }
    }
}

/// Runs the official Npcap uninstaller, then deletes any driver files it left
/// behind once the `npcap` service is gone.
#[cfg(target_os = "windows")]
fn uninstall_npcap() -> Result<(), DriverError> {
    let uninstaller = PathBuf::from(NPCAP_UNINSTALLER);

    let failure = if uninstaller.is_file() {
        // The original installer uses `-n -u`; plain `-u` can return -1 on
        // Windows 11.
        tracing::info!("Запуск {} -n -u ...", uninstaller.display());
        match Command::new(&uninstaller).args(["-n", "-u"]).status() {
            Ok(status) if status.success() => {
                tracing::info!("[OK] Npcap удалён.");
                None
            }
            Ok(status) => Some(DriverError::InstallerFailed {
                name: "Npcap uninstall",
                status,
            }),
            Err(error) => Some(DriverError::Io(error)),
        }
    } else {
        tracing::info!("Npcap uninstaller not found; checking for stale files.");
        None
    };

    // NPFInstall can report success yet leave `wpcap.dll` behind, and fails with
    // "Couldn't get an interface pointer to INSECURE_NPCAP" when the LWF
    // component was already unregistered. In both cases the driver is gone once
    // its service is missing: delete the leftovers so detection (and therefore
    // the UI) reports Npcap as absent.
    if !npcap_service_present() {
        remove_stale_npcap_files()?;
    }

    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Deletes the `%SystemRoot%\System32\Npcap` folder left behind by Npcap.
#[cfg(target_os = "windows")]
fn remove_stale_npcap_files() -> Result<(), DriverError> {
    let stale_dir = windows_dir().join("System32").join("Npcap");
    match std::fs::remove_dir_all(&stale_dir) {
        Ok(()) => {
            tracing::info!(
                "[OK] Stale Npcap files removed from {}.",
                stale_dir.display()
            );
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("Npcap files already absent.");
            Ok(())
        }
        Err(error) => Err(DriverError::Io(error)),
    }
}

/// Returns `true` when the `npcap` driver service still exists.
///
/// `sc.exe` exits with 1060 when the service is not registered; any other
/// outcome is treated as "present" so a query failure never deletes live files.
#[cfg(target_os = "windows")]
fn npcap_service_present() -> bool {
    Command::new("sc.exe")
        .args(["query", "npcap"])
        .output()
        .map_or(true, |output| {
            output.status.success() || output.status.code() != Some(1060)
        })
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
/// `UpdateDriverForPlugAndPlayDevicesW` only *updates* the driver of a device
/// that already exists. Calling it with `*msloop` while there is no devnode
/// with that hardware ID fails with `0xE000020B` and the setupapi message
/// `Unable to find any matching devices`. The original installer succeeded
/// because it first created a root-enumerated `ROOT\NET\XXXX` devnode and
/// registered it. We reproduce that sequence here:
///
/// 1. reuse an existing `*MSLOOP` devnode if a previous attempt left one;
/// 2. otherwise create `ROOT\NET\<generated>` with `SPDRP_HARDWAREID=*MSLOOP`
///    and register it via `DIF_REGISTERDEVICE`;
/// 3. bind `netloop.inf` to that device with `UpdateDriverForPlugAndPlayDevicesW`.
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
    let hardware_id: Vec<u16> = "*MSLOOP".encode_utf16().chain(std::iter::once(0)).collect();

    // The driver-update API cannot create a device. Make sure a matching
    // devnode exists first, otherwise it returns 0xE000020B.
    if msloop_devnode_exists()? {
        tracing::info!("Найден существующий devnode *MSLOOP, создание устройства пропущено.");
    } else {
        create_msloop_devnode()?;
    }

    let flags = INSTALLFLAG_FORCE | INSTALLFLAG_NONINTERACTIVE;
    let mut reboot_required = BOOL::default();

    tracing::info!(
        "Установка Loopback через UpdateDriverForPlugAndPlayDevicesW (INF: {}, HardwareID: *MSLOOP, flags: 0x{:X}) ...",
        netloop_inf.display(),
        flags.0
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
        Err(error) => Err(setup_error(
            "UpdateDriverForPlugAndPlayDevicesW failed",
            error,
        )),
    }
}

/// A `HDEVINFO` handle that is always released with `SetupDiDestroyDeviceInfoList`.
#[cfg(target_os = "windows")]
struct DeviceInfoSet(windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO);

#[cfg(target_os = "windows")]
impl DeviceInfoSet {
    fn as_raw(&self) -> windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO {
        self.0
    }
}

#[cfg(target_os = "windows")]
impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid info-set handle owned by this wrapper.
        unsafe {
            let _ =
                windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiDestroyDeviceInfoList(
                    self.0,
                );
        }
    }
}

/// Converts a SetupAPI error into a `DriverError` with the numeric code in the
/// message, so `0xE000020B` and friends are visible in the UI log.
#[cfg(target_os = "windows")]
fn setup_error(context: &str, error: windows::core::Error) -> DriverError {
    DriverError::Io(std::io::Error::other(format!(
        "{context}: {error} (HRESULT 0x{:08X})",
        error.code().0
    )))
}

/// Returns `true` when a network devnode with HardwareID `*MSLOOP` already
/// exists (for example, left behind by a failed previous attempt).
#[cfg(target_os = "windows")]
fn msloop_devnode_exists() -> Result<bool, DriverError> {
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
        GUID_DEVCLASS_NET, SETUP_DI_GET_CLASS_DEVS_FLAGS, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
    };

    // SAFETY: `PCWSTR::null()` is a valid null enumerator; the class GUID is static.
    let raw = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_NET),
            PCWSTR::null(),
            None,
            SETUP_DI_GET_CLASS_DEVS_FLAGS(0),
        )
    }
    .map_err(|error| setup_error("SetupDiGetClassDevsW failed", error))?;
    let set = DeviceInfoSet(raw);

    let mut index = 0_u32;
    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };

        // SAFETY: `data` is correctly initialized and `index` is just an ordinal.
        if unsafe { SetupDiEnumDeviceInfo(set.as_raw(), index, &mut data) }.is_err() {
            break;
        }
        index += 1;

        let mut buffer = [0_u16; 128];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 2)
        };

        // SAFETY: `bytes` points to `buffer`'s storage and matches its length.
        if unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set.as_raw(),
                &data,
                SPDRP_HARDWAREID,
                None,
                Some(bytes),
                None,
            )
        }
        .is_ok()
            && String::from_utf16_lossy(&buffer)
                .to_ascii_uppercase()
                .contains("*MSLOOP")
        {
            tracing::info!("Обнаружен devnode *MSLOOP (DevInst {}).", data.DevInst);
            return Ok(true);
        }
    }

    Ok(false)
}

/// Removes every network devnode whose HardwareID contains `*MSLOOP`, i.e.
/// the Microsoft KM-TEST Loopback adapter created from `netloop.inf`.
#[cfg(target_os = "windows")]
fn remove_loopback() -> Result<(), DriverError> {
    use windows::core::{Error as WinError, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
        SetupDiGetDeviceRegistryPropertyW, SetupDiRemoveDevice, GUID_DEVCLASS_NET,
        SETUP_DI_GET_CLASS_DEVS_FLAGS, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
    };

    // SAFETY: `PCWSTR::null()` is a valid null enumerator; the GUID is static.
    let raw = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_NET),
            PCWSTR::null(),
            None,
            SETUP_DI_GET_CLASS_DEVS_FLAGS(0),
        )
    }
    .map_err(|error| setup_error("SetupDiGetClassDevsW failed", error))?;
    let set = DeviceInfoSet(raw);

    let mut targets = Vec::new();
    let mut index = 0_u32;
    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };

        // SAFETY: `data` is correctly initialized and `index` is an ordinal.
        if unsafe { SetupDiEnumDeviceInfo(set.as_raw(), index, &mut data) }.is_err() {
            break;
        }
        index += 1;

        let mut buffer = [0_u16; 128];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 2)
        };

        // SAFETY: `bytes` points to `buffer`'s storage and matches its length.
        if unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set.as_raw(),
                &data,
                SPDRP_HARDWAREID,
                None,
                Some(bytes),
                None,
            )
        }
        .is_ok()
            && String::from_utf16_lossy(&buffer)
                .to_ascii_uppercase()
                .contains("*MSLOOP")
        {
            targets.push(data);
        }
    }

    if targets.is_empty() {
        tracing::info!("Loopback-адаптер не найден, удалять нечего.");
        return Ok(());
    }

    for mut data in targets {
        let mut id_buffer = [0_u16; 256];
        let mut required = 0_u32;
        let instance = if unsafe {
            SetupDiGetDeviceInstanceIdW(
                set.as_raw(),
                &data,
                Some(&mut id_buffer),
                Some(&mut required),
            )
        }
        .is_ok()
        {
            let len = id_buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(id_buffer.len());
            String::from_utf16_lossy(&id_buffer[..len])
        } else {
            format!("DevInst {}", data.DevInst)
        };

        // SAFETY: `data` describes a device in `set`; SetupDiRemoveDevice
        // invalidates it on success, which is why it is passed by `&mut`.
        let removed = unsafe { SetupDiRemoveDevice(set.as_raw(), &mut data) };
        if !removed.as_bool() {
            return Err(setup_error(
                &format!("SetupDiRemoveDevice failed for {instance}"),
                WinError::from_thread(),
            ));
        }

        tracing::info!("[OK] Loopback adapter removed successfully ({instance}).");
    }

    Ok(())
}

/// Picks the next free `ROOT\NET\<n>` instance number, mirroring the original
/// installer (which created `ROOT\NET\0001` on a clean system).
#[cfg(target_os = "windows")]
fn next_root_net_instance() -> Result<u32, DriverError> {
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
        GUID_DEVCLASS_NET, SETUP_DI_GET_CLASS_DEVS_FLAGS, SP_DEVINFO_DATA,
    };

    let raw = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_NET),
            PCWSTR::null(),
            None,
            SETUP_DI_GET_CLASS_DEVS_FLAGS(0),
        )
    }
    .map_err(|error| setup_error("SetupDiGetClassDevsW failed", error))?;
    let set = DeviceInfoSet(raw);

    let mut used = std::collections::HashSet::new();
    let mut index = 0_u32;
    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: `data` is initialized correctly; `index` is an ordinal.
        if unsafe { SetupDiEnumDeviceInfo(set.as_raw(), index, &mut data) }.is_err() {
            break;
        }
        index += 1;

        let mut id_buffer = [0_u16; 256];
        let mut required = 0_u32;
        // SAFETY: `id_buffer` is large enough for any device instance ID.
        if unsafe {
            SetupDiGetDeviceInstanceIdW(
                set.as_raw(),
                &data,
                Some(&mut id_buffer),
                Some(&mut required),
            )
        }
        .is_ok()
        {
            let len = id_buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(id_buffer.len());
            let id = String::from_utf16_lossy(&id_buffer[..len]).to_ascii_uppercase();
            if let Some(suffix) = id.strip_prefix("ROOT\\NET\\") {
                if let Ok(number) = suffix.parse::<u32>() {
                    used.insert(number);
                }
            }
        }
    }

    let mut candidate = 0;
    while used.contains(&candidate) {
        candidate += 1;
    }
    Ok(candidate)
}

/// Creates a root-enumerated `ROOT\NET\<n>` devnode with the `*MSLOOP`
/// hardware ID and registers it with the class installer.
#[cfg(target_os = "windows")]
fn create_msloop_devnode() -> Result<(), DriverError> {
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW,
        SetupDiSetDeviceRegistryPropertyW, DIF_REGISTERDEVICE, GUID_DEVCLASS_NET,
        SETUP_DI_DEVICE_CREATION_FLAGS, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
    };

    // SAFETY: static class GUID, no parent window.
    let raw = unsafe { SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_NET), None) }
        .map_err(|error| setup_error("SetupDiCreateDeviceInfoList failed", error))?;
    let set = DeviceInfoSet(raw);

    // `DeviceName` must be a full device instance ID when DICD_GENERATE_ID is
    // not used; "ROOT\NET" alone is rejected with ERROR_INVALID_DEVINST_NAME.
    let instance = next_root_net_instance()?;
    let instance_id = format!("ROOT\\NET\\{instance:04}");
    let instance_id: Vec<u16> = instance_id
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut data = SP_DEVINFO_DATA {
        cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };

    tracing::info!(
        "Создание devnode {} с HardwareID *MSLOOP ...",
        String::from_utf16_lossy(&instance_id[..instance_id.len() - 1])
    );

    // SAFETY: `instance_id` is NUL-terminated and outlives the call; `data` is
    // initialized with the required cbSize.
    unsafe {
        SetupDiCreateDeviceInfoW(
            set.as_raw(),
            PCWSTR::from_raw(instance_id.as_ptr()),
            &GUID_DEVCLASS_NET,
            PCWSTR::null(),
            None,
            SETUP_DI_DEVICE_CREATION_FLAGS(0),
            Some(&mut data),
        )
    }
    .map_err(|error| setup_error("SetupDiCreateDeviceInfoW failed", error))?;

    // REG_MULTI_SZ requires two trailing NULs.
    let hardware_id: Vec<u16> = "*MSLOOP"
        .encode_utf16()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(hardware_id.as_ptr().cast::<u8>(), hardware_id.len() * 2)
    };

    // SAFETY: `bytes` is a UTF-16 REG_MULTI_SZ buffer owned by `hardware_id`.
    unsafe {
        SetupDiSetDeviceRegistryPropertyW(set.as_raw(), &mut data, SPDRP_HARDWAREID, Some(bytes))
    }
    .map_err(|error| setup_error("SetupDiSetDeviceRegistryPropertyW failed", error))?;

    // SAFETY: `data` describes a device in `set`.
    unsafe { SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.as_raw(), Some(&data)) }.map_err(
        |error| {
            setup_error(
                "SetupDiCallClassInstaller(DIF_REGISTERDEVICE) failed",
                error,
            )
        },
    )?;

    tracing::info!("[OK] Devnode ROOT\\NET создан (DevInst {}).", data.DevInst);
    Ok(())
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
