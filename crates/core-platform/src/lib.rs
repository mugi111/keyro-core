use keyro_core_app::{OpenUrlError, UrlOpener};
use keyro_core_domain::SafeUrl;
use std::process::Command;

#[derive(Debug, Default)]
pub struct SystemUrlOpener;

impl UrlOpener for SystemUrlOpener {
    fn open_url(&self, url: &SafeUrl) -> Result<(), OpenUrlError> {
        open_url_with_system(url).map_err(|error| OpenUrlError::new(error.to_string()))
    }
}

#[cfg(target_os = "macos")]
fn open_url_with_system(url: &SafeUrl) -> Result<(), PlatformError> {
    let status = Command::new("/usr/bin/open")
        .arg(url.as_str())
        .status()
        .map_err(PlatformError::LaunchFailed)?;

    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::LaunchExited {
            code: status.code(),
            url: url.redacted_for_log(),
        })
    }
}

#[cfg(target_os = "windows")]
fn open_url_with_system(url: &SafeUrl) -> Result<(), PlatformError> {
    let status = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url.as_str()])
        .status()
        .map_err(PlatformError::LaunchFailed)?;

    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::LaunchExited {
            code: status.code(),
            url: url.redacted_for_log(),
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_url_with_system(url: &SafeUrl) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedOs {
        url: url.redacted_for_log(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("failed to launch URL opener: {0}")]
    LaunchFailed(#[source] std::io::Error),
    #[error("URL opener exited with code {code:?} for {url}")]
    LaunchExited { code: Option<i32>, url: String },
    #[error("URL opener is not supported on this OS for {url}")]
    UnsupportedOs { url: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_opener_accepts_safe_url_type() {
        let url = SafeUrl::parse("https://example.com").unwrap();
        let redacted = url.redacted_for_log();

        assert_eq!(redacted, "https://example.com/");
    }
}
