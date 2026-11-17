# doip-remote — Agent Context

Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

This file is the persistent project memory for AI agents. Read it before making
changes. Update the **Current Status** block at the end of every working day,
and record important architecture decisions as they are made.

## Current Status (обновляй этот блок каждый день)

- **Date:** 2026-11-17
- **Phase:** Ratatui dashboard migration (stages 1-6) — in progress.
- **Done:** Stage 2/6 — `src/app.rs` holds the dashboard state (`App`,
  `Focus`, `ConnectionStatus`, `MenuItem`) with menu navigation, focus toggle,
  log scrolling and driver flags; covered by 8 unit tests.
- **Next Step:** Stage 3/6 — ratatui renderer and event loop: terminal setup,
  panic/Ctrl+C restore, header/menu/status/log layout, Tab and PageUp/PageDown.
- **Repo state:** branch `main`; the TUI is built stage by stage, each stage is
  committed as `wip: stage N completed`. Server/relay work is postponed.

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
- **UI:** terminal CLI built with `inquire`; no WebView2.
- **Roles:** `Car` (agent next to the vehicle) and `Diag` (remote technician).

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # SessionCode, Role, errors, session-code tests
│   └── doip-client/   # Interactive terminal client (binary)
│       └── src/
│           ├── main.rs        # Menu, status rendering, CLI flows
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
- Current CLI uses `server = "auto"` as a placeholder until the relay address is
  agreed.

## Current CLI behavior

- Banner: `=== Rust DoIP Remote v0.1.0 ===`.
- **Startup driver check** (before the menu):
  - Windows: Npcap is detected via
    `%SystemRoot%\System32\Npcap\wpcap.dll`; Loopback is detected by searching
    adapter name/description for `Loopback`/`KM-TEST` via `Get-NetAdapter`
    (`netsh` fallback).
  - Linux/non-Windows: stub returning `false`/`false`.
  - All present → green `[OK] Все драйверы установлены (Npcap, Loopback).`
  - Missing → red list `[!] ВНИМАНИЕ: Отсутствуют драйверы: ...` plus hints.
- Menu: Car / Diag / **Install drivers** / Cleanup / Exit.
- Install (Windows): downloads Npcap 1.80 from the official URL to the temp
  directory, runs it with `/S`, waits, then runs
  `InfDefaultInstall.exe %SystemRoot%\INF\netloop.inf` and waits. Afterwards the
  driver status is checked again and reprinted. On Linux this prints a stub.
- Car asks for the vehicle IP (empty = auto), generates a framed green
  `DT1-...` code, then shows the connection-wait stub.
- Diag prompts for a code, prints `Целевой IP авто`, `Порт DoIP` and the relay
  server, then shows the connection-wait stub.
- Cleanup prints `[STUB] Removing drivers...` and returns to the menu.
- Networking and driver removal are **stubs**; driver detection/installation is
  real only on Windows.

## Commands

```sh
cd doip-remote
source "$HOME/.cargo/env"   # if cargo is not on PATH
cargo build --workspace
cargo test --workspace
cargo run -p doip-client
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Windows cfg compile check from Linux (no linker needed)
rustup target add x86_64-pc-windows-msvc
cargo check -p doip-client --target x86_64-pc-windows-msvc
cargo clippy -p doip-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## Last verification (2026-11-17)

- `cargo build --workspace` — OK.
- `cargo test --workspace` — 17 `doip-core` + 4 `doip-client` tests + 1 doctest,
  all passing.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` and `cargo doc` with warnings denied — clean.
- Windows target code: `cargo check`/`clippy` for
  `x86_64-pc-windows-msvc` — clean (compile only, not runtime-tested).
- Linux PTY run: red missing-driver status, Install menu stub, menu navigation
  and exit — OK.
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
- [ ] Run as a normal user without Npcap/Loopback: confirm the red status.
- [ ] Install drivers from the menu as administrator; check UAC behaviour and
      installer exit codes.
- [ ] Confirm Npcap appears at `%SystemRoot%\System32\Npcap\wpcap.dll`.
- [ ] Confirm the KM-TEST adapter appears and is found by `Get-NetAdapter`.
- [ ] Confirm the status turns green after installation without a restart.
- [ ] Check detection on a localized Windows install (`Get-NetAdapter`/`netsh`
      output may be localized; adapter names/markers should still match).
- [ ] Test failure paths: declined UAC, missing `netloop.inf`, download error.
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
