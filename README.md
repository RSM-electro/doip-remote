# doip-remote

> Universal remote DoIP (ISO 13400) tunnel for automotive diagnostics. Written in Rust.

`doip-remote` is an open-source toolkit for remote automotive diagnostics over
DoIP (Diagnostics over Internet Protocol, ISO 13400). A vehicle-side agent and a
diagnostic client exchange a compact, copy-pasteable session code to establish a
tunnel — no VPN setup and no vendor-specific software required.

> **Status:** early skeleton (`v0.1.0`). The CLI, session-code format and
> workspace layout are in place. Networking, transport and driver integration
> are the next milestones.

## Highlights

- **Universal** — works with any DoIP-capable vehicle or ECU.
- **Portable session codes** — a single `DT1-...` string carries everything the
  diagnostic side needs.
- **Single binary client** — a friendly terminal UI, no configuration files.
- **Rust** — memory safety, predictable performance and easy cross-compilation.

## Workspace layout

```
doip-remote/
├── crates/
│   ├── doip-core/     # Shared types: session codes, roles, errors
│   └── doip-client/   # Interactive command-line client
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
```

The client opens an interactive menu:

1. **Я рядом с машиной (Car)** — generates a new `DT1-...` session code.
2. **Я диагност (Diag)** — accepts a session code and shows the target vehicle.
3. **Удалить драйверы (Очистка)** — driver cleanup (stub for now).
4. **Выход** — quit.

Networking and driver handling are stubbed in this release; the menu already
exercises the real session-code generator and parser.

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
