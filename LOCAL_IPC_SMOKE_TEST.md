# macOS Local IPC Smoke Test

No physical device is required. Use Core with the concurrent development listener
and Studio develop including `2cf34fd`. Close old Core/Studio processes before
starting; keep user data intact.

## Start Core

From the Core repository:

```sh
cargo run -p keyro-core
```

Core writes logs to a file, not the launching terminal. In another terminal:

```sh
tail -f "$HOME/Library/Logs/Keyro/Core/keyro-core.log"
```

Expect `development IPC listener started`. Connection logs include a
`connection_id` for acceptance, errors and closure. Acceptance alone does not
prove a successful protocol handshake.

## Start Studio

From the Studio repository on the intended revision:

```sh
bun install --frozen-lockfile
env KEYRO_STUDIO_CORE_MODE=local-ipc \
  KEYRO_STUDIO_CORE_SOCKET="$HOME/Library/Application Support/Keyro/Core/keyro-core-dev.sock" \
  bun run dev
```

Expect Connected and enabled profile controls. A visible Default profile alone
does not prove the UI has completed connection setup.

## Probe While Studio Is Connected

Run in a separate terminal:

```sh
printf '%s\n' '{"request_id":"req-handshake","message":{"type":"handshake","component":"studio","component_version":"0.3.0","protocol":{"major":0,"minor":3}}}' \
  | nc -U "$HOME/Library/Application Support/Keyro/Core/keyro-core-dev.sock"
```

Expect a `handshake_accepted` response without stopping Studio. If nc stays open
after displaying the response, stop that probe with Ctrl+C. At most eight
concurrent development connections are allowed. Excess connections are closed.

## Record Acceptance Results

- Create a disposable profile; rename and activate it. Verify one active profile.
- Assign `https://example.com` to a control; trigger virtual input. Verify the
  browser opens and Studio receives running and succeeded events.
- Clear the assignment and verify readback. Do not expect an unassigned input
  to open a URL.
- Restart Core while Studio is open. Verify reconnection, then verify profiles
  and saved assignments survived. Repeat after restarting Studio.
- Verify the log does not contain URL queries, fragments or credentials.

Record Core/Studio commit IDs, OS, date, each result and any connection errors.
Actual Studio UI and OS browser execution remain manual acceptance checks until
an end-to-end harness covers them. The Rust socket tests use snapshot commands
and do not launch a browser.
