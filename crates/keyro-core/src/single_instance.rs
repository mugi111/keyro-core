use anyhow::{bail, Context};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::Write;
use std::path::Path;

pub struct SingleInstanceGuard {
    file: File,
}

impl SingleInstanceGuard {
    pub fn acquire(data_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "failed to create data directory before acquiring lock at {}",
                data_dir.display()
            )
        })?;
        let lock_path = data_dir.join("keyro-core.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open lock file at {}", lock_path.display()))?;

        if let Err(error) = file.try_lock_exclusive() {
            if error.kind() == io::ErrorKind::WouldBlock {
                bail!(
                    "another Keyro Core instance is already running; lock file is {}",
                    lock_path.display()
                );
            }

            return Err(error).with_context(|| {
                format!(
                    "failed to acquire exclusive Keyro Core lock at {}",
                    lock_path.display()
                )
            });
        }

        file.set_len(0)
            .context("failed to truncate Keyro Core lock file")?;
        file.write_all(std::process::id().to_string().as_bytes())
            .context("failed to write Keyro Core lock owner pid")?;

        Ok(Self { file })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prevents_second_guard_for_same_directory() {
        let temp = tempfile::tempdir().unwrap();
        let first = SingleInstanceGuard::acquire(temp.path()).unwrap();
        let second = SingleInstanceGuard::acquire(temp.path());

        assert!(second.is_err());
        drop(first);
        assert!(temp.path().join("keyro-core.lock").exists());
        assert!(SingleInstanceGuard::acquire(temp.path()).is_ok());
    }
}
