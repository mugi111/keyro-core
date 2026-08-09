# Keyro Protocol

This directory is the source of truth for Keyro Core IPC protocol contracts.
The protocol currently lives inside `keyro-core`, but it is shaped so Keyro
Studio can consume the same schemas, test vectors, and generated TypeScript
artifacts without copying Core internals.

## Layout

```text
protocol/
├── schemas/v0.1.0/core-studio.schema.json
├── test-vectors/v0.1.0/
└── generated/typescript/v0.1.0/core-studio.ts
```

## Rules

- Treat JSON Schema files as the protocol source of truth.
- Keep Core domain/application models separate from protocol DTOs.
- Commit generated artifacts so Studio can consume them before packaging is
  automated.
- Keep Rust and TypeScript contract tests aligned with the same test vectors.
- TypeScript declarations do not enforce JSON Schema formats, regex patterns,
  or numeric ranges at runtime. Studio should validate IPC JSON with the schema.
- Protocol `0.x` releases require exact version negotiation during handshake.
- Each IPC connection must complete a successful handshake before normal
  command routing. Commands received before handshake, or after an incompatible
  handshake, return structured errors and must not stop Core.
- A failed handshake may be retried on the same connection. Once a connection
  has completed handshake, later handshake messages are rejected as validation
  errors without closing the established session. Reconnects start with a fresh
  session and must handshake again.
- Minor additions must use a new protocol version until a stable compatibility
  policy is introduced.
- Unknown messages must produce structured errors and must not stop Core.
