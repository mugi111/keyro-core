# Keyro Core

Keyro Core is the lightweight resident runtime for Keyro devices and clients.
It owns durable state, SQLite persistence, local IPC, action routing, and small
OS adapters. Client applications such as Keyro Studio must communicate through
IPC and must not access Core storage directly.

This repository is in the first MVP implementation phase. The current focus is
the vertical slice needed to configure profiles, store assignments, receive
virtual control input, and route `open_url` actions.

## MVP Scope

- Rust workspace for Core domain, application, platform, SQLite, IPC, and binary
  crates.
- SQLite-backed profile and assignment persistence.
- Exactly one active profile at a time.
- Fixed MVP layout: 4 pages, 12 keys per page, 2 encoders per page with
  left/right/press operations.
- Single action per control in the initial API, with storage shaped to allow
  future ordered multi-action support.
- Safe `open_url` execution for `http` and `https` URLs only.
- Structured action events: `running`, `succeeded`, and `failed`.

## Protocol Status

`keyro-protocol` is the intended source of truth for Core, Studio, and Device
wire contracts. The protocol source currently lives in this repository under
`protocol/` so Core can own the contract while keeping schemas, shared test
vectors, and TypeScript artifacts easy for Studio to consume.

The IPC crate maps protocol DTOs into Core application commands. Keep protocol
DTOs separate from Core domain/application models so schema evolution does not
leak into Core internals.

Protocol `0.x` handshakes require an exact version match. Studio should consume
the checked-in schema, test vectors, and TypeScript artifact from `protocol/`
instead of copying Core internals.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --locked
```
