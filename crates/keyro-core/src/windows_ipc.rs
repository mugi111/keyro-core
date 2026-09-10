use super::{DevConnections, SqliteProfileRepository, MAX_DEV_CONNECTIONS};
use anyhow::Context;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::null_mut;
use tracing::{info, warn};
use windows_sys::Win32::Foundation::{LocalFree, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

pub const PIPE_PATH: &str = r"\\.\pipe\keyro-core-dev";

pub fn run(repository: SqliteProfileRepository) -> anyhow::Result<()> {
    let mut pending =
        create_pipe(PIPE_PATH, true).context("failed to create development IPC pipe")?;
    info!(path = PIPE_PATH, "development IPC listener started");
    let mut connections = DevConnections::default();
    loop {
        let connected = connect_pipe(&pending);
        // Keep an instance alive while replacing it, including after client disconnects.
        let next = create_pipe(PIPE_PATH, false).context("failed to renew development IPC pipe")?;
        let stream = std::mem::replace(&mut pending, next);
        match connected {
            Ok(()) => {
                if let Err(error) = connections.accept(stream, repository.clone()) {
                    warn!(%error, "failed to start development IPC connection worker");
                }
            }
            Err(error) => warn!(%error, "development IPC accept failed"),
        }
    }
}

fn create_pipe(path: &str, first: bool) -> io::Result<File> {
    let name: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    // Only the object owner receives access; remote clients are rejected separately.
    // Core and Studio should run unelevated under the same Windows account.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain(Some(0)).collect();
    let mut descriptor = null_mut();
    // SAFETY: Both strings are NUL terminated. The API allocates descriptor.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let flags = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: All pointers remain valid for the call; the returned handle is owned.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            (MAX_DEV_CONNECTIONS + 2) as u32,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: The conversion API allocated this buffer with LocalAlloc.
    unsafe {
        LocalFree(descriptor);
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(error);
    }
    // SAFETY: A successful CreateNamedPipeW returned a unique, synchronous handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn connect_pipe(pipe: &File) -> io::Result<()> {
    // SAFETY: File owns a live synchronous pipe; no OVERLAPPED structure is needed.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), null_mut()) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle_dev_connection_inner;
    use keyro_core_app::{OpenUrlError, ProfileRepository, UrlOpener};
    use keyro_core_domain::{Action, Assignment, ControlId, SafeUrl};
    use keyro_core_protocol::{ActionEventDto, ServerMessage};
    use std::fs::OpenOptions;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    struct TestOpener;

    impl UrlOpener for TestOpener {
        fn open_url(&self, url: &SafeUrl) -> Result<(), OpenUrlError> {
            assert_eq!(url.as_str(), "https://example.com/");
            Ok(())
        }
    }

    #[test]
    fn pipe_rejects_second_first_instance() {
        let path = format!(r"\\.\pipe\keyro-test-exclusive-{}", std::process::id());
        let first = create_pipe(&path, true).unwrap();
        assert!(create_pipe(&path, true).is_err());
        drop(first);
        assert!(create_pipe(&path, true).is_ok());
    }

    #[test]
    fn pipe_serves_concurrent_clients_and_reconnects() {
        let path = format!(r"\\.\pipe\keyro-test-session-{}", std::process::id());
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        repository
            .save_assignment(
                &Assignment::single(
                    profile.id,
                    ControlId::key(0, 0).unwrap(),
                    Action::OpenUrl {
                        url: SafeUrl::parse("https://example.com/").unwrap(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let server_path = path.clone();
        let server = std::thread::spawn(move || {
            let mut pending = create_pipe(&server_path, true).unwrap();
            let mut workers = Vec::new();
            for _ in 0..3 {
                ready_tx.send(()).unwrap();
                connect_pipe(&pending).unwrap();
                let next = create_pipe(&server_path, false).unwrap();
                let stream = std::mem::replace(&mut pending, next);
                let repository = repository.clone();
                workers.push(std::thread::spawn(move || {
                    // A disconnected byte pipe reports BrokenPipe on Windows.
                    let result = handle_dev_connection_inner(stream, repository, TestOpener);
                    if let Err(error) = result {
                        assert_eq!(
                            error.downcast_ref::<io::Error>().unwrap().kind(),
                            io::ErrorKind::BrokenPipe
                        );
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        let (done_tx, done_rx) = mpsc::channel();
        let client = std::thread::spawn(move || {
            let mut clients = Vec::new();
            for index in 0..3 {
                ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut pipe = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .unwrap();
                for vector in [
                    include_str!("../../../protocol/test-vectors/v0.3.0/handshake.json"),
                    include_str!("../../../protocol/test-vectors/v0.3.0/get-snapshot.json"),
                ] {
                    let value: serde_json::Value = serde_json::from_str(vector).unwrap();
                    writeln!(pipe, "{value}").unwrap();
                }
                let mut reader = BufReader::new(pipe);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(matches!(
                    serde_json::from_str::<ServerMessage>(&line).unwrap(),
                    ServerMessage::HandshakeAccepted { .. }
                ));
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(matches!(
                    serde_json::from_str::<ServerMessage>(&line).unwrap(),
                    ServerMessage::Snapshot { .. }
                ));
                let input: serde_json::Value = serde_json::from_str(include_str!(
                    "../../../protocol/test-vectors/v0.2.0/virtual-control-input.json"
                ))
                .unwrap();
                writeln!(reader.get_mut(), "{input}").unwrap();
                let mut replies = Vec::new();
                for _ in 0..3 {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    replies.push(serde_json::from_str::<ServerMessage>(&line).unwrap());
                }
                assert!(matches!(
                    &replies[0],
                    ServerMessage::ActionEvent {
                        event: ActionEventDto::Running { .. }
                    }
                ));
                assert!(matches!(
                    &replies[1],
                    ServerMessage::ActionEvent {
                        event: ActionEventDto::Succeeded { .. }
                    }
                ));
                assert!(matches!(&replies[2], ServerMessage::Acknowledged { .. }));
                clients.push(reader);
                if index == 1 {
                    clients.clear();
                }
            }
            drop(clients);
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(15))
            .expect("named-pipe exchange timed out");
        client.join().unwrap();
        server.join().unwrap();
    }
}
