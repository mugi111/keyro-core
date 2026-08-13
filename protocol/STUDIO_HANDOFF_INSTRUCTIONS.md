# Studio Protocol Package Handoff Instructions

This document is the message Core should hand to Studio after producing the
`@keyro/protocol@0.2.0` artifact.

## Required Dependency State

Studio must consume an immutable dependency. Use one of these forms:

```json
"@keyro/protocol": "0.2.0"
```

or, only as a temporary fallback:

```json
"@keyro/protocol": "file:/absolute/path/to/keyro-protocol-0.2.0.tgz"
```

Do not merge this form:

```json
"@keyro/protocol": "file:../keyro-core/protocol"
```

The sibling `file:` package is mutable and depends on local directory layout.

## Studio Migration Steps

Run from `/Users/mugi111/Documents/code/keyro-studio` on a fresh branch from
`develop`:

```bash
git fetch origin
git checkout develop
git merge --ff-only origin/develop
git checkout -b feature/consume-core-protocol-package-YYYYMMDD
```

Install the dependency:

```bash
bun add @keyro/protocol@0.2.0
```

If using the temporary tarball fallback, install the immutable tarball instead:

```bash
bun add @keyro/protocol@file:/absolute/path/to/keyro-protocol-0.2.0.tgz
```

## Expected Studio Code Changes

- Keep package imports in `src/infrastructure/main`.
- Make `src/infrastructure/main/core-protocol.ts` the local facade that imports
  and re-exports package DTOs/constants.
- Preserve Studio-owned mapper functions in the facade, such as:
  - `createHandshakeMessage`
  - `createClientEnvelope`
  - `snapshotToMessage`
  - `assignmentDtoFromAction`
  - action/control conversion helpers
- Delete `src/infrastructure/main/protocol-readiness.ts`.
- Do not import `@keyro/protocol` from:
  - `src/domain`
  - `src/application`
  - `src/shared`
  - `src/ui`

## Required Studio Tests

Add or keep tests that verify:

- `KEYRO_PROTOCOL_VERSION` is `0.2.0`.
- `KEYRO_PROTOCOL_HANDSHAKE_VERSION` is `{ major: 0, minor: 2 }`.
- `@keyro/protocol/schemas/v0.2.0/core-studio` resolves.
- `@keyro/protocol/test-vectors/v0.2.0/snapshot-response` resolves.
- Studio's infrastructure facade creates `ClientEnvelope` values typed against
  the package contract.
- Production source imports `@keyro/protocol` only from
  `src/infrastructure/main/core-protocol.ts`.

## Verification Commands

Run from `/Users/mugi111/Documents/code/keyro-studio`:

```bash
bun install --frozen-lockfile
bun run check
git diff --check
```

If Studio has a build pipeline available, also run:

```bash
bun run build
```

## PR Requirements

The Studio PR must include:

- `package.json` and `bun.lock` updated to the immutable dependency.
- The protocol facade migration.
- Package consumption tests.
- Import-boundary guard test.
- A note if the dependency is a temporary immutable tarball rather than npm
  `0.2.0`.
