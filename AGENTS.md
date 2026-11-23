# doip-remote — Agent Context

Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

This file is the persistent project memory for AI agents. Read it before making
changes. Update the **Current Status** block at the end of every working day,
and record important architecture decisions as they are made.

## Current Status (обновляй этот блок каждый день)

- **Date:** 2026-11-23
- **Phase:** Windows 11 validation; Loopback install **and removal** fixed and
  verified on the target machine.
- **Done:** Stage 6/6 — `--mock` mode and final verification. The client is now a
  ratatui dashboard with a menu, dot-aligned status panel, scrolling log console,
  background driver installation, UAC gate and a cleanup modal. `inquire` is gone.
  Follow-up: the key-hint row is now readable (bold yellow keys, cyan actions),
  and `--mock` no longer fakes the connection: the status stays `Disconnected`
  (`[ОТКЛЮЧЕНО]`) until the relay server exists.
  UI polish: white bold header, strictly red `[НЕТ]`/`[MISSING]` statuses, a
  `Log File` status row, a bold red `[!]` admin warning marker, and a green
  session-code popup after the Car action.
  Lifecycle: the menu now has separate "Удалить драйверы и логи" and
  "Завершить диагностику" entries; cleanup swaps the file writer for a sink,
  deletes `doip-remote.log` without Windows lock errors and keeps the app running
  with refreshed statuses.
  Shutdown: `net::NetworkService` owns a cancellation token and a join set; every
  exit path cancels it and waits for tasks to drop their sockets.
  Drivers & extras: Npcap is uninstalled via `NPFInstall.exe -n -u` (with a
  stale-file fallback); the Loopback
  adapter is created by first registering a root-enumerated `ROOT\NET\<n>`
  devnode with HardwareID `*MSLOOP` (SetupAPI) and then calling
  `newdev!UpdateDriverForPlugAndPlayDevicesW` with `%SystemRoot%\INF\netloop.inf`
  and removed with `SetupDiRemoveDevice`; the Car code is copied to the clipboard
  on `Enter`; the header shows the
  package version (`v0.1.0`).
  **Windows 11 fix (2026-11-23):** the previous code called
  `UpdateDriverForPlugAndPlayDevicesW` without creating a device, so setupapi
  reported `Unable to find any matching devices` / `0xE000020B`. The fix creates
  the devnode first (next free `ROOT\NET` instance), reuses an existing
  `*MSLOOP` node, and logs SetupAPI HRESULTs in the UI. Runtime-verified on
  Windows 11: `ROOT\NET\0001` = Microsoft KM-TEST Loopback Adapter, visible in
  `Get-NetAdapter`, UI status `Loopback [OK]`, setupapi section ends with
  `Exit status: SUCCESS`.
  Icon: `crates/doip-client/build.rs` converts `icon.png` into a multi-size,
  PNG-compressed `.ico` (16/32/48/64/128/256) and links it through
  `embed-resource` for Windows targets, so Explorer and the taskbar show it.
- **Next Step:** Finish the remaining items of the Windows 11 checklist below
  (icon rendering, non-admin warning, localized names); the Loopback install and
  cleanup paths are now green. Server/relay work (`doip-server`) remains
  postponed.
- **Repo state:** branch `main`; stages 1-5 are committed as
  `wip: stage N completed`, stage 6 as the final dashboard commit, followed by
  `fix(ui): improve hotkeys readability with yellow/cyan colors`,
  `fix(state): force connection status to Disconnected until server is implemented`
  and the Windows-polish checkpoints, then
  `fix(drivers): resolve Loopback installation failure (0xE000020B) via api` and
  `fix(drivers): actually remove Loopback adapter and refresh UI status` and
  `style(ui): change header color to white`.

## Architecture

- **Current focus:** `doip-client` interface and Windows driver management.
  The relay/server (`doip-server`) is postponed until the Windows client driver
  flow is validated.
- **Topology:** `Car (Windows) <-> Relay Server (Linux) <-> Diag (Windows)`.
- **Data path (planned):** the car-side agent tunnels DoIP traffic to the relay;
  the diagnostic client joins the same session using a `DT1-...` session code.
- **L2/L4:** both are planned. **L4 (proxy) comes first**, then **L2**
  (Npcap + KM-TEST Loopback) on Windows. The client now checks and installs the
  L2 driver prerequisites; packet capture itself is not implemented yet.
- **Crypto (planned):** X25519 key agreement + ChaCha20-Poly1305 AEAD. Not
  implemented yet; `token` and `key` in session codes are currently random
  placeholders, not negotiated secrets.
- **UI:** ratatui dashboard (crossterm backend); no WebView2, no `inquire`.
- **Shutdown:** a `tokio_util::sync::CancellationToken`, owned by
  `net::NetworkService` together with a `JoinSet`, stops every network task.
  Finish diagnostics, `Esc`, `q` and `Ctrl+C` cancel the token; shutdown waits up
  to 3 s, aborts stragglers, drops listeners (releasing their ports) and closes
  accepted streams with `linger(0)` so nothing lingers in `TIME_WAIT`.
- **Roles:** `Car` (agent next to the vehicle) and `Diag` (remote technician).

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # SessionCode, Role, errors, session-code tests
│   └── doip-client/   # Interactive ratatui client (binary)
│       ├── build.rs   # PNG -> multi-size ICO + Windows resource linking
│       ├── icon.png   # Source application icon (1254×1254 PNG)
│       └── src/
│           ├── main.rs        # Event loop, menu actions, install events
│           ├── app.rs         # App state, menu items, focus, scroll
│           ├── ui.rs          # ratatui layout, status rows, modals
│           ├── tui.rs         # Terminal init/restore, panic hook, key reader
│           ├── logging.rs     # File + in-memory log layers
│           ├── net.rs         # Cancellation token, network tasks, shutdown
│           └── drivers/mod.rs # Npcap/Loopback detection and installation
├── .github/workflows/ # CI: build + test on Ubuntu and Windows
├── Cargo.toml         # Cargo workspace
├── Cargo.lock         # Committed for reproducible application builds
├── rust-toolchain.toml
├── AGENTS.md          # This file
└── README.md
```

Planned: `crates/doip-server` (relay) — **postponed** until the Windows client
driver flow is validated.

## Session code format (implemented)

- Prefix: `DT1-`.
- Payload: URL-safe Base64 **without padding** of compact JSON.
- Short JSON keys:

| Key | Field        | Notes                                      |
| --- | ------------ | ------------------------------------------ |
| `r` | `role`       | `car` or `diag`                            |
| `s` | `server`     | Relay address, e.g. `diag.example.com:13500` |
| `n` | `session_id` | Unique session identifier                  |
| `t` | `token`      | Session token (placeholder for now)        |
| `k` | `key`        | Session key (placeholder for now)          |
| `l` | `bridge`     | Bridge mode flag                           |
| `p` | `doip_port`  | DoIP port, default `13400`                 |
| `v` | `vehicle_ip` | Optional; omitted when unknown             |

- `SessionCode::new(role, server, vehicle_ip)` generates UUID-based identifiers,
  `bridge = true`, `doip_port = 13400`.
- `generate()` is infallible; `parse()` returns `doip_core::Error`.
- Parser ignores whitespace anywhere, so codes pasted from wrapped terminal
  output work. Missing `l`/`p`/`v` use defaults. Required fields must be
  non-empty; port `0` is rejected.
- The client uses `server = "auto"` as a placeholder until the relay address is
  agreed.

## Current TUI behavior

- **Layout:** header (`=== DoIP Remote ===` plus key hints), middle row split
  70/30 into the menu and the dot-aligned status panel, bottom `[ ЛОГИ ]`
  console. Modals are drawn on top. The header title is **white and bold**;
  the package version (`v0.1.0`) is shown in the top-right corner.
- **Key hints:** built from the `HOTKEYS` table in `ui.rs` and rendered as
  `[ключ] - действие • ...`; key caps are bold yellow, separators and action
  descriptions are cyan so the row stays readable on a dark terminal.
- **Startup driver check** (before the first frame):
  - Windows: Npcap is detected via
    `%SystemRoot%\System32\Npcap\wpcap.dll`; Loopback is detected by searching
    adapter name/description for `Loopback`/`KM-TEST` via `Get-NetAdapter`
    (`netsh` fallback). Admin rights are probed with `net session`.
  - Linux/non-Windows: stub returning `false`/`false`; admin is `false`.
- **Status panel:** `Права Админа`, `Npcap` and `Loopback` (green `[OK]` / red
  `[НЕТ]` — a missing value is **never** green), `Log File` (green `[EXISTS]` /
  red `[MISSING]`, based on `doip-remote.log`), and `Соединение` (red
  `[ОТКЛЮЧЕНО]` / green `[ЕСТЬ КОНТАКТ ✓]`). The connection is always
  `Disconnected`: the `Connected` variant is reserved for the future relay and is
  never constructed, including in `--mock` mode.
- **Menu:** Install / Car / Diag / Delete drivers and logs (red) / Finish
  diagnostics.
- **Navigation:** `↑`/`↓` move (or scroll logs), `Enter` activates, `Tab`
  toggles menu/log focus, `PageUp`/`PageDown` scroll the log panel, `Ctrl+C`
  quits. `Esc` closes a popup; in the menu it quits.
- **Logs:** written to `doip-remote.log` next to the executable and mirrored into
  the panel by a custom `tracing` layer (ring buffer, 1000 lines). Levels are
  colour-coded; admin warnings start with a bold red `[!]` marker. The file layer
  is reloadable: cleanup disables it while the in-memory mirror keeps working.
- **Install (Windows):** a background task streams the Npcap 1.80 download with
  percentage progress in the log, runs the installer **without** `/S`, then
  installs the KM-TEST Loopback adapter. That step first ensures a devnode with
  HardwareID `*MSLOOP` exists: an existing one is reused, otherwise the next
  free `ROOT\NET\<n>` instance is created with
  `SetupDiCreateDeviceInfoW` + `SetupDiSetDeviceRegistryPropertyW(SPDRP_HARDWAREID)`
  + `SetupDiCallClassInstaller(DIF_REGISTERDEVICE)`. Only then
  `newdev.dll!UpdateDriverForPlugAndPlayDevicesW` binds the dynamically resolved
  `%SystemRoot%\INF\netloop.inf` (flags
  `INSTALLFLAG_FORCE | INSTALLFLAG_NONINTERACTIVE`); the reboot flag and any
  SetupAPI HRESULT are logged. The menu is locked meanwhile and statuses refresh
  afterwards. Without admin rights the action only logs an instruction to
  restart as Administrator.
- **Car:** generates a `DT1-...` code (server `"auto"`), logs it and opens a
  popup where the code is rendered in green. `Enter` copies it to the system
  clipboard and logs `[INFO] Code copied to clipboard!`; `Esc` closes the popup.
- **Diag:** opens a popup; typed/pasted code is parsed on `Enter`, and the
  target IP, DoIP port and relay server are logged.
- **Delete drivers and logs:** opens a `Y/N` confirmation popup. `Y` runs the
  Npcap uninstaller (`C:\Program Files\Npcap\NPFInstall.exe -n -u`, waited for),
  removes the KM-TEST Loopback devnodes (`*MSLOOP`) via `SetupDiRemoveDevice`,
  then swaps the file log writer for a sink, drops the old appender guard
  (closing the file handle) and deletes `doip-remote.log`; the app **keeps
  running**. Npcap and Loopback removals are independent: a failed Npcap
  uninstall no longer skips the Loopback adapter. When the `npcap` service is
  gone but `%SystemRoot%\System32\Npcap` remains (NPFInstall can report success
  yet leave `wpcap.dll`), the stale folder is deleted. The driver statuses are
  re-probed immediately and again after log deletion, so the panel shows
  `Npcap [НЕТ]` / `Loopback [НЕТ]`; the refreshed values are logged before the
  file writer is disabled.
- **Finish diagnostics:** exits the application and cancels the network token;
  active tasks stop and their sockets are closed before the process ends.
- **`--mock`:** forces admin rights and both driver statuses green so the
  dashboard can be previewed on Linux. The network status is deliberately not
  faked and stays red `[ОТКЛЮЧЕНО]` until the server exists.
- Networking is a **stub**; driver detection, Npcap installation/uninstallation
  and Loopback creation are real only on Windows. Loopback creation was
  runtime-tested on Windows 11; the other Windows paths were compile-checked.

## Commands

```sh
cd doip-remote
source "$HOME/.cargo/env"   # if cargo is not on PATH
cargo build --workspace
cargo test --workspace
cargo run -p doip-client
cargo run -p doip-client -- --mock   # preview with admin/drivers emulated
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Windows cfg compile check from Linux (no linker needed)
rustup target add x86_64-pc-windows-msvc
cargo check -p doip-client --target x86_64-pc-windows-msvc
cargo clippy -p doip-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## Last verification (2026-11-23)

- `cargo build --release` — OK.
- `cargo test --workspace` — 27 `doip-client` + 17 `doip-core` tests + 1 doctest,
  all passing.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- Windows target code: `cargo check`/`clippy` for
  `x86_64-pc-windows-msvc` — clean (compile only, not runtime-tested), including
  `newdev!UpdateDriverForPlugAndPlayDevicesW` and `arboard`.
- Icon build: `build.rs` produced a valid 151552-byte ICO with six
  PNG-compressed entries (16/32/48/64/128/256) in `OUT_DIR`; the Windows check
  only warns about the missing `llvm-rc` on Linux, as designed.
- Linux PTY runs — OK: non-admin dashboard with red statuses and bold red `[!]`,
  `Log File [EXISTS]`, white bold header with `v0.1.0`, green Car-code popup,
  clipboard copy on `Enter` (`Code copied to clipboard!`), separate cleanup/exit
  entries, cleanup deletes the log while the app stays alive, graceful Ctrl+C
  shutdown (`Сетевые задачи остановлены, сокеты закрыты`).
- Windows 11 (elevated) runtime run of the rebuilt `doip-client.exe`: pressed
  Enter on "Install Drivers"; log showed devnode creation, INF binding and
  `Loopback [OK]`. `Get-NetAdapter` listed `Ethernet 3` /
  `Microsoft KM-TEST Loopback Adapter`, PnP showed `ROOT\NET\0001`, and
  `setupapi.dev.log` ended the section with `Exit status: SUCCESS`.
- Windows 11 (elevated) cleanup run: pressed "Удалить драйверы и логи" and
  confirmed `Y`. Log showed
  `[OK] Loopback adapter removed successfully (ROOT\NET\0001).`; `Get-NetAdapter`
  and `Get-PnpDevice` no longer listed the adapter, and the stale Npcap files
  were removed. A second install→cleanup cycle reproduced the result with
  statuses re-probed to `[НЕТ]`/`[НЕТ]`. Screenshot:
  `%TEMP%\opencode\doip_cleanup_screen.png`.
- The embedded icon on real Windows 11 is **not yet visually confirmed**.

## Environment notes

- Rust was missing on this machine and was installed via rustup:
  **stable 1.100.0**, profile `minimal`, in `~/.cargo`.
- PATH may not include `~/.cargo/bin`; run `source "$HOME/.cargo/env"` first.
- `rustfmt` and `clippy` were added with `rustup component add rustfmt clippy`.
- The `x86_64-pc-windows-msvc` target is installed for cross-compile checks;
  there is no Windows runtime on this Linux machine.
- Workspace `rust-version` is `1.89`; CI uses the stable channel.
- `reqwest` is a Windows-only target dependency; Linux builds do not compile it.
  Driver installation requires administrator rights on Windows.
- Windows-only crates: `windows 0.62` (newdev API) and `arboard 3.7` (clipboard,
  also cross-platform). `tokio-util 0.7` provides `CancellationToken`.
- Build dependencies (`crates/doip-client`): `embed-resource 3.0` and
  `image 0.25` (PNG only). `build.rs` generates the `.ico` itself, so no manual
  conversion is needed.
- No `rc`/`windres` on this Linux host, so cross-compiling for Windows emits
  `embed-resource: compilation not attempted: missing compiler: llvm-rc` as a
  warning and skips the icon. On a real Windows host (or with `llvm-rc` /
  `windres` installed) the icon is compiled and linked automatically; a resource
  compilation failure there is a hard error.
- TUI stack: `ratatui 0.26` + `crossterm 0.27`; logs use `tracing-appender 0.2`.
  `inquire` was removed in stage 1.

## Conventions

- Edition 2021; stable Rust; `Cargo.lock` is committed.
- `doip-core` has `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`; keep
  public APIs documented. Isolate any unavoidable `unsafe` in the L2/driver
  layer later, never in the protocol core.
- UI strings are Russian; code, comments, README and commit messages are English.
- Add unit tests for every parsing/state-machine change.
- Do not commit `/target`, `.env`, credentials, tokens or generated keys.
- Before committing: fmt, clippy with `-D warnings`, workspace tests.

## Next Step — Windows 11 validation

Server/relay work is **postponed**. The next milestone is validating the client
and its driver flow on real Windows 11:

- [x] `cargo build --release` and `cargo test --workspace` on Windows 11.
- [ ] Confirm the built `.exe` shows the custom icon in Explorer and on the
      taskbar (the resource is embedded by `build.rs` on Windows).
- [ ] Confirm the dashboard renders correctly in Windows Terminal (colours,
      borders, Cyrillic, `✓`).
- [ ] Run as a normal user without Npcap/Loopback: confirm the red admin status,
      red driver statuses and the admin warning on Install.
- [x] Install drivers from the menu as administrator (Npcap was already
      installed, so only the Loopback path ran; UAC elevation was approved).
- [x] Confirm the SetupAPI result and devnode creation appear in the log panel
      and in `doip-remote.log`.
- [x] Confirm Npcap appears at `%SystemRoot%\System32\Npcap\wpcap.dll`.
- [x] Confirm the KM-TEST adapter appears and is found by `Get-NetAdapter`
      (`ROOT\NET\0001`, `Ethernet 3`).
- [x] Confirm the status turns green after installation without a restart
      (`Loopback [OK]`).
- [ ] Confirm the Car popup copies the code on `Enter` (clipboard contains a
      `DT1-...` string and the log says `Code copied to clipboard!`).
- [ ] Confirm `v0.1.0` appears in the header's top-right corner.
- [x] Confirm "Delete drivers and logs" runs `NPFInstall.exe -n -u`, removes the
      KM-TEST adapter, clears both statuses and deletes the log without a
      sharing violation (`ROOT\NET\0001` and `wpcap.dll` both gone afterwards).
- [ ] Check detection on a localized Windows install (`Get-NetAdapter`/`netsh`
      output may be localized; adapter names/markers should still match).
- [ ] Test failure paths: declined UAC, canceled Npcap wizard, missing
      `netloop.inf`, download error, cleanup `Y/N` and log deletion.
- [ ] Confirm Ctrl+C and window close restore the console (no broken prompt).
- [ ] Only after this is green, resume `doip-server` work.

## Postponed: relay/server design questions

Decide these when `doip-server` is resumed:

- [ ] Wire framing after TCP accept: length-prefixed frames? JSON or binary?
- [ ] Handshake/authentication with `session_id` + `token`; replay protection,
      expiry and one-Car/one-Diag session slots.
- [ ] Relay registration and pairing lifecycle, timeouts, reconnect behavior.
- [ ] How the relay address (`server` field) is discovered/configured; replace
      the `"auto"` placeholder.
- [ ] Where X25519 + ChaCha20-Poly1305 fits: end-to-end or relay-terminated?
- [ ] L4 proxy semantics, port mapping and DoIP payload transparency.
- [ ] L2 data path later: Npcap capture + KM-TEST Loopback integration.

## How to resume

1. Read this **Current Status** block and the Windows 11 checklist above.
2. `git log --oneline -5` and `git status` in `doip-remote`.
3. Run `cargo test --workspace` to confirm the baseline.
4. Continue with **Next Step** (Windows 11 validation); server work is postponed.
   Update this file, then commit.
