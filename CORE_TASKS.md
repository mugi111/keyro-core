# Core Status and Remaining Work

Reviewed on 2026-09-10 against `keyro-core.md` and the system guide in
`keyro.md` provided in Downloads. This is a tracked implementation backlog,
not a replacement for the product specifications.

## Implemented Baseline

- Core develop base: `6582789`; protocol contract and npm exports: v0.3.0.
- Domain, application, protocol adapter, SQLite and OS adapters are separate
  workspace crates. Core does not depend on Studio.
- SQLite stores profiles and ordered assignments. The public command API accepts
  one action per control. Exactly one profile is active.
- Profile creation, rename, activation, assignment save/clear, snapshot readback,
  virtual input and open_url action result events are implemented.
- SQLite reopen tests, protocol vectors, single-instance locking and rotating
  logs exist. URLs are restricted to HTTP/HTTPS.
- The MVP layout is fixed as specified: four pages, twelve keys, two encoders.
- Studio's startup handshake and hydration fixes are merged into its remote
  develop (`2cf34fd`). The local Studio checkout is still on the earlier fix
  branch; this review does not change that checkout.
- Protocol remains inside Core under `protocol/`, with versioned exports for
  Studio. Do not copy Core internal models into Studio.

## Current Branch

`fix/core-dev-ipc-concurrency-20260910`

- Complete: independently serve up to eight Unix development IPC connections.
- Complete: log connection IDs, acceptance, failure and closure; reclaim exited
  workers on the next connection, without periodic polling.
- Complete: regression coverage for simultaneous snapshots, idle connection
  limits and reconnection after clients disconnect.
- The ninth concurrent connection is closed without a protocol response.
- This is development transport support, not production IPC. Per-client events
  are not broadcast to other clients; clients must request fresh snapshots.

## Prioritized Backlog

| Priority | Type | Task | Completion Criteria / Dependencies |
| --- | --- | --- | --- |
| P0 | Functional / verification | Complete Core + Studio macOS smoke test | Connected UI; create/rename/activate profile; save/clear assignment; virtual input opens URL and reports result; restart Core and Studio, verify reconnect and persisted state. No device required. See LOCAL_IPC_SMOKE_TEST.md. |
| P1 | Non-functional | Bound development IPC input and stalled clients | Limit JSON line size before allocation grows unbounded; bound handshake and write waits; slow or malformed clients do not consume all connection slots indefinitely. Add oversized/partial-input and slow-reader tests. |
| P1 | Functional | Implement Windows named-pipe transport | Replace the non-Unix parked fallback; handshake, commands, events and reconnect use the same contract. Verify on Windows 11 x64. |
| P1 | Non-functional | Secure and define production IPC | Restrict endpoints to the current user; define framing, backpressure, shutdown and compatibility with Studio before changing wire format. Test denied access and overload behavior. |
| P1 | Non-functional | Measure resource and dispatch targets | Record idle RSS <= 50 MB, average CPU < 0.5%, and input-to-action-start <= 50 ms with reproducible duration and environment. Use a fake opener for dispatch timing; also measure real macOS behavior. |
| P2 | Non-functional | Automate shared contract and OS checks | Add CI for format, clippy, workspace tests, locked build and package checks; validate schema/type consistency and shared negative vectors. Review exact-match 0.x policy against system compatibility guidance before changing it. |
| P2 | Non-functional | Harden storage evolution and failure recovery | Reject unsupported future DB versions, test interrupted migration and lock/error paths, and define backup/recovery behavior before a schema change. Existing migration v1 and reopen tests are not full upgrade coverage. |

The macOS MVP has an implemented vertical slice, but its end-to-end acceptance
and resource targets are not yet recorded as passing. Windows support is
incomplete. A successful unit test run does not close either gap.

## Deferred by Specification

Physical-device completion, QuickJS-ng integrations, credential-backed services,
automatic updates and multi-action macros are outside the initial milestone.
Do not persist the last displayed page without a product decision. Hardware
absence does not block the virtual-input milestone.

## Implementation and Review Notes

This branch uses manual orchestration, architecture, implementation and review
passes, with no subagents. Runtime changes are confined to the binary's Unix
development transport. Workers share the existing SQLite mutex and transactional
repository; no schema, public command or protocol version changes are needed.
The connection cap bounds worker count but does not bound message size or total
memory. That limitation is explicitly tracked above.

Verification: `cargo test --workspace` passed 65 tests, including two new
connection regressions. Format, clippy with warnings denied, locked workspace
build, protocol package check and `git diff --check` passed. Socket binding
required running tests outside the filesystem sandbox. Manual final-diff review
found no BLOCKER or MAJOR issues within this branch's scope; no workflow phases
were skipped. Studio UI acceptance and Windows execution were not performed.
