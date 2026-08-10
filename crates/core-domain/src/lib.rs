use std::fmt;
use url::Url;
use uuid::Uuid;

pub const PAGE_COUNT: u8 = 4;
pub const KEY_ROWS: u8 = 3;
pub const KEY_COLUMNS: u8 = 4;
pub const KEYS_PER_PAGE: u8 = KEY_ROWS * KEY_COLUMNS;
pub const ENCODERS_PER_PAGE: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub is_active: bool,
}

impl Profile {
    pub fn new(name: impl Into<String>, is_active: bool) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::EmptyProfileName);
        }

        Ok(Self {
            id: ProfileId::new(),
            name,
            is_active,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileId(Uuid);

impl ProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, DomainError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| DomainError::InvalidProfileId(value.to_owned()))
    }
}

impl Default for ProfileId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageIndex(u8);

impl PageIndex {
    pub fn new(value: u8) -> Result<Self, DomainError> {
        if value < PAGE_COUNT {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidPageIndex(value))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyIndex(u8);

impl KeyIndex {
    pub fn new(value: u8) -> Result<Self, DomainError> {
        if value < KEYS_PER_PAGE {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidKeyIndex(value))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncoderIndex(u8);

impl EncoderIndex {
    pub fn new(value: u8) -> Result<Self, DomainError> {
        if value < ENCODERS_PER_PAGE {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidEncoderIndex(value))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncoderOperation {
    Left,
    Right,
    Press,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Key {
        page: PageIndex,
        key: KeyIndex,
    },
    Encoder {
        page: PageIndex,
        encoder: EncoderIndex,
        operation: EncoderOperation,
    },
}

impl ControlId {
    pub fn key(page: u8, key: u8) -> Result<Self, DomainError> {
        Ok(Self::Key {
            page: PageIndex::new(page)?,
            key: KeyIndex::new(key)?,
        })
    }

    pub fn encoder(
        page: u8,
        encoder: u8,
        operation: EncoderOperation,
    ) -> Result<Self, DomainError> {
        Ok(Self::Encoder {
            page: PageIndex::new(page)?,
            encoder: EncoderIndex::new(encoder)?,
            operation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenUrl { url: SafeUrl },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeUrl(String);

impl SafeUrl {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let parsed = Url::parse(&value).map_err(|_| DomainError::InvalidUrl(value.clone()))?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DomainError::UrlCredentialsNotAllowed);
        }

        match parsed.scheme() {
            "http" | "https" => Ok(Self(value)),
            scheme => Err(DomainError::UnsupportedUrlScheme(scheme.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn redacted_for_log(&self) -> String {
        match Url::parse(&self.0) {
            Ok(mut url) => {
                url.set_query(None);
                url.set_fragment(None);
                url.to_string()
            }
            Err(_) => "<invalid-url>".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub profile_id: ProfileId,
    pub control: ControlId,
    pub actions: Vec<Action>,
}

impl Assignment {
    pub fn single(
        profile_id: ProfileId,
        control: ControlId,
        action: Action,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            profile_id,
            control,
            actions: vec![action],
        })
    }

    pub fn first_action(&self) -> Option<&Action> {
        self.actions.first()
    }
}

pub fn ensure_exactly_one_active(profiles: &[Profile]) -> Result<(), DomainError> {
    let active_count = profiles.iter().filter(|profile| profile.is_active).count();
    if active_count == 1 {
        Ok(())
    } else {
        Err(DomainError::ActiveProfileInvariant { active_count })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("profile name cannot be empty")]
    EmptyProfileName,
    #[error("invalid profile id: {0}")]
    InvalidProfileId(String),
    #[error("page index {0} is outside the supported range 0..4")]
    InvalidPageIndex(u8),
    #[error("key index {0} is outside the supported range 0..12")]
    InvalidKeyIndex(u8),
    #[error("encoder index {0} is outside the supported range 0..2")]
    InvalidEncoderIndex(u8),
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("unsupported url scheme: {0}")]
    UnsupportedUrlScheme(String),
    #[error("url credentials are not allowed")]
    UrlCredentialsNotAllowed,
    #[error("expected exactly one active profile, found {active_count}")]
    ActiveProfileInvariant { active_count: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_mvp_controls() {
        assert!(ControlId::key(3, 11).is_ok());
        assert!(ControlId::encoder(3, 1, EncoderOperation::Press).is_ok());
    }

    #[test]
    fn rejects_controls_outside_mvp_layout() {
        assert_eq!(ControlId::key(4, 0), Err(DomainError::InvalidPageIndex(4)));
        assert_eq!(ControlId::key(0, 12), Err(DomainError::InvalidKeyIndex(12)));
        assert_eq!(
            ControlId::encoder(0, 2, EncoderOperation::Left),
            Err(DomainError::InvalidEncoderIndex(2))
        );
    }

    #[test]
    fn only_http_and_https_urls_are_safe() {
        assert!(SafeUrl::parse("https://example.com/path?q=secret#token").is_ok());
        assert_eq!(
            SafeUrl::parse("file:///tmp/keyro").unwrap_err(),
            DomainError::UnsupportedUrlScheme("file".to_owned())
        );
        assert_eq!(
            SafeUrl::parse("javascript:alert(1)").unwrap_err(),
            DomainError::UnsupportedUrlScheme("javascript".to_owned())
        );
        assert_eq!(
            SafeUrl::parse("https://user:password@example.com").unwrap_err(),
            DomainError::UrlCredentialsNotAllowed
        );
    }

    #[test]
    fn redacts_url_query_and_fragment_for_logs() {
        let url = SafeUrl::parse("https://example.com/path?token=secret#fragment").unwrap();

        assert_eq!(url.redacted_for_log(), "https://example.com/path");
    }

    #[test]
    fn enforces_active_profile_invariant() {
        let active = Profile::new("Active", true).unwrap();
        let inactive = Profile::new("Inactive", false).unwrap();

        assert!(ensure_exactly_one_active(&[active.clone(), inactive]).is_ok());
        assert_eq!(
            ensure_exactly_one_active(&[active.clone(), active]),
            Err(DomainError::ActiveProfileInvariant { active_count: 2 })
        );
    }
}
