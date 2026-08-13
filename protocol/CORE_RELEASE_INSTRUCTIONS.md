# Core Protocol Release Instructions

This document describes the Core-side work required before Studio can merge a
dependency on `@keyro/protocol`.

## Goal

Publish or otherwise produce an immutable `@keyro/protocol@0.2.0` artifact from
the Core-owned `protocol/` package.

Studio must not merge a dependency on a mutable sibling checkout such as:

```json
"@keyro/protocol": "file:../keyro-core/protocol"
```

That form is allowed only for local development and smoke testing.

## Preconditions

- Work from `keyro-core` `develop` after the protocol package export PR is
  merged.
- Confirm `protocol/package.json` has:
  - `"name": "@keyro/protocol"`
  - `"version": "0.2.0"`
  - `"license": "Apache-2.0"`
  - only versioned exports such as `./core-studio/v0.2.0`
- Confirm npm credentials can publish to the `@keyro` scope.

## Verification Before Publishing

Run from `/Users/mugi111/Documents/code/keyro-core`:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --locked
npm --prefix protocol run check:package
npm --prefix protocol pack --dry-run --json
```

Inspect the dry-run output. It should include only:

- `LICENSE`
- `README.md`
- `generated/javascript/v0.2.0/core-studio.js`
- `generated/typescript/v0.2.0/core-studio.ts`
- `package.json`
- `schemas/v0.2.0/core-studio.schema.json`
- `scripts/check-package.mjs`
- `test-vectors/v0.2.0/*.json`

It must not include Rust crates, storage files, app code, or `v0.1.0`
artifacts.

## Preferred Release Path: npm

Run from `/Users/mugi111/Documents/code/keyro-core/protocol`:

```bash
npm publish --access public
```

Then verify:

```bash
npm view @keyro/protocol@0.2.0 version
npm view @keyro/protocol@0.2.0 dist.integrity
```

Record the published version and integrity in the Studio handoff.

## Temporary Fallback: Immutable Tarball

Use this only if npm publication is blocked.

Run from `/Users/mugi111/Documents/code/keyro-core/protocol`:

```bash
npm pack
shasum -a 256 keyro-protocol-0.2.0.tgz
```

The tarball must be treated as immutable and tied to the merged Core commit.
Share the tarball path and SHA-256 with Studio. Do not ask Studio to depend on
`file:../keyro-core/protocol` for a mergeable PR.

## Handoff Checklist For Studio

Send Studio:

- Dependency source:
  - preferred: `@keyro/protocol@0.2.0` from npm
  - fallback: immutable `keyro-protocol-0.2.0.tgz` plus SHA-256
- Core commit used for the package.
- `npm view` version and integrity, or tarball checksum.
- Confirmation that `npm --prefix protocol run check:package` passed.
- Confirmation that `npm --prefix protocol pack --dry-run --json` contains only
  the expected v0.2.0 protocol files.
