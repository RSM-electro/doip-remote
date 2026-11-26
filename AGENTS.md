# doip-remote — Agent Context

Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

This file is the persistent project memory for AI agents. Read it before making
changes. Update the **Current Status** block at the end of every working day,
and record important architecture decisions as they are made.

## Current Status (обновляй этот блок каждый день)

- **Date:** 2026-11-26
- **Phase:** Plaintext client ↔ relay integration complete. The Car/Diag actions
  now pair through `doip-server` and tunnel raw bytes. TLS/encryption is the next
  backend milestone.
- **Done:** Stage 6/6 — `--mock` mode and final verification. The client is now a
  ratatui dashboard with a menu, dot-aligned status panel, scrolling log console,
  background driver installation, UAC gate and a cleanup modal. `inquire` is gone.
  Follow-up: the key-hint row is now readable (bold yellow keys, cyan actions),
  and `--mock` no longer fakes the connection: the status stays `Disconnected`
  until a real relay session starts.
  UI polish: white bold header, strictly red `[MISSING]`/`[DISCONNECTED]`
  statuses, a `Log File` status row, a bold red `[!]` admin warning marker, and
  a green session-code popup after the Car action.
  Lifecycle: the menu has "4. Uninstall drivers and logs" and "5. Exit"; cleanup
  swaps the file writer for a sink, deletes `doip-remote.log` without Windows
  lock errors and keeps the app running with refreshed statuses.
  **Log location (2026-11-24):** `doip-remote.log` is always created on the
  user's Desktop (`USERPROFILE` on Windows, `HOME` on Linux), falling back to the
  home directory when `Desktop` does not exist. The exact resolved path is logged
  at startup and cleanup deletes that same resolved path.
  **Stage 1 (2026-11-24):** all UI strings translated to English; numeric
  hotkeys `1-5` select and trigger menu items immediately; menu shows numbered
  labels; top hint bar includes `[1-5] - select`.
  **Stage 2 (2026-11-24):** Windows release builds embed a UAC manifest requiring
  admin (`requireAdministrator`, `uiAccess="false"`) via `embed-resource` with
  `.manifest_required()` on Windows hosts (debug omits manifest so tests can run
  without elevation); cleanup confirmation modal uses `Enter` to confirm and
  `Esc` to cancel (no `Y/N`), and `Esc` in modals only closes the modal.
  **Stage 3 (2026-11-24):** cleanup cancels the network `CancellationToken`
  (disconnecting sockets), removes drivers/log, switches logging to a no-op file
  sink, refreshes Npcap/Loopback/Log File to `[MISSING]` and Connection to
  `[DISCONNECTED]`, sets `is_cleaned_up`, disables menu items 1-4 (DarkGray,
  inputs ignored), and leaves only `5. Exit` active.
  **Task 1 (2026-11-24):** exiting while `ConnectionStatus::Connected` shows a
  confirmation modal `[!] Active session. Disconnect and exit? (Enter/Esc)`;
  `Enter` disconnects and exits, `Esc` cancels. Exiting when disconnected/cleaned
  up is immediate.
  **Task 2 (2026-11-24):** cleanup is panic-safe (`catch_unwind`) and never exits;
  sequence: cancel `CancellationToken`, remove Npcap/Loopback, swap file logger to
  sink/drop guard/delete log, force driver/log status refresh, set `is_cleaned_up`
  and lock menu 1-4 (DarkGray, inputs ignored), only `5. Exit` active.
  **Cleanup-crash root cause (2026-11-24, reproduced on Windows 11):** `panic.log`
  hook was added, but repro showed **no panic**. Sending `4` + `Enter` kept the app
  alive; sending `4` + `Enter` + `Enter` made it exit with code 0. Cause: cleanup
  moved `menu_index` to `5. Exit`, so a repeated/auto-repeated Enter immediately
  activated Exit. Fix: leave the selection on the disabled cleanup item after
  cleanup; repeated Enter is ignored, only numeric `5` (or explicit navigation to
  Exit) quits.
  **Hidden-console root cause (2026-11-24, the real "app closes" symptom):** the
  process was often still alive while the window disappeared. The leftover
  elevated process had a hidden console titled
  `Administrator: NPFInstall for Npcap 1.80`, proving that `NPFInstall.exe`
  hijacked the shared console, changed its title and hid it. All driver helper
  commands now run with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` and null
  stdio, so they cannot touch the TUI console. After cleanup the app calls
  `ShowWindow(SW_RESTORE)`, restores the title `DoIP Remote`, re-enables raw mode,
  re-enters the alternate screen and redraws. Post-cleanup `Esc`/`q` are ignored;
  only `5. Exit` or `Ctrl+C` leaves. `cleanup-trace.log` (next to the exe) records
  cleanup steps, console restoration and explicit exit reasons. Scripted release
  run: `4` + `Enter`, repeated `Enter` and `Esc` keep the process and console
  visible; `5` + `Enter` exits with code 0; no `panic.log`. The user then manually
  re-tested the elevated build and confirmed the cleanup screen stays open.
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
  Icon/manifest: `crates/doip-client/build.rs` converts `icon.png` into a
  multi-size, PNG-compressed `.ico` (16/32/48/64/128/256), writes a Windows
  manifest requiring administrator privileges, and links both through
  `embed-resource` for Windows targets; the UAC manifest is embedded in release
  builds (`.manifest_required()` on Windows hosts) and omitted in debug builds so
  test binaries can run without elevation.
  **Server checkpoint 1 (2026-11-25):** added `crates/doip-protocol` with the
  versioned relay handshake (`Hello` → `Waiting`/`Paired`/`Error`), `u32`-BE
  length-prefixed JSON control frames capped at 16 KiB, field validation and
  async read/write helpers. `Role` now serializes as `car`/`diag`. 9 protocol
  tests + 17 core tests pass.
  **Server checkpoint 2 (2026-11-25):** added `crates/doip-server` (library +
  binary) with a Tokio accept loop, handshake timeout, session registry,
  token/role checks, pending TTL and pending-session cap, `Waiting`/`Paired`/
  `Error` responses, raw `copy_bidirectional` tunnels, cancellable graceful
  shutdown and CLI flags. 12 integration tests over real loopback TCP cover
  forwarding, 1 MiB payloads, session isolation, wrong token, duplicate role,
  malformed/oversized frames, handshake and pending timeouts, disconnect
  propagation, shutdown and the pending-session cap.
  **Server checkpoint 3 (2026-11-25):** deployed `doip-server` to
  `176.53.237.193`. Installed Debian `build-essential` + rustup 1.100, built the
  release binary, installed `/usr/local/bin/doip-server`, created the system user
  `doip`, and enabled `doip-server.service` (plaintext `0.0.0.0:13500`,
  `Restart=on-failure`, systemd hardening). A public smoke test from the
  workstation paired a session, forwarded 27 bytes both ways and rejected a
  wrong token with `token_mismatch`; the service is `active` and `enabled`.
  Deployment artifacts: `deploy/doip-server.service`, `deploy/README.md`.
  **Client checkpoint 1 (2026-11-25):** integrated the relay into `doip-client`.
  `net::NetworkService` now owns real Car/Diag tasks: both connect to the relay
  with the v1 handshake, wait for `Waiting` → `Paired`, and then bridge raw bytes
  with `copy_bidirectional`. Car dials `vehicle_ip:doip_port` (default
  `127.0.0.1:13400`, `--vehicle`/`DOIP_VEHICLE`); Diag binds a local listener
  (default `127.0.0.1:13400`, `--diag-listen`/`DOIP_DIAG_LISTEN`) before pairing
  and serves the first local diagnostic connection. Relay defaults to
  `176.53.237.193:13500` (`--relay`/`DOIP_RELAY`); `--help`/`--version` were
  added. `ConnectionStatus` gained `Connecting` (`[CONNECTING]`, yellow) and the
  exit confirmation now triggers on any active session, not only `Connected`.
  Network tasks report status/end events to the UI loop; cleanup and exit cancel
  them. 42 client tests pass, including a loopback end-to-end test (real
  `doip-server` + echo vehicle + Car/Diag tasks). The real client binaries were
  also driven through PTYs against both a local relay and the public
  `176.53.237.193:13500`; a 30-byte payload echoed both ways.
  **Server verification (2026-11-26):** the existing rendezvous relay was
  re-verified without changes. `cargo build -p doip-server` is clean and the
  targeted `pairs_and_forwards_bytes_both_ways` integration test passes. A
  standalone two-client run against `127.0.0.1:15600` with the same session
  id/token received `waiting` → `paired` on both sockets and exchanged 9 bytes
  each way; the log showed `paired session` and
  `tunnel closed ... car_to_diag=9 diag_to_car=9`. The production protocol stays
  the shared length-prefixed JSON handshake (`doip-protocol`); it was **not**
  downgraded to first-line `DT1-` + `Arc<Mutex<HashMap>>`.
  **VPS deployment (2026-11-26):** rebuilt `doip-server` natively on the Linux
  workstation (`cargo build --release -p doip-server`), copied it with `scp` to
  `/root/doip-server`, installed it atomically to `/usr/local/bin/doip-server`
  (`install -m 0755 … .new` then `mv`), and restarted the existing
  `doip-server.service`. The binary requires at most `GLIBC_2.34` (checked with
  `objdump -T`); the VPS runs Debian 13.7 with glibc `2.41`, so it is compatible.
  Local and remote SHA-256 match:
  `a4a19b181ba41212840f39fc9d06461d77f839b06e2333d9e8771795b37c304b`.
  The service is `active`/`enabled` (PID 18094) and listens on `0.0.0.0:13500`;
  `ufw` still allows only `22/tcp` and `13500/tcp` (IPv4 + IPv6). A public
  pairing test from the workstation (`deploy-verify-1795684067`) received
  `waiting` → `paired` on both sockets and exchanged 10 bytes (`car→diag`) and
  11 bytes (`diag→car`); the journal logged `paired session` and
  `tunnel closed … car_to_diag=10 diag_to_car=11 elapsed_ms=15`.
  The `doip-relay` SSH alias is **not** configured on this workstation, so the
  commands use `root@176.53.237.193` directly. A second `nohup` instance was
  deliberately avoided because it would collide with the systemd unit on port
  `13500`; the binary accepts `--listen`, not `--port`.
- **Next Step:** Backend encryption (TLS/Noise) still deferred by design. Before
  that, consider client hardening: allow a new Diag connection after the first
  one closes (currently one tunnel serves one local DoIP connection), auto-detect
  the vehicle IP/loopback adapter, and surface session identifiers in the UI.
  Then TLS at the relay, followed by X25519 + ChaCha20-Poly1305 as decided.
  The remaining Windows 11 checklist items are non-blocking.
- **Repo state:** branch `main`; recent commits include Stage 1-3 UX hardening,
  `feat(ui): add confirmation modal for active session exit`,
  `fix(lifecycle): definitively resolve cleanup crash and implement strict menu blocking`,
  the `panic.log` diagnostics commit, the strict post-cleanup lifecycle fix
  (console isolation, quit fence, hidden-window restoration), the plaintext
  relay protocol, server and deployment checkpoints, client relay integration,
  and the native VPS binary redeploy.

## Architecture

- **Current focus:** plaintext L4 tunnel works end-to-end. The next backend
  milestone is transport security (TLS at the relay) and then end-to-end
  encryption; client UX hardening continues alongside.
- **Topology:** `Car (Windows) <-> Relay Server (Linux) <-> Diag (Windows)`.
- **Data path (implemented):** the Car connects to the relay, waits for the
  Diag, then dials the vehicle and forwards raw bytes. The Diag connects to the
  same session using a `DT1-...` code, listens on `127.0.0.1:13400` and forwards
  the first local diagnostic connection. One relay session serves one DoIP TCP
  connection.
- **L2/L4:** **L4 (proxy) is implemented**. **L2** (Npcap + KM-TEST Loopback) on
  Windows is still planned; the client checks and installs the prerequisites but
  does not capture packets yet.
- **Crypto (planned):** X25519 key agreement + ChaCha20-Poly1305 AEAD. Not
  implemented yet; `token` and `key` in session codes are currently random
  placeholders, not negotiated secrets.
- **UI:** ratatui dashboard (crossterm backend); no WebView2, no `inquire`.
- **Shutdown:** a `tokio_util::sync::CancellationToken`, owned by
  `net::NetworkService` together with a `JoinSet`, stops every network task.
  Finish diagnostics, `Esc`, `q` and `Ctrl+C` cancel the token; shutdown waits up
  to 3 s, aborts stragglers, drops listeners (releasing their ports) and shuts
  down accepted streams.
- **Roles:** `Car` (agent next to the vehicle) and `Diag` (remote technician).

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # SessionCode, Role, errors, session-code tests
│   ├── doip-protocol/ # Relay handshake messages, validation and framing
│   ├── doip-server/   # Plaintext relay (binary + testable library)
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
├── deploy/           # systemd unit and deployment notes
├── .github/workflows/ # CI: build + test on Ubuntu and Windows
├── Cargo.toml         # Cargo workspace
├── Cargo.lock         # Committed for reproducible application builds
├── rust-toolchain.toml
├── AGENTS.md          # This file
└── README.md
```

Next: integrate the relay into `doip-client`; see the Current Status block.

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
- The Car puts the relay address into `server` (`--relay`, default
  `176.53.237.193:13500`); the Diag parses it from the code. The `role` field
  records the creator (Car), so the Diag still claims `diag` in its own `Hello`.
- The Diag listener defaults to `127.0.0.1:13400`; `doip_port` in the code is the
  **vehicle** port used by the Car, not the local listener.

## Relay protocol (plaintext, v1)

Implemented in `crates/doip-protocol`; shared by `doip-client` and
`doip-server`.

- Handshake only: `u32` big-endian length + compact JSON. Maximum frame 16 KiB.
  The body is read only after the length check, so oversized frames are rejected
  without allocation.
- Client: `{"type":"hello","v":1,"session":"…","token":"…","role":"car|diag","client":"…"}`.
  `session` is required (≤128 bytes), `token` required (≤256 bytes), `client`
  optional (≤64 bytes). Version must be `1`.
- Server: `waiting`, `paired`, or `error` with a snake_case `code`
  (`bad_frame`, `unsupported_version`, `invalid_hello`, `token_mismatch`,
  `role_taken`, `too_many_sessions`, `pending_timeout`, `handshake_timeout`,
  `server_shutdown`).
- After `paired`, framing stops and both sockets carry raw bytes; the relay is a
  transparent L4 pipe.
- Tests: 9 unit tests in `doip-protocol` cover round-trips, JSON shape, role
  serialization, validation, empty/oversized/invalid frames.
- Server rules (`crates/doip-server`): one Car + one Diag per session, matching
  token, duplicate role/token rejected, 10 s handshake timeout, 300 s pending
  TTL, 1024 pending sessions by default, 1 s control-write timeout.
- Tunnel: `tokio::io::copy_bidirectional` with `TCP_NODELAY`, graceful
  cancellation and byte counters in the log.
- CLI: `--listen` (default `0.0.0.0:13500`), `--handshake-timeout`,
  `--pending-timeout`, `--max-pending`; `RUST_LOG` controls log level.
- Deployed relay: `176.53.237.193:13500` (plaintext), systemd unit
  `doip-server.service`, binary `/usr/local/bin/doip-server`, user `doip`.
- 12 integration tests in `doip-server/tests/relay.rs` use real loopback TCP and
  cover forwarding, 1 MiB payloads, isolation, errors and shutdown.
- Client rules (`crates/doip-client/src/net.rs`): connect timeout 10 s, pairing
  wait 330 s (server expires at 300 s), vehicle dial timeout 10 s, local listener
  binds before pairing. The Car dials the vehicle after `Paired`; the Diag serves
  one local connection per session. Status events: `Connecting` → `Connected` →
  `Disconnected`; task completion sends `NetEvent::Finished`.
- Client CLI: `--relay`, `--vehicle`, `--diag-listen`, `--mock`; matching
  `DOIP_RELAY`, `DOIP_VEHICLE`, `DOIP_DIAG_LISTEN` environment variables.

## Current TUI behavior

- **Layout:** header (`=== DoIP Remote ===` plus key hints), middle row split
  70/30 into the menu and the dot-aligned status panel, bottom `[ LOGS ]`
  console. Modals are drawn on top. The header title is **white and bold**;
  the package version (`v0.1.0`) is shown in the top-right corner.
- **Key hints:** built from the `HOTKEYS` table in `ui.rs` and rendered as
  `[key] - action • ...`; key caps are bold yellow, separators and action
  descriptions are cyan. Includes `[1-5] - select`.
- **Startup driver check** (before the first frame):
  - Windows: Npcap is detected via
    `%SystemRoot%\System32\Npcap\wpcap.dll`; Loopback is detected by searching
    adapter name/description for `Loopback`/`KM-TEST` via `Get-NetAdapter`
    (`netsh` fallback). Admin rights are probed with `net session`.
  - Linux/non-Windows: stub returning `false`/`false`; admin is `false`.
- **Status panel:** `Admin Rights`, `Npcap` and `Loopback` (green `[OK]` / red
  `[MISSING]` — a missing value is **never** green), `Log File` (green `[EXISTS]`
  / red `[MISSING]`, based on `doip-remote.log`), and `Connection` (red
  `[DISCONNECTED]`, yellow `[CONNECTING]` while pairing/waiting, green
  `[CONNECTED ✓]` while bytes flow).
- **Menu:** `1. Install Drivers` / `2. I am near the car (Car)` /
  `3. I am a diagnostician (Diag)` / `4. Uninstall drivers and logs` (red) /
  `5. Exit`.
- **Navigation:** `1-5` select and trigger menu items immediately; `↑`/`↓` move
  (or scroll logs), `Enter` activates, `Tab` toggles menu/log focus,
  `PageUp`/`PageDown` scroll the log panel, `Ctrl+C` quits. `Esc` closes a
  popup; in the menu it quits.
- **Logs:** written to `doip-remote.log` on the user's Desktop
  (`[USERPROFILE]\Desktop\doip-remote.log` on Windows, `$HOME/Desktop/...` on
  Linux; falls back to `[home]/doip-remote.log` when `Desktop` is missing) and
  mirrored into the panel by a custom `tracing` layer (ring buffer, 1000 lines). Levels are
  colour-coded; admin warnings start with a bold red `[!]` marker. The file layer
  is reloadable: cleanup disables it while the in-memory mirror keeps working.
  A panic hook writes panic message, location and forced backtrace to
  `panic.log` next to the executable (gitignored via `*.log` and explicit entry).
  A separate `cleanup-trace.log` survives log deletion and records cleanup steps,
  console restoration and explicit exit reasons.
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
- **Car:** generates a `DT1-...` code with the configured relay address,
  immediately starts the Car network session (relay pairing, vehicle dial-out,
  bridge), logs the code and opens a popup where it is rendered in green. `Enter`
  copies it to the system clipboard and logs `[INFO] Code copied to clipboard!`;
  `Esc` closes the popup. A second session is refused while one is active.
- **Diag:** opens a popup; typed/pasted code is parsed on `Enter`. The relay
  address comes from the code; the client starts the Diag session, which binds
  `127.0.0.1:13400` (or `--diag-listen`) before pairing and serves the first
  local diagnostic connection.
- **Uninstall drivers and logs:** opens a confirmation popup (`Enter` confirms,
  `Esc` cancels; `Esc` only closes the modal). Confirmed cleanup is panic-safe
  (`catch_unwind`) and never exits/quits. Sequence: (1) cancel `CancellationToken`
  to disconnect sockets, (2) remove Npcap/Loopback using console-isolated child
  processes, (3) swap file logger to a no-op sink, drop the old appender guard,
  delete `doip-remote.log`, log `[INFO] Logging disabled, log file deleted`,
  (4) force driver/log status refresh, (5) re-show/re-title the console, re-enable
  raw mode and redraw, (6) set `is_cleaned_up=true`. Menu items 1-4 render
  DarkGray and inputs/hotkeys are ignored; only `5. Exit` remains active.
  Post-cleanup `Esc`/`q` are ignored and a stray `should_quit` without
  `exit_requested` is cleared. Statuses refresh to `Npcap [MISSING]` /
  `Loopback [MISSING]` / `Log File [MISSING]` / `Connection [DISCONNECTED]`.
  Later tracing calls continue through the in-memory UI layer and a kept-alive
  no-op file writer.
- **Exit:** if a session is starting or connected (`session_active`), shows a
  confirmation modal `[!] Active session. Disconnect and exit? (Enter/Esc)`;
  `Enter` cancels the network token, shuts down sockets, and exits, `Esc`
  cancels. If disconnected, exits immediately. After cleanup only `5. Exit` or
  `Ctrl+C` exits; `Esc`/`q` are ignored. All exit paths cancel the network token
  and wait for socket shutdown before process end.
- **`--mock`:** forces admin rights and both driver statuses green so the
  dashboard can be previewed on Linux. The network status is deliberately not
  faked; it only turns green for a real session.
- Networking is **real** on all platforms (the relay is plaintext); driver
  detection, Npcap installation/uninstallation and Loopback creation are real
  only on Windows. Loopback creation was runtime-tested on Windows 11; the other
  Windows paths were compile-checked.

## Commands

```sh
cd doip-remote
source "$HOME/.cargo/env"   # if cargo is not on PATH
cargo build --workspace
cargo test --workspace
cargo test -p doip-protocol
cargo test -p doip-server
cargo run -p doip-server -- --listen 127.0.0.1:13500
cargo run -p doip-client -- --help
cargo run -p doip-client -- --relay 127.0.0.1:13500 --vehicle 127.0.0.1:13400
cargo run -p doip-client -- --relay 127.0.0.1:13500   # Diag mode (menu 3)
# deployment (see deploy/README.md)
rsync -az --exclude target --exclude .git ./ root@176.53.237.193:/opt/doip-remote/src/
# fast path: build locally, copy the binary, restart the unit
cargo build --release -p doip-server
scp target/release/doip-server root@176.53.237.193:/root/doip-server
ssh root@176.53.237.193 'install -m 0755 /root/doip-server /usr/local/bin/doip-server.new && mv /usr/local/bin/doip-server.new /usr/local/bin/doip-server && systemctl restart doip-server'
cargo run -p doip-client -- --mock   # preview with admin/drivers emulated
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Windows cfg compile check from Linux (no linker needed)
rustup target add x86_64-pc-windows-msvc
cargo check -p doip-client --target x86_64-pc-windows-msvc
cargo clippy -p doip-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## Last verification (2026-11-26)

- `cargo build --release` — OK.
- `cargo test --workspace` — 29 `doip-client` + 17 `doip-core` tests + 1 doctest,
  all passing.
- Protocol checkpoint (2026-11-25): `cargo test -p doip-protocol -p doip-core`
  — 9 protocol + 17 core tests pass; `cargo clippy -p doip-protocol -p doip-core
  --all-targets -- -D warnings` — clean.
- Server checkpoint (2026-11-25): `cargo test -p doip-server` — 12 integration
  tests + 4 CLI tests + 1 config test pass; `cargo test --workspace` all green;
  `cargo clippy --workspace --all-targets -- -D warnings` clean. Windows target:
  `cargo check`/`clippy -p doip-server --target x86_64-pc-windows-msvc` clean.
- Deployment checkpoint (2026-11-25): `doip-server` is `active` and `enabled` on
  `doip-relay` (`176.53.237.193:13500`). Public smoke test: Car/Diag paired, a
  27-byte payload round-tripped both ways, and a wrong token returned
  `token_mismatch`. Journal shows `paired session` and
  `tunnel closed ... car_to_diag=27 diag_to_car=27`.
- Client checkpoint (2026-11-25): `cargo test -p doip-client` — 42 tests pass,
  including `net::tests::car_and_diag_forward_bytes_through_the_relay`, which
  runs real Car/Diag tasks against a loopback `doip-server` and an echo vehicle.
  `cargo test --workspace` all green; `cargo clippy --workspace --all-targets --
  -D warnings` and `cargo fmt --all --check` clean. Windows target
  `cargo check`/`clippy -p doip-client --target x86_64-pc-windows-msvc` clean
  (only the expected `llvm-rc` warning).
- Public client E2E (2026-11-25): both real `doip-client` binaries were driven
  through PTYs (Car menu 2, Diag menu 3) against `176.53.237.193:13500`; a
  30-byte payload echoed from a local vehicle simulator. Relay journal:
  `paired session ...` and
  `tunnel closed ... car_to_diag=30 diag_to_car=30 elapsed_ms=215`.
- Server rendezvous re-verification (2026-11-26): `cargo build -p doip-server`
  clean; `pairs_and_forwards_bytes_both_ways` passes; a standalone script
  against a local `doip-server` paired two sockets with the same session id and
  exchanged 9 bytes in each direction (`car_to_diag=9 diag_to_car=9`). No
  protocol rewrite was needed.
- VPS binary redeploy (2026-11-26): native `cargo build --release -p doip-server`
  succeeded in 21.5 s; the binary is 2.6 MB and needs at most `GLIBC_2.34`
  (VPS glibc `2.41`). `scp` + atomic `install` + `systemctl restart` left the
  service `active`/`enabled` on PID 18094, listening on `0.0.0.0:13500`.
  SHA-256 matched locally and remotely
  (`a4a19b181ba41212840f39fc9d06461d77f839b06e2333d9e8771795b37c304b`). Public
  pairing test `deploy-verify-1795684067`: `waiting` → `paired`, 10 bytes
  `car→diag`, 11 bytes `diag→car`, `elapsed_ms=15`. `ufw status numbered` shows
  only `22/tcp` and `13500/tcp` for IPv4 and IPv6.
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
- Windows 11 lifecycle check: after cleanup the process remained running
  (`exited=False` 8s later), `Log File` disappeared on disk, and the app
  continued accepting input until an explicit quit. Cleanup never sets
  `should_quit`, cancels the token, or shuts down the tokio runtime.
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
- Relay host: `176.53.237.193` (`root` SSH key already installed), Debian 13,
  hostname `doip-relay`, 2 vCPU / 3.8 GiB RAM / 2.4 GiB free. `ufw` is active and
  allows `22/tcp` and `13500/tcp`. Rust 1.100 (rustup, minimal profile),
  `build-essential`, git and rsync are installed. The service runs as the system
  user `doip` from `/usr/local/bin/doip-server`; source mirror is
  `/opt/doip-remote/src`.
- The `doip-relay` SSH alias is not configured on this workstation; use
  `root@176.53.237.193` (the key is already installed). The VPS runs Debian 13.7,
  hostname `doip-relay`, glibc `2.41`, which runs binaries built here against
  glibc `2.44` as long as they need at most `GLIBC_2.34`.
- The relay is plaintext-only for now (no TLS, no end-to-end encryption), so it
  is for development/staging. Do not expose real vehicle traffic yet.

## Conventions

- Edition 2021; stable Rust; `Cargo.lock` is committed.
- `doip-core` has `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`; keep
  public APIs documented. Isolate any unavoidable `unsafe` in the L2/driver
  layer later, never in the protocol core.
- UI strings are English; code, comments, README and commit messages are English.
- Add unit tests for every parsing/state-machine change.
- Do not commit `/target`, `.env`, credentials, tokens or generated keys.
- Before committing: fmt, clippy with `-D warnings`, workspace tests.

## Windows 11 validation checklist

These items remain open but are **non-blocking** for relay development. Validate
the client and its driver flow on real Windows 11:

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
- [ ] Validate the Car/Diag relay flow on real Windows 11 (driver checks,
      `127.0.0.1:13400` listener, clipboard code, Ctrl+C shutdown).

## Relay design decisions

Still open:

- [x] Wire framing: `u32` BE length-prefixed JSON for the handshake; raw bytes
      after pairing.
- [x] Handshake/authentication: `session_id` + `token`, one Car and one Diag per
      session, duplicate role and token mismatch rejected.
- [x] Pending-session expiry (300 s TTL) and a pending-session cap (1024).
      Reconnect/rate-limit policy is still open.
- [x] Relay address configuration: `--relay`/`DOIP_RELAY` (default
      `176.53.237.193:13500`), embedded in the session code by the Car.
- [ ] TLS termination at the relay and certificate strategy.
- [ ] Where X25519 + ChaCha20-Poly1305 fits: end-to-end or relay-terminated?
- [x] L4 proxy semantics: one relay session per DoIP TCP connection; raw bytes,
      no DoIP parsing, Diag local listener on 13400.
- [ ] L2 data path later: Npcap capture + KM-TEST Loopback integration.

## How to resume

1. Read this **Current Status** block and the server/client checkpoints.
2. `git log --oneline -5` and `git status` in `doip-remote`.
3. Run `cargo test --workspace` to confirm the baseline.
4. Continue with the next milestone in the **Current Status** block (TLS and
   encryption, or client hardening). The Windows 11 checklist is non-blocking.
   Update this file, then commit.
