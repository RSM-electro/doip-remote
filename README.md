# doip-remote

> Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

`doip-remote` is an open-source toolkit for remote automotive diagnostics over
DoIP (Diagnostics over Internet Protocol, ISO 13400). A vehicle-side agent and a
diagnostic client exchange a compact, copy-pasteable session code to establish a
tunnel — no VPN setup and no vendor-specific software required.

> **Status (2026-11-17):** The client now uses a full ratatui dashboard: menu,
> live status panel and a scrolling log console. Driver installation runs in the
> background with progress in the log. Next: validate the build and driver flow
> on Windows 11. Server/relay work is postponed.
> See [AGENTS.md](AGENTS.md) for the full project context and next steps.

## Highlights

- **Universal** — works with any DoIP-capable vehicle or ECU.
- **Portable session codes** — a single `DT1-...` string carries everything the
  diagnostic side needs.
- **Dashboard UI** — ratatui terminal interface with a live driver/connection
  status panel and a scrolling log console.
- **Windows-ready driver checks** — detects Npcap and the KM-TEST Loopback
  Adapter, and installs them in the background.
- **Single binary client** — no configuration files.
- **Rust** — memory safety, predictable performance and easy cross-compilation.

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # Shared types: session codes, roles, errors
│   └── doip-client/   # Interactive terminal client
│       └── src/
│           ├── main.rs        # Event loop and menu actions
│           ├── app.rs         # Dashboard state
│           ├── ui.rs          # ratatui rendering
│           ├── tui.rs         # Terminal setup/restore and key reader
│           ├── logging.rs     # File + in-memory log routing
│           └── drivers/       # Driver detection and installation
├── .github/workflows/ # CI (Linux + Windows)
├── Cargo.toml         # Cargo workspace
└── rust-toolchain.toml
```

## Requirements

- Rust stable (installed automatically by `rustup` via `rust-toolchain.toml`)
- Linux or Windows (macOS is expected to work as well)

## Build

```sh
# from the repository root
cargo build --workspace
```

## Run the client

```sh
cargo run -p doip-client

# Preview the dashboard with emulated admin rights and drivers
cargo run -p doip-client -- --mock
```

The dashboard is split into a header, a menu and status panel, and a log
console at the bottom:

```
┌ Меню ──────────────────────────────┐┌ Статусы ────────────────────┐
│> Установить драйверы               ││ Права Админа........... [OK] │
│  Я рядом с машиной (Car)           ││ Npcap.................. [OK] │
│  Я диагност (Diag)                 ││ Loopback............... [OK] │
│  Завершить диагностику с удалением ││ Log File........... [EXISTS] │
│  Завершить диагностику             ││ Соединение...... [ОТКЛЮЧЕНО] │
│                                    ││                              │
└────────────────────────────────────┘└──────────────────────────────┘
┌ [ ЛОГИ ] ───────────────────────────────────────────────────────────┐
```

Menu entries:

1. **Установить драйверы** — downloads and installs missing drivers in the
   background (Windows; requires administrator rights).
2. **Я рядом с машиной (Car)** — generates a new `DT1-...` session code and
   shows it in a green popup (and in the log).
3. **Я диагност (Diag)** — opens a popup for pasting a session code.
4. **Завершить диагностику с удалением драйверов и логов** (red) — asks for
   confirmation, then removes drivers and `doip-remote.log` and exits.
5. **Завершить диагностику** — exits.

Keyboard:

| Key | Action |
| --- | ------ |
| `↑` / `↓` | Move the menu selection (or scroll logs) |
| `Enter` | Activate the selected entry |
| `Tab` | Switch focus between the menu and the log console |
| `PageUp` / `PageDown` | Scroll the log console (logs focused) |
| `Ctrl+C` | Quit immediately, restoring the terminal |

Logs are written both to `doip-remote.log` (next to the executable) and to the
console. The terminal is always restored on exit, on `Ctrl+C` and on panic.
Networking and driver removal are still stubs; the session-code generator/parser
and the Windows driver flow are real.

## Driver checks

| Driver | Detection |
| ------ | --------- |
| Npcap | `%SystemRoot%\System32\Npcap\wpcap.dll` exists |
| Loopback | Adapter name/description contains `Loopback` or `KM-TEST` (`Get-NetAdapter`, `netsh` fallback) |

The **Install** menu item runs in the background: it downloads the Npcap
installer from `npcap.com/dist/npcap-1.80.exe` with percentage progress in the
log, runs the installer **without** `/S` so the user can follow the wizard, then
installs the Loopback adapter from `%SystemRoot%\INF\netloop.inf` via
`InfDefaultInstall.exe` and forwards its stdout/stderr to the log. The menu is
locked until the installation finishes, then the statuses are refreshed.
Driver installation is Windows-only. On Linux the client reports that the
drivers are missing and refuses to install; use `--mock` to preview the dashboard
with emulated admin rights and drivers. The connection status is never emulated:
it stays red `[ОТКЛЮЧЕНО]` until the relay server is implemented.

## Session code format

A session code is a Base64Url-encoded (no padding) JSON document prefixed with
`DT1-`. JSON keys are intentionally short to keep codes compact:

| Key | Field        | Description                                  |
| --- | ------------ | -------------------------------------------- |
| `r` | `role`       | `car` or `diag`                              |
| `s` | `server`     | Relay server, e.g. `diag.example.com:13500`  |
| `n` | `session_id` | Unique session identifier                    |
| `t` | `token`      | Session token                                |
| `k` | `key`        | Session encryption key                       |
| `l` | `bridge`     | Whether bridge mode is enabled               |
| `p` | `doip_port`  | DoIP port (default `13400`)                  |
| `v` | `vehicle_ip` | Optional vehicle IP address                  |

Example (truncated):

```
DT1-eyJyIjoiY2FyIiwicyI6ImF1dG8iLCJuIjoi...
```

Whitespace inside a code is ignored when parsing, so codes copied from wrapped
terminal output can be pasted back as-is.

## Tests

```sh
cargo test --workspace
```

## License

Licensed under the [MIT License](LICENSE).
