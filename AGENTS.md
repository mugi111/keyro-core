# Keyro Core Agent Guide

This repository uses global Codex custom agents in `~/.codex/agents`.
Choose the smallest agent workflow that fits the task.

If custom agents are not available, treat the agent names below as human or
single-agent working modes. Follow the same responsibilities, permissions, and
handoff expectations manually.

## Required Workflow

For non-trivial implementation tasks, follow this workflow in order. A task is
non-trivial when it changes runtime behavior, architecture, public contracts,
security posture, persistence, build configuration, device communication, or
more than one layer. When a task might be non-trivial, run the Orchestrator Pass
first to classify and route the work before deciding which later phases are
needed.

Do not skip a phase just because the next step looks obvious. If a phase is not
applicable, record why in the handoff.

### 1. Orchestrator Pass

Use the `orchestrator` custom agent when available for task breakdown,
workflow routing, sequencing, and handoff management.

- Clarify the goal, success criteria, constraints, and open questions.
- Decide whether the task is trivial or non-trivial.
- Split non-trivial work into appropriate architect, implementer, and reviewer
  phases.
- Identify which files, layers, or responsibilities each phase should cover.
- Track dependency order, parallelizable work, risks, and expected verification.
- Do not modify files during this pass.
- If product intent, ownership, or priority is ambiguous, stop and ask before
  assigning implementation work.

### 2. Architect Pass

Use the `architect` custom agent when available. If custom agents are not
available, perform this pass manually before editing files.

- Read the relevant code, tests, configuration, protocol notes, and existing
  architecture notes.
- Identify affected layers and confirm dependency direction remains explicit
  and acyclic.
- Write a short implementation plan covering approach, affected files, risks,
  tests, and commit boundaries.
- Do not modify files during this pass.
- If the design is ambiguous or requires a product decision, stop and ask before
  implementing.

### 3. Implementer Pass

Use the `implementer` custom agent when available. If custom agents are not
available, treat this as a separate implementation phase after the architecture
pass is complete.

- Implement only the approved or clearly stated plan.
- Keep changes focused to the task and preserve existing APIs, schemas, and
  protocols unless the task requires changing them.
- Add or update tests at the closest layer to the behavior being changed.
- Keep business rules and device/protocol rules out of presentation or tooling
  code.
- Make commits at meaningful boundaries, such as foundation, domain/application
  logic, protocol/adapter work, persistence, tooling, tests, or security.
- Run the relevant verification commands before entering the review pass.

### 4. Reviewer Pass

Use the `reviewer` custom agent when available. If custom agents are not
available, use a separate human reviewer when possible. If no separate reviewer
is available, perform a distinct self-review after stepping away from the
implementation context.

- Review the final diff, not just the final files.
- Check requirements, dependency direction, public contracts, protocol
  compatibility, security rules, test coverage, maintainability, and regression
  risk.
- Classify findings as `BLOCKER`, `MAJOR`, `MINOR`, or `NIT`.
- Fix all `BLOCKER` and `MAJOR` findings before handoff.
- If only `MINOR` or `NIT` findings remain, mention them in the handoff.

### 5. Handoff

Before final handoff, report:

- Which workflow phases were completed and whether custom agents or manual
  passes were used.
- Changed files and a concise change summary.
- Tests added or updated.
- Verification commands and results.
- Remaining risks or skipped phases, if any.

## Custom Agents

### Orchestrator

Use `orchestrator` for task intake, task decomposition, workflow routing,
sequencing, coordination across agents, and final handoff readiness.

Without the custom agent, perform a manual orchestration pass before deciding
which specialist phases are needed.

- Confirm goal, constraints, success criteria, and ambiguity.
- Choose the smallest workflow that safely fits the task.
- Break work into clear phases and ownership boundaries.
- Decide when architect, implementer, and reviewer passes are required.
- Track handoff requirements, verification expectations, and unresolved risks.
- Do not modify files.
- Do not make detailed architecture decisions that belong to the architect.
- Do not implement code or perform final code review.

### Architect

Use `architect` for requirements analysis, feature design, domain modeling,
protocol design, storage design, API contract changes, or any task where the
implementation approach is not yet settled.

Without the custom agent, do an architecture pass first and write the design
notes before implementation begins.

- Read the existing code before proposing changes.
- Do not modify files.
- Produce a design another engineer can implement without revisiting major
  architecture decisions.
- Call out affected files, risks, and implementation order.

### Implementer

Use `implementer` for approved, concrete code changes.

Without the custom agent, switch into implementation mode only after the design
is clear enough to execute.

- Read nearby code and tests before editing.
- Keep changes focused and avoid speculative abstractions.
- Preserve existing APIs unless the requested behavior requires a change.
- Add or update tests when behavior changes.
- Stop and explain if the design requires an architectural decision first.

### Reviewer

Use `reviewer` for independent review before merge or after non-trivial edits.

Without the custom agent, have a person other than the implementer review when
possible. If that is not possible, do a separate self-review pass after a short
context reset.

- Do not modify files.
- Review requirements, architecture, conventions, tests, maintainability, and
  regression risk.
- Prioritize meaningful correctness and design issues over style preferences.
- Classify findings as `BLOCKER`, `MAJOR`, `MINOR`, or `NIT`.

## Repository Architecture

Keyro Core is the runtime and source-of-truth layer for Keyro devices and
clients. It should own durable state, device-facing behavior, protocol
contracts, and validation that must remain consistent across clients such as
Keyro Studio.

The repository is currently minimal. As implementation is added, keep clear
boundaries between these responsibilities:

- Core domain rules: pure rules for profiles, pages, keys, encoders, actions,
  device layouts, validation, and compatibility.
- Application services: use cases that coordinate domain logic, persistence,
  device operations, and client-facing commands.
- Protocol contracts: schemas and message types used by clients, device
  transports, plugins, or automation layers.
- Infrastructure: persistence, transport adapters, filesystem, process,
  network, OS, device, and hardware-specific code.
- Tooling and clients: CLIs, development utilities, test harnesses, and any
  optional local UI used to inspect or operate Core.

Prefer dependency direction that keeps domain logic independent from
infrastructure and tools. Infrastructure should depend inward on contracts and
application interfaces, not the other way around.

## Development Rules

- Preserve Core as the source of state. Client applications may request and
  render state, but should not become authoritative for Core-owned behavior.
- Treat protocols, schemas, storage formats, and public APIs as compatibility
  surfaces. Version or migrate them deliberately.
- Keep expected validation failures explicit, typed, and testable. Avoid using
  uncaught exceptions for normal user, client, or device input errors.
- Device layout is variable. Do not hard-code key, encoder, page, or capability
  counts when layout or capability data is available.
- URL actions must allow only `http` and `https` unless a task explicitly
  changes that security policy.
- Keep hardware, OS, filesystem, socket, process, and database details out of
  pure domain code.
- Avoid broad refactors unless the requested change requires them.

## Commands

Run commands from the repository root. This repository does not yet define a
package manager, build system, or test command. When those are introduced,
update this section with the canonical commands.

Until then, use the closest available verification for the files being changed,
such as formatting, linting, typechecking, tests, or documentation rendering.

## Testing Expectations

- Domain rule changes should have focused unit tests near the domain code.
- Application behavior changes should test use cases and error paths.
- Protocol, adapter, transport, or persistence behavior should include
  integration-style tests where practical.
- Security-sensitive changes should test rejected inputs as well as accepted
  inputs.
- Compatibility changes should include migration, versioning, or contract tests
  when relevant.
- Documentation-only changes should be checked for accuracy against the current
  repository state.

## Handoff Expectations

For implementation work, report:

- Changed files
- What changed
- Tests added or updated
- Verification commands and results
- Remaining concerns, if any

For reviews, lead with findings ordered by severity and include precise file
and line references.
