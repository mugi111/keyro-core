use keyro_core_app::{AppError, ProfileRepository, SnapshotState};
use keyro_core_domain::{
    Action, Assignment, ControlId, EncoderOperation, Profile, ProfileId, SafeUrl,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SqliteProfileRepository {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteProfileRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteStorageError> {
        let connection = Connection::open(path)?;
        let repository = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        repository.migrate()?;
        Ok(repository)
    }

    pub fn in_memory() -> Result<Self, SqliteStorageError> {
        let connection = Connection::open_in_memory()?;
        let repository = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        repository.migrate()?;
        Ok(repository)
    }

    fn migrate(&self) -> Result<(), SqliteStorageError> {
        let connection = self.connection.lock().unwrap();
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_migrations (
              version INTEGER PRIMARY KEY,
              applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS profiles (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              is_active INTEGER NOT NULL CHECK (is_active IN (0, 1)),
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE UNIQUE INDEX IF NOT EXISTS only_one_active_profile
              ON profiles(is_active)
              WHERE is_active = 1;

            CREATE TABLE IF NOT EXISTS assignments (
              profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
              page_index INTEGER NOT NULL,
              control_kind TEXT NOT NULL,
              control_index INTEGER NOT NULL,
              control_operation TEXT NOT NULL,
              action_position INTEGER NOT NULL DEFAULT 0,
              action_kind TEXT NOT NULL,
              action_url TEXT,
              PRIMARY KEY (
                profile_id,
                page_index,
                control_kind,
                control_index,
                control_operation,
                action_position
              )
            );

            INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
            ",
        )?;
        Ok(())
    }
}

impl ProfileRepository for SqliteProfileRepository {
    fn ensure_default_profile(&self) -> Result<Profile, AppError> {
        let connection = self.connection.lock().unwrap();
        let existing = load_active_profile(&connection).map_err(to_app_error)?;
        if let Some(profile) = existing {
            return Ok(profile);
        }

        let profile = Profile::new("Default", true)?;
        connection
            .execute(
                "INSERT INTO profiles(id, name, is_active) VALUES (?1, ?2, 1)",
                params![profile.id.to_string(), profile.name],
            )
            .map_err(to_app_error)?;
        Ok(profile)
    }

    fn list_profiles(&self) -> Result<Vec<Profile>, AppError> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT id, name, is_active FROM profiles ORDER BY created_at, name")
            .map_err(to_app_error)?;
        let rows = statement
            .query_map([], |row| {
                let id: String = row.get(0)?;
                Ok(Profile {
                    id: ProfileId::parse(&id).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    name: row.get(1)?,
                    is_active: row.get::<_, i64>(2)? == 1,
                })
            })
            .map_err(to_app_error)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(to_app_error)
    }

    fn set_active_profile(&self, profile_id: ProfileId) -> Result<(), AppError> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction().map_err(to_app_error)?;
        let updated = transaction
            .execute("UPDATE profiles SET is_active = 0 WHERE is_active = 1", [])
            .map_err(to_app_error)?;
        if updated == 0 {
            return Err(AppError::ActiveProfileMissing);
        }

        let updated = transaction
            .execute(
                "UPDATE profiles SET is_active = 1 WHERE id = ?1",
                params![profile_id.to_string()],
            )
            .map_err(to_app_error)?;
        if updated != 1 {
            return Err(AppError::Storage(format!(
                "profile {profile_id} does not exist"
            )));
        }

        transaction.commit().map_err(to_app_error)
    }

    fn save_assignment(&self, assignment: &Assignment) -> Result<(), AppError> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction().map_err(to_app_error)?;
        transaction
            .execute(
                "DELETE FROM assignments
                 WHERE profile_id = ?1
                   AND page_index = ?2
                   AND control_kind = ?3
                   AND control_index = ?4
                   AND control_operation = ?5",
                assignment_identity_params(assignment),
            )
            .map_err(to_app_error)?;

        for (position, action) in assignment.actions.iter().enumerate() {
            let (kind, url) = encode_action(action);
            let (page, control_kind, control_index, operation) = encode_control(assignment.control);
            transaction
                .execute(
                    "INSERT INTO assignments(
                       profile_id,
                       page_index,
                       control_kind,
                       control_index,
                       control_operation,
                       action_position,
                       action_kind,
                       action_url
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        assignment.profile_id.to_string(),
                        page,
                        control_kind,
                        control_index,
                        operation,
                        position as i64,
                        kind,
                        url
                    ],
                )
                .map_err(to_app_error)?;
        }

        transaction.commit().map_err(to_app_error)
    }

    fn load_snapshot_state(&self) -> Result<SnapshotState, AppError> {
        let connection = self.connection.lock().unwrap();
        let profiles = list_profiles_locked(&connection)?;
        let assignments = list_assignments_locked(&connection)?;
        Ok(SnapshotState {
            profiles,
            assignments,
        })
    }

    fn find_assignment(
        &self,
        profile_id: ProfileId,
        control: ControlId,
    ) -> Result<Option<Assignment>, AppError> {
        let connection = self.connection.lock().unwrap();
        let (page, control_kind, control_index, operation) = encode_control(control);
        let mut statement = connection
            .prepare(
                "SELECT action_kind, action_url
                 FROM assignments
                 WHERE profile_id = ?1
                   AND page_index = ?2
                   AND control_kind = ?3
                   AND control_index = ?4
                   AND control_operation = ?5
                 ORDER BY action_position",
            )
            .map_err(to_app_error)?;
        let rows = statement
            .query_map(
                params![
                    profile_id.to_string(),
                    page,
                    control_kind,
                    control_index,
                    operation
                ],
                |row| decode_action(row.get(0)?, row.get(1)?),
            )
            .map_err(to_app_error)?;
        let actions = rows.collect::<Result<Vec<_>, _>>().map_err(to_app_error)?;

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Assignment {
                profile_id,
                control,
                actions,
            }))
        }
    }
}

fn list_profiles_locked(connection: &Connection) -> Result<Vec<Profile>, AppError> {
    let mut statement = connection
        .prepare("SELECT id, name, is_active FROM profiles ORDER BY created_at, name")
        .map_err(to_app_error)?;
    let rows = statement
        .query_map([], |row| {
            let id: String = row.get(0)?;
            Ok(Profile {
                id: ProfileId::parse(&id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                name: row.get(1)?,
                is_active: row.get::<_, i64>(2)? == 1,
            })
        })
        .map_err(to_app_error)?;

    rows.collect::<Result<Vec<_>, _>>().map_err(to_app_error)
}

fn list_assignments_locked(connection: &Connection) -> Result<Vec<Assignment>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT profile_id,
                        page_index,
                        control_kind,
                        control_index,
                        control_operation,
                        action_kind,
                        action_url
                 FROM assignments
                 ORDER BY profile_id,
                          page_index,
                          control_kind,
                          control_index,
                          control_operation,
                          action_position",
        )
        .map_err(to_app_error)?;
    let rows = statement
        .query_map([], |row| {
            let profile_id: String = row.get(0)?;
            let page: i64 = row.get(1)?;
            let control_kind: String = row.get(2)?;
            let control_index: i64 = row.get(3)?;
            let control_operation: String = row.get(4)?;
            let action_kind: String = row.get(5)?;
            let action_url: Option<String> = row.get(6)?;
            Ok((
                ProfileId::parse(&profile_id).map_err(domain_to_sql_read_error)?,
                decode_control(page, control_kind, control_index, control_operation)?,
                decode_action(action_kind, action_url)?,
            ))
        })
        .map_err(to_app_error)?;

    let mut assignments: Vec<Assignment> = Vec::new();
    for row in rows {
        let (profile_id, control, action) = row.map_err(to_app_error)?;
        if let Some(existing) = assignments
            .iter_mut()
            .find(|assignment| assignment.profile_id == profile_id && assignment.control == control)
        {
            existing.actions.push(action);
        } else {
            assignments.push(Assignment {
                profile_id,
                control,
                actions: vec![action],
            });
        }
    }

    Ok(assignments)
}

fn load_active_profile(connection: &Connection) -> Result<Option<Profile>, SqliteStorageError> {
    connection
        .query_row(
            "SELECT id, name, is_active FROM profiles WHERE is_active = 1",
            [],
            |row| {
                let id: String = row.get(0)?;
                Ok(Profile {
                    id: ProfileId::parse(&id).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    name: row.get(1)?,
                    is_active: row.get::<_, i64>(2)? == 1,
                })
            },
        )
        .optional()
        .map_err(SqliteStorageError::Sqlite)
}

fn assignment_identity_params(assignment: &Assignment) -> [rusqlite::types::Value; 5] {
    let (page, control_kind, control_index, operation) = encode_control(assignment.control);
    [
        assignment.profile_id.to_string().into(),
        page.into(),
        control_kind.to_owned().into(),
        control_index.into(),
        operation.to_owned().into(),
    ]
}

fn encode_control(control: ControlId) -> (i64, &'static str, i64, &'static str) {
    match control {
        ControlId::Key { page, key } => (page.get() as i64, "key", key.get() as i64, "press"),
        ControlId::Encoder {
            page,
            encoder,
            operation,
        } => (
            page.get() as i64,
            "encoder",
            encoder.get() as i64,
            encode_encoder_operation(operation),
        ),
    }
}

fn encode_encoder_operation(operation: EncoderOperation) -> &'static str {
    match operation {
        EncoderOperation::Left => "left",
        EncoderOperation::Right => "right",
        EncoderOperation::Press => "press",
    }
}

fn encode_action(action: &Action) -> (&'static str, Option<&str>) {
    match action {
        Action::OpenUrl { url } => ("open_url", Some(url.as_str())),
    }
}

fn decode_action(kind: String, url: Option<String>) -> Result<Action, rusqlite::Error> {
    match kind.as_str() {
        "open_url" => {
            let url = url.ok_or(rusqlite::Error::InvalidQuery)?;
            Ok(Action::OpenUrl {
                url: SafeUrl::parse(url).map_err(domain_to_sql_error)?,
            })
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn decode_control(
    page: i64,
    control_kind: String,
    control_index: i64,
    control_operation: String,
) -> Result<ControlId, rusqlite::Error> {
    let page = u8::try_from(page).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let control_index = u8::try_from(control_index).map_err(|_| rusqlite::Error::InvalidQuery)?;
    match control_kind.as_str() {
        "key" if control_operation == "press" => {
            ControlId::key(page, control_index).map_err(domain_to_sql_read_error)
        }
        "key" => Err(rusqlite::Error::InvalidQuery),
        "encoder" => ControlId::encoder(
            page,
            control_index,
            decode_encoder_operation(&control_operation)?,
        )
        .map_err(domain_to_sql_read_error),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn decode_encoder_operation(value: &str) -> Result<EncoderOperation, rusqlite::Error> {
    match value {
        "left" => Ok(EncoderOperation::Left),
        "right" => Ok(EncoderOperation::Right),
        "press" => Ok(EncoderOperation::Press),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn domain_to_sql_error(error: keyro_core_domain::DomainError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn domain_to_sql_read_error(error: keyro_core_domain::DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn to_app_error(error: impl std::error::Error) -> AppError {
    AppError::Storage(error.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum SqliteStorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyro_core_domain::{EncoderOperation, SafeUrl};

    #[test]
    fn creates_default_active_profile() {
        let repository = SqliteProfileRepository::in_memory().unwrap();

        let profile = repository.ensure_default_profile().unwrap();
        let profiles = repository.list_profiles().unwrap();

        assert!(profile.is_active);
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].is_active);
    }

    #[test]
    fn persists_assignment_round_trip() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::encoder(1, 1, EncoderOperation::Right).unwrap();
        let assignment = Assignment {
            profile_id: profile.id,
            control,
            actions: vec![Action::OpenUrl {
                url: SafeUrl::parse("https://example.com/path").unwrap(),
            }],
        };
        let replacement = Assignment::single(
            profile.id,
            control,
            Action::OpenUrl {
                url: SafeUrl::parse("https://example.com/replacement").unwrap(),
            },
        )
        .unwrap();

        repository.save_assignment(&assignment).unwrap();
        repository.save_assignment(&replacement).unwrap();
        let loaded = repository
            .find_assignment(profile.id, control)
            .unwrap()
            .unwrap();

        assert_eq!(loaded, replacement);
        assert_eq!(
            repository.load_snapshot_state().unwrap().assignments,
            vec![replacement]
        );
    }

    #[test]
    fn file_database_restores_profile_and_assignment() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();
        let profile_id;
        let control = ControlId::key(0, 0).unwrap();
        {
            let repository = SqliteProfileRepository::open(&path).unwrap();
            let profile = repository.ensure_default_profile().unwrap();
            profile_id = profile.id;
            let assignment = Assignment::single(
                profile.id,
                control,
                Action::OpenUrl {
                    url: SafeUrl::parse("https://example.com").unwrap(),
                },
            )
            .unwrap();
            repository.save_assignment(&assignment).unwrap();
        }

        let repository = SqliteProfileRepository::open(&path).unwrap();
        assert_eq!(repository.list_profiles().unwrap()[0].id, profile_id);
        assert!(repository
            .find_assignment(profile_id, control)
            .unwrap()
            .is_some());
        assert_eq!(
            repository.load_snapshot_state().unwrap().assignments.len(),
            1
        );
    }

    #[test]
    fn snapshot_state_preserves_ordered_assignment_actions() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        let assignment = Assignment {
            profile_id: profile.id,
            control,
            actions: vec![
                Action::OpenUrl {
                    url: SafeUrl::parse("https://example.com/first").unwrap(),
                },
                Action::OpenUrl {
                    url: SafeUrl::parse("https://example.com/second").unwrap(),
                },
            ],
        };

        repository.save_assignment(&assignment).unwrap();

        assert_eq!(
            repository.load_snapshot_state().unwrap().assignments,
            vec![assignment]
        );
    }

    #[test]
    fn failed_assignment_save_rolls_back_existing_assignment() {
        let repository = SqliteProfileRepository::in_memory().unwrap();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        let original = Assignment::single(
            profile.id,
            control,
            Action::OpenUrl {
                url: SafeUrl::parse("https://example.com/original").unwrap(),
            },
        )
        .unwrap();
        repository.save_assignment(&original).unwrap();

        repository
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                "
                CREATE TRIGGER fail_assignment_insert
                BEFORE INSERT ON assignments
                BEGIN
                  SELECT RAISE(ABORT, 'forced insert failure');
                END;
                ",
            )
            .unwrap();
        let replacement = Assignment::single(
            profile.id,
            control,
            Action::OpenUrl {
                url: SafeUrl::parse("https://example.com/replacement").unwrap(),
            },
        )
        .unwrap();

        assert!(repository.save_assignment(&replacement).is_err());
        repository
            .connection
            .lock()
            .unwrap()
            .execute("DROP TRIGGER fail_assignment_insert", [])
            .unwrap();
        let loaded = repository
            .find_assignment(profile.id, control)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, original);
    }
}
