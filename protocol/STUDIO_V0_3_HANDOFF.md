# Keyro Studio v0.3 Protocol Handoff

## Core Release

Publish the protocol package after the Core PR is merged into `develop` and the
local checkout is updated to that merge commit.

```bash
cd /Users/mugi111/Documents/code/keyro-core/protocol
npm publish --access public --registry=https://registry.npmjs.org/ --otp=YOUR_OTP
```

Verify the published artifact:

```bash
npm view @mugi111/keyro-protocol@0.3.0 version dist.integrity --registry=https://registry.npmjs.org/
```

Expected package:

- package: `@mugi111/keyro-protocol@0.3.0`
- protocol export: `@mugi111/keyro-protocol/core-studio/v0.3.0`
- schema export: `@mugi111/keyro-protocol/schemas/v0.3.0/core-studio`

## Studio Goal

Update Studio to consume Core protocol `v0.3.0` and use the newly supported
Core-owned mutations over local IPC:

- `create_profile`
- `rename_profile`
- `clear_assignment`

## Studio Branch

Create a fresh branch from Studio `develop`. Do not reuse previous protocol
package branches.

```bash
cd /Users/mugi111/Documents/code/keyro-studio
git fetch origin
git checkout develop
git merge --ff-only origin/develop
git checkout -b feature/studio-core-profile-commands-20260815
```

## Dependency Update

Pin the published package exactly:

```json
"@mugi111/keyro-protocol": "0.3.0"
```

Run:

```bash
bun install
```

Commit the regenerated `bun.lock`.

## Protocol Imports

Update Studio protocol imports from:

```ts
@mugi111/keyro-protocol/core-studio/v0.2.0
```

to:

```ts
@mugi111/keyro-protocol/core-studio/v0.3.0
```

Schema and vector imports should also move from `v0.2.0` to `v0.3.0`.

Keep production imports of `@mugi111/keyro-protocol` confined to Studio's
infrastructure protocol facade.

## Local IPC Adapter Behavior

Implement the currently unsupported Studio local IPC paths:

- `createProfile(name)`
  - send `{ type: "create_profile", name }`
  - require a `profile` response
  - map the returned Core-owned profile DTO into Studio's domain profile shape
  - refresh snapshot after creation if the UI workflow needs current pages/state

- `renameProfile(profileId, name)`
  - send `{ type: "rename_profile", profile_id: profileId, name }`
  - require a `profile` response
  - map the returned Core-owned profile DTO into Studio's domain profile shape
  - refresh snapshot after rename if the UI workflow needs current state

- assignment clearing in `savePage`
  - when a changed action becomes `null`, send:

```ts
{
  type: "clear_assignment",
  profile_id: profileId,
  control
}
```

  - require `acknowledged`
  - refresh the authoritative Core snapshot after mutation

Keep the existing single-change limitation unless a separate Studio task
explicitly introduces batching.

## Error Handling

Map Core `not_found` responses to Studio validation/user-facing errors for
stale profile references. Keep `validation_failed` for invalid names, invalid
controls, invalid URLs, and exact-version handshake failures.

## Expected Tests

Run from Studio root:

```bash
bun install --frozen-lockfile
bun run typecheck
bun test
bun run check
git diff --check
```

Add or update focused tests for:

- package constants, schema, and vectors resolving from `v0.3.0`
- exact v0.3 handshake constants
- local IPC `createProfile` success and malformed/unexpected response handling
- local IPC `renameProfile` success and `not_found` handling
- local IPC assignment clear success, idempotent clear behavior, and snapshot refresh
- production protocol package imports staying inside the infrastructure facade

## Notes

- Core keeps profile names exactly as supplied except all-whitespace names are
  rejected.
- New profiles are inactive. Studio should activate them explicitly if the UI
  wants that behavior.
- Duplicate profile names remain allowed.
- `clear_assignment` is idempotent for an existing profile and exact control.
- Protocol `0.x` still requires exact `{ major: 0, minor: 3 }` negotiation.
