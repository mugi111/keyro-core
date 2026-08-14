# Keyro Protocol

This directory is the source of truth for Keyro Core IPC protocol contracts.
The protocol currently lives inside `keyro-core`, but it is shaped so Keyro
Studio can consume the same schemas, test vectors, and generated TypeScript
artifacts without copying Core internals.

## Layout

```text
protocol/
├── LICENSE
├── package.json
├── scripts/check-package.mjs
├── schemas/v0.1.0/core-studio.schema.json
├── schemas/v0.2.0/core-studio.schema.json
├── test-vectors/v0.1.0/
├── test-vectors/v0.2.0/
├── generated/typescript/v0.1.0/core-studio.ts
└── generated/typescript/v0.2.0/core-studio.ts
```

The current Core/Studio protocol is `v0.2.0`. It adds `get_snapshot` so
clients can hydrate Core-owned MVP layout, profile state, and persisted
assignments after reconnect or restart.

## TypeScript Package

`protocol/` is also the package root for the Studio-facing
`@mugi111/keyro-protocol` TypeScript package. Publish this directory as an immutable
package release, then pin Studio to the exact package version. Before registry
publishing is available, create a tarball with `npm pack ./protocol` and
install that tarball in Studio for local verification.

Use versioned exports only:

```ts
import type { ClientEnvelope, ServerMessage } from "@mugi111/keyro-protocol/core-studio/v0.2.0";
import { KEYRO_PROTOCOL_VERSION } from "@mugi111/keyro-protocol/core-studio/v0.2.0";
```

Schema and test-vector artifacts are exported by versioned package paths:

```ts
import schema from "@mugi111/keyro-protocol/schemas/v0.2.0/core-studio";
import snapshotVector from "@mugi111/keyro-protocol/test-vectors/v0.2.0/snapshot-response";
```

The package intentionally does not expose an unversioned `core-studio` export.
Consumers must opt in to a concrete protocol version so minor `0.x` changes do
not silently alter the contract they compile against.

This first package release ships only `v0.2.0` artifacts. Older protocol
artifacts remain in this repository for Core compatibility tests, but are not
part of the initial npm package surface.

## Rules

- Treat JSON Schema files as the protocol source of truth.
- Keep `protocol/package.json` aligned with the current generated TypeScript,
  schema, and test-vector artifacts.
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
