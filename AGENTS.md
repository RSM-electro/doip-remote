# doip-remote — Agent Context

Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

This file is the persistent project memory for AI agents. Read it before making
changes. Update the **Current Status** block at the end of every working day,
and record important architecture decisions as they are made.

## Current Status (обновляй этот блок каждый день)

- **Date:** 2026-11-20
- **Phase:** Windows polish and lifecycle work in progress (4 checkpoints).
- **Done:** Stage 6/6 — `--mock` mode and final verification. The client is now a
  ratatui dashboard with a menu, dot-aligned status panel, scrolling log console,
  background driver installation, UAC gate and a cleanup modal. `inquire` is gone.
  Follow-up: the key-hint row is now readable (bold yellow keys, cyan actions),
  and `--mock` no longer fakes the connection: the status stays `Disconnected`
  (`[ОТКЛЮЧЕНО]`) until the relay server exists.
  UI polish: magenta bold header, strictly red `[НЕТ]`/`[MISSING]` statuses, a
  `Log File` status row, a bold red `[!]` admin warning marker, and a green
  session-code popup after the Car action.
- **Next Step:** Continue the Windows polish series: menu/lifecycle split, safe
  log deletion, graceful network shutdown, driver fixes and clipboard/version
  extras. After that, validate on real **Windows 11** (full checklist below).
  Server/relay work (`doip-server`) remains postponed.
- **Repo state:** branch `main`; stages 1-5 are committed as
  `wip: stage N completed`, stage 6 as the final dashboard commit, followed by
  `fix(ui): improve hotkeys readability with yellow/cyan colors`,
  `fix(state): force connection status to Disconnected until server is implemented`
  and the Windows-polish checkpoints.

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
- **Roles:** `Car` (agent next to the vehicle) and `Diag` (remote technician).

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # SessionCode, Role, errors, session-code tests
│   └── doip-client/   # Interactive ratatui client (binary)
│       └── src/
│           ├── main.rs        # Event loop, menu actions, install events
│           ├── app.rs         # App state, menu items, focus, scroll
│           ├── ui.rs          # ratatui layout, status rows, modals
│           ├── tui.rs         # Terminal init/restore, panic hook, key reader
│           ├── logging.rs     # File + in-memory log layers
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
  console. Modals are drawn on top. The header title is **magenta and bold**.
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
- **Menu:** Install / Car / Diag / Cleanup (red) / Exit.
- **Navigation:** `↑`/`↓` move (or scroll logs), `Enter` activates, `Tab`
  toggles menu/log focus, `PageUp`/`PageDown` scroll the log panel, `Ctrl+C`
  quits. `Esc` closes a popup; in the menu it quits.
- **Logs:** written to `doip-remote.log` next to the executable and mirrored into
  the panel by a custom `tracing` layer (ring buffer, 1000 lines). Levels are
  colour-coded; admin warnings start with a bold red `[!]` marker.
- **Install (Windows):** a background task streams the Npcap 1.80 download with
  percentage progress in the log, runs the installer **without** `/S`, captures
  `InfDefaultInstall.exe %SystemRoot%\INF\netloop.inf` stdout/stderr into the
  log, then sends a completion event. The menu is locked meanwhile and statuses
  refresh afterwards. Without admin rights the action only logs an instruction
  to restart as Administrator.
- **Car:** generates a `DT1-...` code (server `"auto"`), logs it and opens a
  popup where the code is rendered in green.
- **Diag:** opens a popup; typed/pasted code is parsed on `Enter`, and the
  target IP, DoIP port and relay server are logged.
- **Cleanup:** opens a `Y/N` confirmation popup. `Y` calls the driver-removal
  stub, deletes `doip-remote.log` and exits; on Windows the deletion is retried
  after the log writer is dropped.
- **`--mock`:** forces admin rights and both driver statuses green so the
  dashboard can be previewed on Linux. The network status is deliberately not
  faked and stays red `[ОТКЛЮЧЕНО]` until the server exists.
- Networking and driver removal are **stubs**; driver detection/installation is
  real only on Windows.

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

## Last verification (2026-11-17)

- `cargo build --workspace` — OK.
- `cargo test --workspace` — 18 `doip-client` + 17 `doip-core` tests + 1 doctest,
  all passing.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` and `cargo doc` with warnings denied — clean.
- Windows target code: `cargo check`/`clippy` for
  `x86_64-pc-windows-msvc` — clean (compile only, not runtime-tested).
- Linux PTY runs — OK: red status dashboard; menu navigation; Tab/PgUp/PgDn log
  focus and scrolling; Car code generation; Diag popup and parse-error log;
  Install admin gate; cleanup modal (cancel + confirm, log file deleted);
  `--mock` dashboard with green admin/drivers and red `[ОТКЛЮЧЕНО]`; Ctrl+C
  restores the alternate screen, cursor and raw mode. The hint row renders bold
  yellow `[ключ]` caps and cyan descriptions (SGR `38;5;3` / `38;5;6`).
- Actual driver detection/installation on real Windows 11 is **not yet tested**.

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

- [ ] `cargo build --workspace` and `cargo test --workspace` on Windows 11.
- [ ] Confirm the dashboard renders correctly in Windows Terminal (colours,
      borders, Cyrillic, `✓`).
- [ ] Run as a normal user without Npcap/Loopback: confirm the yellow admin
      status, red driver statuses and the admin warning on Install.
- [ ] Install drivers from the menu as administrator; follow the interactive
      Npcap wizard and check UAC behaviour and installer exit codes.
- [ ] Confirm download progress and `InfDefaultInstall` output appear in the log
      panel and in `doip-remote.log`.
- [ ] Confirm Npcap appears at `%SystemRoot%\System32\Npcap\wpcap.dll`.
- [ ] Confirm the KM-TEST adapter appears and is found by `Get-NetAdapter`.
- [ ] Confirm the status turns green after installation without a restart.
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
