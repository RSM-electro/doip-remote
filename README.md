# doip-remote

> Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

`doip-remote` is an open-source toolkit for remote automotive diagnostics over
DoIP (Diagnostics over Internet Protocol, ISO 13400). A vehicle-side agent and a
diagnostic client exchange a compact, copy-pasteable session code to establish a
tunnel — no VPN setup and no vendor-specific software required.

> **Status (2026-11-25):** The client has a full ratatui dashboard, driver
> management, clipboard session codes and graceful lifecycle handling. The
> plaintext relay (`doip-server`) is implemented and deployed at
> `176.53.237.193:13500`. Next: integrate the relay into the client and validate
> L4 end-to-end. TLS comes after the plaintext path is proven.
> See [AGENTS.md](AGENTS.md) for the full project context and next steps.

## Highlights

- **Universal** — works with any DoIP-capable vehicle or ECU.
- **Portable session codes** — a single `DT1-...` string carries everything the
  diagnostic side needs.
- **Dashboard UI** — ratatui terminal interface with a live driver/connection
  status panel, a version label and a scrolling log console.
- **Windows-ready driver checks** — detects Npcap and the KM-TEST Loopback
  Adapter, and installs them in the background.
- **Plaintext relay server** — pairs a Car and a Diag by session id/token, then
  forwards raw DoIP bytes; deployed on a small Linux host.
- **Single binary client** — no configuration files.
- **Rust** — memory safety, predictable performance and easy cross-compilation.

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # Shared types: session codes, roles, errors
│   ├── doip-protocol/ # Relay handshake messages and framing
│   ├── doip-server/   # Plaintext relay server
│   └── doip-client/   # Interactive terminal client
│       └── src/
│           ├── main.rs        # Event loop and menu actions
│           ├── app.rs         # Dashboard state
│           ├── ui.rs          # ratatui rendering
│           ├── tui.rs         # Terminal setup/restore and key reader
│           ├── logging.rs     # File + in-memory log routing
│           └── drivers/       # Driver detection and installation
├── deploy/            # systemd unit and deployment notes
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

On Windows the executable automatically gets the application icon from
`crates/doip-client/icon.png`: `build.rs` converts it into a multi-size
PNG-compressed `.ico` (16/32/48/64/128/256) and links it with `embed-resource`,
so Explorer and the taskbar show it. No manual conversion is required.

When cross-compiling for Windows from Linux without `rc`/`lld` resource tools
(`llvm-rc` or `windres`), the build still succeeds but prints a warning that the
icon was skipped. Build on Windows (or install one of those tools) to get the
icon embedded in the final `.exe`.

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
│  Удалить драйверы и логи           ││ Log File........... [EXISTS] │
│  Завершить диагностику             ││ Соединение...... [ОТКЛЮЧЕНО] │
│                                    ││                              │
└────────────────────────────────────┘└──────────────────────────────┘
┌ [ ЛОГИ ] ───────────────────────────────────────────────────────────┐
```

Menu entries:

1. **Установить драйверы** — downloads and installs missing drivers in the
   background (Windows; requires administrator rights).
2. **Я рядом с машиной (Car)** — generates a new `DT1-...` session code and
   shows it in a green popup (and in the log). Press `Enter` to copy it to the
   clipboard (`Code copied to clipboard!`), `Esc` to close.
3. **Я диагност (Diag)** — opens a popup for pasting a session code.
4. **Удалить драйверы и логи** (red) — asks for confirmation, removes the
   drivers and deletes the log file, then keeps the app running with the
   refreshed statuses.
5. **Завершить диагностику** — exits.

Keyboard:

| Key | Action |
| --- | ------ |
| `↑` / `↓` | Move the menu selection (or scroll logs) |
| `Enter` | Activate the selected entry (copy the Car code when its popup is open) |
| `Tab` | Switch focus between the menu and the log console |
| `PageUp` / `PageDown` | Scroll the log console (logs focused) |
| `Ctrl+C` | Quit immediately, restoring the terminal and closing network sockets |

Logs are written both to `doip-remote.log` on the user's Desktop
(`%USERPROFILE%\Desktop`, or `~/Desktop`; the home directory is used if
`Desktop` does not exist) and to the console. The terminal is always restored
on exit, on `Ctrl+C` and on panic.
Networking on the client is still a stub; the session-code generator/parser,
the Windows driver flow and the relay server are real. Choosing **Удалить
драйверы и логи** closes the file writer before deleting the log, so the app
keeps running without Windows sharing violations and shows the refreshed
`[НЕТ]` / `[MISSING]` statuses.

Every exit path (Finish diagnostics, `Esc`, `q`, `Ctrl+C`) cancels a shared
`CancellationToken`. `net::NetworkService` waits for its spawned tasks, drops
listeners so their ports are released, and closes accepted streams with
`linger(0)` to avoid leftover `TIME_WAIT` sockets.

## Run the relay server

```sh
# Local development
cargo run -p doip-server -- --listen 127.0.0.1:13500

# Show all options
cargo run -p doip-server -- --help
```

The relay accepts a length-prefixed JSON `Hello` (`session`, `token`, `role`),
pairs one Car with one Diag, then forwards raw bytes between them. It does not
parse DoIP. Defaults: 10 s handshake timeout, 300 s pending-session TTL, 1024
pending sessions, control frames capped at 16 KiB.

The reference deployment runs at `176.53.237.193:13500` under systemd; see
[`deploy/README.md`](deploy/README.md) for the unit file and update steps.

> **Plaintext only:** the handshake and tunnel are not encrypted yet. Use this
> for development and staging, not for real vehicle traffic. TLS is the next
> backend milestone after client integration.

## Driver checks

| Driver | Detection |
| ------ | --------- |
| Npcap | `%SystemRoot%\System32\Npcap\wpcap.dll` exists |
| Loopback | Adapter name/description contains `Loopback` or `KM-TEST` (`Get-NetAdapter`, `netsh` fallback) |

The **Install** menu item runs in the background: it downloads the Npcap
installer from `npcap.com/dist/npcap-1.80.exe` with percentage progress in the
log, runs the installer **without** `/S` so the user can follow the wizard, then
creates the Loopback adapter by calling
`newdev.dll!UpdateDriverForPlugAndPlayDevicesW` with
`%SystemRoot%\INF\netloop.inf` and the `*msloop` hardware ID
(`INSTALLFLAG_FORCE | INSTALLFLAG_NONINTERACTIVE`). The reboot flag returned by
the API is logged. The menu is locked until the installation finishes, then the
statuses are refreshed.
Driver installation is Windows-only. On Linux the client reports that the
drivers are missing and refuses to install; use `--mock` to preview the dashboard
with emulated admin rights and drivers. The connection status is never emulated:
it stays red `[ОТКЛЮЧЕНО]` until the relay server is implemented.

Uninstalling runs the official Npcap uninstaller
(`C:\Program Files\Npcap\NPFInstall.exe -u`) and waits for it to finish. The
client never opens Npcap files, so no capture handle blocks the uninstaller. The
KM-TEST Loopback device is currently left in place.

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
cargo test -p doip-protocol
cargo test -p doip-server
```

## License

Licensed under the [MIT License](LICENSE).
