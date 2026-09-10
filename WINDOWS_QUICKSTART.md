# Windows minimum runtime

This development transport implements the existing newline-delimited JSON
protocol (0.3). No new protocol package publication is required.

## Build and start Core

Use Windows 11 x64 with Rust stable (MSVC), Visual Studio Build Tools with
Desktop development with C++, and the Windows SDK. Run Core and Studio as
the same ordinary user, without administrator elevation.

From the Core repository in PowerShell:

```powershell
cargo test --workspace --locked
cargo build --release --workspace --locked
.\target\release\keyro-core.exe
```

Keep that terminal running. Core uses:

- Pipe: `\\.\pipe\keyro-core-dev`
- Database: `%LOCALAPPDATA%\Keyro\Core\keyro-core.sqlite3`
- Log: `%LOCALAPPDATA%\Keyro\Core\logs\keyro-core.log`
- Lock: `%LOCALAPPDATA%\Keyro\Core\keyro-core.lock`

The pipe accepts only its owner and rejects remote clients. A second server
cannot take over an existing pipe name. The fixed development name supports
one Core account per machine at a time; per-user endpoint discovery is follow-up
work. There are at most eight connection workers. Idle/read/write deadlines,
message-size limits, and production framing remain follow-up work.

## Start Studio

In a second PowerShell terminal, from the Studio repository with its Windows
development prerequisites installed:

```powershell
bun install --frozen-lockfile
$env:KEYRO_STUDIO_CORE_MODE = 'local-ipc'
$env:KEYRO_STUDIO_CORE_SOCKET = '\\.\pipe\keyro-core-dev'
bun run dev
```

Studio already passes the configured endpoint to its local IPC adapter. No
Studio source changes are included in this branch; the packaged Windows UI
still requires acceptance testing.

## Probe without Studio

This probe can also run while Studio remains connected. It uses a five-second
deadline and closes the pipe afterwards.

```powershell
$pipe = [System.IO.Pipes.NamedPipeClientStream]::new('.', 'keyro-core-dev', [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous)
try {
    $pipe.Connect(5000)
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    $writer = [System.IO.StreamWriter]::new($pipe, $utf8, 1024, $true)
    $reader = [System.IO.StreamReader]::new($pipe, $utf8, $false, 1024, $true)
    $writer.AutoFlush = $true
    foreach ($message in @(
        '{"request_id":"hello","message":{"type":"handshake","component":"studio","component_version":"0.3.0","protocol":{"major":0,"minor":3}}}',
        '{"request_id":"snapshot","message":{"type":"get_snapshot"}}'
    )) {
        $writer.WriteLine($message)
        $reply = $reader.ReadLineAsync()
        if (-not $reply.Wait(5000)) { throw 'Core response timed out' }
        $reply.Result
    }
} finally {
    $pipe.Dispose()
}
```

Expect `handshake_accepted`, then `snapshot`. To view Core logs:

```powershell
Get-Content "$env:LOCALAPPDATA\Keyro\Core\logs\keyro-core.log" -Tail 50 -Wait
```

## Acceptance checklist

- Studio displays Connected and can create, rename, and activate profiles.
- Save and clear an assignment; use virtual input to open an HTTPS URL.
- Observe running and succeeded/failed action events.
- Close Studio, reconnect, and confirm Core remains available.
- Stop Core with Ctrl+C, restart it, and confirm saved settings persist.
- Confirm a second Core process fails without disturbing the first.

No physical device or Windows service is required. CI runs the native Windows
tests and produces an executable artifact. A passing CI run does not replace
the Studio UI and real default-browser checks above.

The Windows adapter follows the [CreateNamedPipeW contract](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew).
