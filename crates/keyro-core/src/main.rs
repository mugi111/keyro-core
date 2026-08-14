use anyhow::Context;
use keyro_core_app::{AppError, CoreEvent, CoreService, EventSink, UrlOpener};
use keyro_core_ipc::{
    encode_protocol_message, error_message, event_from_core, response_from_core, ProtocolDispatch,
    ProtocolSession,
};
use keyro_core_platform::SystemUrlOpener;
#[cfg(unix)]
use keyro_core_protocol::ServerMessage;
use keyro_core_storage_sqlite::SqliteProfileRepository;
use logging::{LogRetention, RotatingLogWriter};
use single_instance::SingleInstanceGuard;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod logging;
mod single_instance;

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

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_dev_connection(stream, repository.clone()),
            Err(error) => warn!(%error, "development IPC accept failed"),
        }
    }

    Ok(())
}

#[cfg(unix)]
fn handle_dev_connection(stream: UnixStream, repository: SqliteProfileRepository) {
    if let Err(error) = handle_dev_connection_inner(stream, repository, SystemUrlOpener) {
        warn!(%error, "development IPC connection failed");
    }
}

#[cfg(unix)]
fn handle_dev_connection_inner<O>(
    stream: UnixStream,
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

#[cfg(unix)]
#[derive(Clone)]
struct IpcEventSink {
    writer: Arc<Mutex<UnixStream>>,
}

#[cfg(unix)]
impl IpcEventSink {
    fn new(writer: Arc<Mutex<UnixStream>>) -> Self {
        Self { writer }
    }
}

#[cfg(unix)]
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

#[cfg(unix)]
fn write_protocol_message(
    writer: &Arc<Mutex<UnixStream>>,
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
}

#[cfg(not(unix))]
fn run_dev_ipc(_repository: SqliteProfileRepository, _socket_path: PathBuf) -> anyhow::Result<()> {
    warn!("development IPC listener is currently implemented for Unix domain sockets only");
    std::thread::park();
    Ok(())
}
