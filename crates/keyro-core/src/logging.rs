use anyhow::Context;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing_subscriber::fmt::MakeWriter;

const LOG_FILE_NAME: &str = "keyro-core.log";

#[derive(Debug, Clone, Copy)]
pub struct LogRetention {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_files: usize,
    pub max_age: Duration,
}

impl Default for LogRetention {
    fn default() -> Self {
        Self {
            max_file_bytes: 5 * 1024 * 1024,
            max_total_bytes: 50 * 1024 * 1024,
            max_files: 10,
            max_age: Duration::from_secs(14 * 24 * 60 * 60),
        }
    }
}

#[derive(Clone)]
pub struct RotatingLogWriter {
    state: Arc<Mutex<LogState>>,
}

impl RotatingLogWriter {
    pub fn new(log_dir: impl Into<PathBuf>, retention: LogRetention) -> io::Result<Self> {
        let log_dir = log_dir.into();
        fs::create_dir_all(&log_dir)?;
        cleanup_logs(&log_dir, retention)?;
        let file = open_current_log(&log_dir)?;
        Ok(Self {
            state: Arc::new(Mutex::new(LogState {
                log_dir,
                retention,
                file,
            })),
        })
    }
}

impl<'writer> MakeWriter<'writer> for RotatingLogWriter {
    type Writer = RotatingLogHandle;

    fn make_writer(&'writer self) -> Self::Writer {
        RotatingLogHandle {
            state: Arc::clone(&self.state),
        }
    }
}

pub struct RotatingLogHandle {
    state: Arc<Mutex<LogState>>,
}

impl Write for RotatingLogHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?;
        state.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?;
        state.file.flush()
    }
}

struct LogState {
    log_dir: PathBuf,
    retention: LogRetention,
    file: File,
}

impl LogState {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let current_size = self.file.metadata()?.len();
        if current_size > 0
            && current_size.saturating_add(buf.len() as u64) > self.retention.max_file_bytes
        {
            self.rotate()?;
        }
        let written = self.file.write(buf)?;
        cleanup_logs(&self.log_dir, self.retention)?;
        Ok(written)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        rotate_logs(&self.log_dir, self.retention)?;
        self.file = open_current_log(&self.log_dir)?;
        Ok(())
    }
}

fn open_current_log(log_dir: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(LOG_FILE_NAME))
}

fn rotate_logs(log_dir: &Path, retention: LogRetention) -> io::Result<()> {
    let max_rotated_files = retention.max_files.saturating_sub(1);
    if max_rotated_files == 0 {
        let current = log_dir.join(LOG_FILE_NAME);
        if current.exists() {
            fs::remove_file(current)?;
        }
        return Ok(());
    }

    let oldest = rotated_log_path(log_dir, max_rotated_files - 1);
    if oldest.exists() {
        fs::remove_file(oldest)?;
    }

    for index in (1..max_rotated_files).rev() {
        let source = rotated_log_path(log_dir, index - 1);
        let target = rotated_log_path(log_dir, index);
        if source.exists() {
            fs::rename(source, target)?;
        }
    }

    let current = log_dir.join(LOG_FILE_NAME);
    if current.exists() {
        fs::rename(current, rotated_log_path(log_dir, 0))?;
    }

    cleanup_logs(log_dir, retention)
}

fn rotated_log_path(log_dir: &Path, index: usize) -> PathBuf {
    log_dir.join(format!("keyro-core.{index}.log"))
}

fn cleanup_logs(log_dir: &Path, retention: LogRetention) -> io::Result<()> {
    let cutoff = SystemTime::now()
        .checked_sub(retention.max_age)
        .context("invalid log retention age")
        .map_err(io::Error::other)?;

    let mut retained = Vec::new();
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !is_keyro_log(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < cutoff {
            fs::remove_file(path)?;
        } else {
            retained.push(LogFile {
                is_current: path.file_name().and_then(|name| name.to_str()) == Some(LOG_FILE_NAME),
                path,
                modified,
                size: metadata.len(),
            });
        }
    }

    retained.sort_by(|left, right| right.modified.cmp(&left.modified));

    let current_index = retained.iter().position(|log_file| log_file.is_current);
    let mut total_bytes = current_index
        .and_then(|index| retained.get(index))
        .map_or(0, |log_file| log_file.size);
    let mut kept_files = usize::from(current_index.is_some());

    for (index, log_file) in retained.into_iter().enumerate() {
        if Some(index) == current_index {
            continue;
        }

        if kept_files >= retention.max_files
            || total_bytes.saturating_add(log_file.size) > retention.max_total_bytes
        {
            fs::remove_file(log_file.path)?;
        } else {
            kept_files += 1;
            total_bytes = total_bytes.saturating_add(log_file.size);
        }
    }

    Ok(())
}

struct LogFile {
    is_current: bool,
    path: PathBuf,
    modified: SystemTime,
    size: u64,
}

fn is_keyro_log(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == LOG_FILE_NAME || name.starts_with("keyro-core.") && name.ends_with(".log")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_when_current_file_exceeds_limit() {
        let temp = tempfile::tempdir().unwrap();
        let retention = LogRetention {
            max_file_bytes: 12,
            max_total_bytes: 36,
            max_files: 3,
            max_age: Duration::from_secs(60),
        };
        let writer = RotatingLogWriter::new(temp.path(), retention).unwrap();
        let mut handle = writer.make_writer();

        handle.write_all(b"hello world\n").unwrap();
        handle.write_all(b"second line\n").unwrap();
        handle.flush().unwrap();

        assert!(temp.path().join("keyro-core.log").exists());
        assert!(temp.path().join("keyro-core.0.log").exists());
    }

    #[test]
    fn keeps_configured_number_of_rotated_files() {
        let temp = tempfile::tempdir().unwrap();
        let retention = LogRetention {
            max_file_bytes: 4,
            max_total_bytes: 12,
            max_files: 2,
            max_age: Duration::from_secs(60),
        };
        let writer = RotatingLogWriter::new(temp.path(), retention).unwrap();
        let mut handle = writer.make_writer();

        for _ in 0..5 {
            handle.write_all(b"data\n").unwrap();
        }
        handle.flush().unwrap();

        assert!(temp.path().join("keyro-core.log").exists());
        assert!(temp.path().join("keyro-core.0.log").exists());
        assert!(!temp.path().join("keyro-core.1.log").exists());
    }

    #[test]
    fn enforces_total_log_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let retention = LogRetention {
            max_file_bytes: 10,
            max_total_bytes: 18,
            max_files: 10,
            max_age: Duration::from_secs(60),
        };
        let writer = RotatingLogWriter::new(temp.path(), retention).unwrap();
        let mut handle = writer.make_writer();

        for _ in 0..4 {
            handle.write_all(b"123456789\n").unwrap();
        }
        handle.flush().unwrap();

        let total_bytes = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum::<u64>();
        assert!(total_bytes <= retention.max_total_bytes);
    }

    #[test]
    fn startup_cleanup_enforces_count_and_total_bytes() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..5 {
            fs::write(
                temp.path().join(format!("keyro-core.{index}.log")),
                b"12345",
            )
            .unwrap();
        }
        fs::write(temp.path().join("keyro-core.log"), b"12345").unwrap();
        let retention = LogRetention {
            max_file_bytes: 10,
            max_total_bytes: 15,
            max_files: 3,
            max_age: Duration::from_secs(60),
        };

        let _writer = RotatingLogWriter::new(temp.path(), retention).unwrap();

        let logs = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                is_keyro_log(&path).then_some(path)
            })
            .collect::<Vec<_>>();
        let total_bytes = logs
            .iter()
            .map(|path| path.metadata().unwrap().len())
            .sum::<u64>();
        assert!(logs.len() <= retention.max_files);
        assert!(total_bytes <= retention.max_total_bytes);
        assert!(temp.path().join("keyro-core.log").exists());
    }
}
