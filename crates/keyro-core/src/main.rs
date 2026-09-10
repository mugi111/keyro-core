use anyhow::Context;
use keyro_core_app::{AppError, CoreEvent, CoreService, EventSink, UrlOpener};
use keyro_core_ipc::{
    encode_protocol_message, error_message, event_from_core, response_from_core, ProtocolDispatch,
    ProtocolSession,
};
use keyro_core_platform::SystemUrlOpener;
use keyro_core_protocol::ServerMessage;
use keyro_core_storage_sqlite::SqliteProfileRepository;
use logging::{LogRetention, RotatingLogWriter};
use single_instance::SingleInstanceGuard;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod logging;
mod single_instance;
#[cfg(windows)]
mod windows_ipc;

#[cfg(unix)]
type DevStream = UnixStream;
#[cfg(windows)]
type DevStream = std::fs::File;

fn main() -> anyhow::Result<()> {
    let data_dir = default_data_dir().context("failed to resolve Keyro data directory")?;
    std::fs::create_dir_all(&data_dir).context("failed to create Keyro data directory")?;
    let _single_instance_guard = SingleInstanceGuard::acquire(&data_dir)?;

    init_logging()?;

    let db_path = data_dir.join("keyro-core.sqlite3");

    let repository = SqliteProfileRepository::open(&db_path)
        .with_context(|| format!("failed to open SQLite database at {}", db_path.display()))?;
    let service = CoreService::new(repository.clone(), SystemUrlOpener, LoggingEventSink);
    let profile = service
        .initialize()
        .context("failed to initialize Core state")?;

    info!(
        profile_id = %profile.id,
        profile_name = %profile.name,
        "Keyro Core initialized"
    );
    warn!("running development IPC listener; production framing and backpressure are not implemented yet");
    run_dev_ipc(repository, data_dir.join("keyro-core-dev.sock"))?;

    Ok(())
}

fn init_logging() -> anyhow::Result<()> {
    let log_dir = default_log_dir().context("failed to resolve Keyro log directory")?;
    let log_writer = RotatingLogWriter::new(log_dir, LogRetention::default())
        .context("failed to create rotating log writer")?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(log_writer)
        .with_ansi(false)
        .init();

    Ok(())
}

fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/Keyro/Core"))
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Keyro").join("Core"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/share/keyro/core"))
    }
}

fn default_log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Logs/Keyro/Core"))
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Keyro").join("Core").join("logs"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/state/keyro/core/logs"))
    }
}

struct LoggingEventSink;

impl EventSink for LoggingEventSink {
    fn emit(&self, event: CoreEvent) -> Result<(), AppError> {
        info!(?event, "Core event");
        Ok(())
    }
}

#[cfg(unix)]
fn run_dev_ipc(repository: SqliteProfileRepository, socket_path: PathBuf) -> anyhow::Result<()> {
    use std::os::unix::net::UnixListener;

    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!(
                "failed to remove stale development IPC socket at {}",
                socket_path.display()
            )
        })?;
    }

    let listener = UnixListener::bind(&socket_path).with_context(|| {
        format!(
            "failed to bind development IPC socket at {}",
            socket_path.display()
        )
    })?;
    info!(path = %socket_path.display(), "development IPC listener started");

    let mut connections = DevConnections::default();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = connections.accept(stream, repository.clone()) {
                    warn!(%error, "failed to start development IPC connection worker");
                }
            }
            Err(error) => warn!(%error, "development IPC accept failed"),
        }
    }

    Ok(())
}

const MAX_DEV_CONNECTIONS: usize = 8;

#[derive(Default)]
struct DevConnections {
    workers: Vec<std::thread::JoinHandle<()>>,
    next_id: u64,
}

impl DevConnections {
    fn accept(
        &mut self,
        stream: DevStream,
        repository: SqliteProfileRepository,
    ) -> std::io::Result<()> {
        // Reap only finished workers so an idle client cannot block acceptance.
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].is_finished() {
                if self.workers.swap_remove(index).join().is_err() {
                    warn!("development IPC connection worker panicked");
                }
            } else {
                index += 1;
            }
        }
        if self.workers.len() >= MAX_DEV_CONNECTIONS {
            warn!(
                limit = MAX_DEV_CONNECTIONS,
                "development IPC connection limit reached; closing new connection"
            );
            return Ok(());
        }

        let connection_id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let span = tracing::info_span!("dev_ipc_connection", connection_id);
        let worker = std::thread::Builder::new()
            .name("keyro-dev-ipc".to_owned())
            .spawn(move || {
                let _entered = span.enter();
                info!("development IPC connection accepted");
                handle_dev_connection(stream, repository);
                info!("development IPC connection closed");
            })?;
        self.workers.push(worker);
        Ok(())
    }
}

fn handle_dev_connection(stream: DevStream, repository: SqliteProfileRepository) {
    if let Err(error) = handle_dev_connection_inner(stream, repository, SystemUrlOpener) {
        warn!(%error, "development IPC connection failed");
    }
}

fn handle_dev_connection_inner<O>(
    stream: DevStream,
    repository: SqliteProfileRepository,
    url_opener: O,
) -> anyhow::Result<()>
where
    O: UrlOpener,
{
    let reader = BufReader::new(stream.try_clone()?);
    let writer = Arc::new(Mutex::new(stream));
    let event_sink = IpcEventSink::new(Arc::clone(&writer));
    let service = CoreService::new(repository, url_opener, event_sink);
    let mut session = ProtocolSession::new(env!("CARGO_PKG_VERSION"));

    for line in reader.lines() {
        let line = line?;
        let reply = match session.receive_line(&line) {
            ProtocolDispatch::Immediate(message) => message,
            ProtocolDispatch::Command {
                request_id,
                command,
            } => match service.handle_command(command) {
                Ok(response) => response_from_core(request_id, response),
                Err(error) => {
                    warn!(%request_id, %error, "Core command failed");
                    error_message(Some(request_id), &error)
                }
            },
        };
        write_protocol_message(&writer, &reply)?;
    }

    Ok(())
}

#[derive(Clone)]
struct IpcEventSink {
    writer: Arc<Mutex<DevStream>>,
}

impl IpcEventSink {
    fn new(writer: Arc<Mutex<DevStream>>) -> Self {
        Self { writer }
    }
}

impl EventSink for IpcEventSink {
    fn emit(&self, event: CoreEvent) -> Result<(), AppError> {
        info!(?event, "Core event");
        let message = event_from_core(event);
        write_protocol_message(&self.writer, &message).map_err(|error| {
            AppError::EventSink(format!(
                "failed to write action event to IPC client: {error}"
            ))
        })
    }
}

fn write_protocol_message(
    writer: &Arc<Mutex<DevStream>>,
    message: &ServerMessage,
) -> anyhow::Result<()> {
    let encoded = encode_protocol_message(message)?;
    let mut writer = writer
        .lock()
        .map_err(|_| anyhow::anyhow!("IPC writer lock poisoned"))?;
    writeln!(writer, "{encoded}")?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use keyro_core_app::{ActionRunId, OpenUrlError, ProfileRepository};
    use keyro_core_domain::{Action, Assignment, ControlId, ProfileId, SafeUrl};
    use keyro_core_protocol::{ActionEventDto, ControlDto};
    use std::net::Shutdown;
    use std::sync::Arc;
    use std::thread;

    #[derive(Clone, Default)]
    struct RecordingOpener {
        opened: Arc<Mutex<Vec<String>>>,
        error: Arc<Mutex<Option<String>>>,
    }

    impl UrlOpener for RecordingOpener {
        fn open_url(&self, url: &SafeUrl) -> Result<(), OpenUrlError> {
            self.opened.lock().unwrap().push(url.as_str().to_owned());
            if let Some(message) = self.error.lock().unwrap().clone() {
                Err(OpenUrlError::new(message))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn ipc_event_sink_writes_action_events_to_connection() {
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer = Arc::new(Mutex::new(writer));
        let sink = IpcEventSink::new(writer);
        let control = ControlId::key(0, 0).unwrap();
        let profile_id = ProfileId::new();

        sink.emit(CoreEvent::ActionRunning {
            run_id: ActionRunId::default(),
            profile_id,
            control,
        })
        .unwrap();

        let mut line = String::new();
        BufReader::new(reader).read_line(&mut line).unwrap();
        let message: ServerMessage = serde_json::from_str(line.trim_end()).unwrap();

        let ServerMessage::ActionEvent {
            event:
                ActionEventDto::Running {
                    profile_id: encoded_profile_id,
                    ..
                },
        } = message
        else {
            panic!("expected running action event");
        };
        assert_eq!(encoded_profile_id, profile_id.to_string());
    }

    #[test]
    fn dev_connection_delivers_action_events_before_acknowledgement() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        let assignment = Assignment::single(
            profile.id,
            control,
            Action::OpenUrl {
                url: SafeUrl::parse("https://example.com").unwrap(),
            },
        )
        .unwrap();
        repository.save_assignment(&assignment).unwrap();

        let opener = RecordingOpener::default();
        let opened = Arc::clone(&opener.opened);
        let (mut client, server) = UnixStream::pair().unwrap();
        let handler =
            thread::spawn(move || handle_dev_connection_inner(server, repository, opener));

        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.3.0/handshake.json"
            ))
        )
        .unwrap();
        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.2.0/virtual-control-input.json"
            ))
        )
        .unwrap();
        client.shutdown(Shutdown::Write).unwrap();

        let mut reader = BufReader::new(client);
        let handshake = read_server_message(&mut reader);
        let running = read_server_message(&mut reader);
        let succeeded = read_server_message(&mut reader);
        let acknowledged = read_server_message(&mut reader);

        assert!(matches!(handshake, ServerMessage::HandshakeAccepted { .. }));
        let ServerMessage::ActionEvent {
            event:
                ActionEventDto::Running {
                    control: running_control,
                    ..
                },
        } = running
        else {
            panic!("expected running event");
        };
        assert_eq!(running_control, ControlDto::Key { page: 0, key: 0 });
        assert!(matches!(
            succeeded,
            ServerMessage::ActionEvent {
                event: ActionEventDto::Succeeded { .. }
            }
        ));
        assert!(matches!(
            acknowledged,
            ServerMessage::Acknowledged {
                request_id
            } if request_id == "req-virtual-key-press"
        ));
        assert_eq!(opened.lock().unwrap().as_slice(), ["https://example.com"]);
        handler.join().unwrap().unwrap();
    }

    #[test]
    fn dev_connection_returns_snapshot_with_persisted_assignment() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        repository
            .save_assignment(
                &Assignment::single(
                    profile.id,
                    control,
                    Action::OpenUrl {
                        url: SafeUrl::parse("https://example.com").unwrap(),
                    },
                )
                .unwrap(),
            )
            .unwrap();

        let (mut client, server) = UnixStream::pair().unwrap();
        let handler = thread::spawn(move || {
            handle_dev_connection_inner(server, repository, RecordingOpener::default())
        });

        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.3.0/handshake.json"
            ))
        )
        .unwrap();
        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.2.0/get-snapshot.json"
            ))
        )
        .unwrap();
        client.shutdown(Shutdown::Write).unwrap();

        let mut reader = BufReader::new(client);
        let handshake = read_server_message(&mut reader);
        let snapshot = read_server_message(&mut reader);

        assert!(matches!(handshake, ServerMessage::HandshakeAccepted { .. }));
        let ServerMessage::Snapshot {
            request_id,
            layout,
            profiles,
            assignments,
        } = snapshot
        else {
            panic!("expected snapshot response");
        };
        assert_eq!(request_id, "req-snapshot");
        assert_eq!(layout.page_count, 4);
        assert_eq!(layout.key_rows, 3);
        assert_eq!(layout.key_columns, 4);
        assert_eq!(layout.encoder_count, 2);
        assert_eq!(profiles[0].id, profile.id.to_string());
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].profile_id, profile.id.to_string());
        assert_eq!(assignments[0].control, ControlDto::Key { page: 0, key: 0 });
        assert_eq!(assignments[0].actions.len(), 1);

        handler.join().unwrap().unwrap();
    }

    #[test]
    fn dev_connection_delivers_failed_action_event_before_error() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        let assignment = Assignment::single(
            profile.id,
            control,
            Action::OpenUrl {
                url: SafeUrl::parse("https://example.com").unwrap(),
            },
        )
        .unwrap();
        repository.save_assignment(&assignment).unwrap();

        let opener = RecordingOpener::default();
        *opener.error.lock().unwrap() = Some("open failed".to_owned());
        let (mut client, server) = UnixStream::pair().unwrap();
        let handler =
            thread::spawn(move || handle_dev_connection_inner(server, repository, opener));

        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.3.0/handshake.json"
            ))
        )
        .unwrap();
        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.2.0/virtual-control-input.json"
            ))
        )
        .unwrap();
        client.shutdown(Shutdown::Write).unwrap();

        let mut reader = BufReader::new(client);
        let handshake = read_server_message(&mut reader);
        let running = read_server_message(&mut reader);
        let failed = read_server_message(&mut reader);
        let request_error = read_server_message(&mut reader);

        assert!(matches!(handshake, ServerMessage::HandshakeAccepted { .. }));
        assert!(matches!(
            running,
            ServerMessage::ActionEvent {
                event: ActionEventDto::Running { .. }
            }
        ));
        assert!(matches!(
            failed,
            ServerMessage::ActionEvent {
                event:
                    ActionEventDto::Failed {
                        message,
                        ..
                    }
            } if message == "open failed"
        ));
        assert!(matches!(
            request_error,
            ServerMessage::Error {
                request_id,
                ..
            } if request_id.as_deref() == Some("req-virtual-key-press")
        ));
        handler.join().unwrap().unwrap();
    }

    fn read_server_message(reader: &mut BufReader<UnixStream>) -> ServerMessage {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(line.trim_end()).unwrap()
    }

    fn compact_json(input: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(input).unwrap();
        serde_json::to_string(&value).unwrap()
    }

    #[test]
    fn dev_connections_serve_snapshot_while_another_client_remains_connected() {
        use std::os::unix::net::UnixListener;
        use std::time::Duration;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let mut connections = DevConnections::default();
        let mut clients = Vec::new();

        for _ in 0..2 {
            let mut client = UnixStream::connect(&path).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let (server, _) = listener.accept().unwrap();
            connections.accept(server, repository.clone()).unwrap();
            writeln!(
                client,
                "{}",
                compact_json(include_str!(
                    "../../../protocol/test-vectors/v0.3.0/handshake.json"
                ))
            )
            .unwrap();
            let mut reader = BufReader::new(client);
            assert!(matches!(
                read_server_message(&mut reader),
                ServerMessage::HandshakeAccepted { .. }
            ));
            clients.push(reader);
        }

        // Both established clients must remain usable, not just the newest one.
        for reader in &mut clients {
            writeln!(
                reader.get_mut(),
                "{}",
                compact_json(include_str!(
                    "../../../protocol/test-vectors/v0.3.0/get-snapshot.json"
                ))
            )
            .unwrap();
            let ServerMessage::Snapshot { profiles, .. } = read_server_message(reader) else {
                panic!("expected snapshot");
            };
            assert_eq!(profiles[0].id, profile.id.to_string());
        }
        drop(clients);
        for worker in connections.workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn dev_connections_bound_idle_clients_and_reclaim_disconnected_workers() {
        use std::io::Read;
        use std::time::{Duration, Instant};

        let repository = SqliteProfileRepository::in_memory().unwrap();
        let mut connections = DevConnections::default();
        let mut clients = Vec::new();
        for _ in 0..MAX_DEV_CONNECTIONS {
            let (client, server) = UnixStream::pair().unwrap();
            connections.accept(server, repository.clone()).unwrap();
            clients.push(client);
        }
        let (mut rejected, server) = UnixStream::pair().unwrap();
        rejected
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        connections.accept(server, repository.clone()).unwrap();
        assert_eq!(rejected.read(&mut [0]).unwrap(), 0);
        assert_eq!(connections.workers.len(), MAX_DEV_CONNECTIONS);

        drop(clients);
        let deadline = Instant::now() + Duration::from_secs(2);
        while connections
            .workers
            .iter()
            .any(|worker| !worker.is_finished())
        {
            assert!(
                Instant::now() < deadline,
                "disconnected workers did not exit"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        connections.accept(server, repository).unwrap();
        assert_eq!(connections.workers.len(), 1);
        writeln!(
            client,
            "{}",
            compact_json(include_str!(
                "../../../protocol/test-vectors/v0.3.0/handshake.json"
            ))
        )
        .unwrap();
        let mut reader = BufReader::new(client);
        assert!(matches!(
            read_server_message(&mut reader),
            ServerMessage::HandshakeAccepted { .. }
        ));
        drop(reader);
        for worker in connections.workers {
            worker.join().unwrap();
        }
    }
}

#[cfg(windows)]
fn run_dev_ipc(repository: SqliteProfileRepository, _socket_path: PathBuf) -> anyhow::Result<()> {
    windows_ipc::run(repository)
}
