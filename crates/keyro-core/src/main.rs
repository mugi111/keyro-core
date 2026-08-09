use anyhow::Context;
use keyro_core_app::{AppError, CoreEvent, CoreService, EventSink};
use keyro_core_ipc::{
    decode_protocol_dispatch, encode_protocol_message, error_message, response_from_core,
    ProtocolDispatch,
};
use keyro_core_platform::SystemUrlOpener;
use keyro_core_storage_sqlite::SqliteProfileRepository;
use logging::{LogRetention, RotatingLogWriter};
use single_instance::SingleInstanceGuard;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
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
    warn!("running development IPC listener; production framing, sessions, and backpressure are not implemented yet");
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
fn handle_dev_connection(
    stream: std::os::unix::net::UnixStream,
    repository: SqliteProfileRepository,
) {
    if let Err(error) = handle_dev_connection_inner(stream, repository) {
        warn!(%error, "development IPC connection failed");
    }
}

#[cfg(unix)]
fn handle_dev_connection_inner(
    mut stream: std::os::unix::net::UnixStream,
    repository: SqliteProfileRepository,
) -> anyhow::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let service = CoreService::new(repository, SystemUrlOpener, LoggingEventSink);

    for line in reader.lines() {
        let line = line?;
        let reply = match decode_protocol_dispatch(&line, env!("CARGO_PKG_VERSION")) {
            ProtocolDispatch::Immediate(message) => message,
            ProtocolDispatch::Command {
                request_id,
                command,
            } => match service.handle_command(command) {
                Ok(response) => response_from_core(request_id, response),
                Err(error) => error_message(Some(request_id), error),
            },
        };
        let encoded = encode_protocol_message(&reply)?;
        writeln!(stream, "{encoded}")?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn run_dev_ipc(_repository: SqliteProfileRepository, _socket_path: PathBuf) -> anyhow::Result<()> {
    warn!("development IPC listener is currently implemented for Unix domain sockets only");
    std::thread::park();
    Ok(())
}
