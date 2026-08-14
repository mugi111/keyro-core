use std::fmt;

use keyro_core_domain::SafeUrl;
use keyro_core_domain::{
    ensure_exactly_one_active, Action, Assignment, ControlId, Profile, ProfileId,
    ENCODERS_PER_PAGE, KEY_COLUMNS, KEY_ROWS, PAGE_COUNT,
};
use uuid::Uuid;

pub trait ProfileRepository: Send + Sync {
    fn ensure_default_profile(&self) -> Result<Profile, AppError>;
    fn create_profile(&self, name: String) -> Result<Profile, AppError>;
    fn list_profiles(&self) -> Result<Vec<Profile>, AppError>;
    fn rename_profile(&self, profile_id: ProfileId, name: String) -> Result<Profile, AppError>;
    fn set_active_profile(&self, profile_id: ProfileId) -> Result<(), AppError>;
    fn save_assignment(&self, assignment: &Assignment) -> Result<(), AppError>;
    fn clear_assignment(&self, profile_id: ProfileId, control: ControlId) -> Result<(), AppError>;
    fn load_snapshot_state(&self) -> Result<SnapshotState, AppError>;
    fn find_assignment(
        &self,
        profile_id: ProfileId,
        control: ControlId,
    ) -> Result<Option<Assignment>, AppError>;
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent) -> Result<(), AppError>;
}

pub trait UrlOpener: Send + Sync {
    fn open_url(&self, url: &SafeUrl) -> Result<(), OpenUrlError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreCommand {
    GetSnapshot,
    ListProfiles,
    CreateProfile {
        name: String,
    },
    RenameProfile {
        profile_id: ProfileId,
        name: String,
    },
    SetActiveProfile {
        profile_id: ProfileId,
    },
    SaveAssignment {
        assignment: Assignment,
    },
    ClearAssignment {
        profile_id: ProfileId,
        control: ControlId,
    },
    VirtualControlInput {
        control: ControlId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreResponse {
    Snapshot { snapshot: CoreSnapshot },
    Profiles { profiles: Vec<Profile> },
    Profile { profile: Profile },
    Acknowledged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSnapshot {
    pub layout: DeviceLayout,
    pub profiles: Vec<Profile>,
    pub assignments: Vec<SnapshotAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotAssignment {
    pub profile_id: ProfileId,
    pub control: ControlId,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotState {
    pub profiles: Vec<Profile>,
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLayout {
    pub page_count: u8,
    pub key_rows: u8,
    pub key_columns: u8,
    pub encoder_count: u8,
}

pub const MVP_DEVICE_LAYOUT: DeviceLayout = DeviceLayout {
    page_count: PAGE_COUNT,
    key_rows: KEY_ROWS,
    key_columns: KEY_COLUMNS,
    encoder_count: ENCODERS_PER_PAGE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRunId(Uuid);

impl ActionRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ActionRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ActionRunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    ActionRunning {
        run_id: ActionRunId,
        profile_id: ProfileId,
        control: ControlId,
    },
    ActionSucceeded {
        run_id: ActionRunId,
    },
    ActionFailed {
        run_id: ActionRunId,
        failure: ActionFailure,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionFailure {
    OpenUrlFailed,
}

pub struct CoreService<R, O, E> {
    repository: R,
    url_opener: O,
    event_sink: E,
}

impl<R, O, E> CoreService<R, O, E>
where
    R: ProfileRepository,
    O: UrlOpener,
    E: EventSink,
{
    pub fn new(repository: R, url_opener: O, event_sink: E) -> Self {
        Self {
            repository,
            url_opener,
            event_sink,
        }
    }

    pub fn initialize(&self) -> Result<Profile, AppError> {
        let profile = self.repository.ensure_default_profile()?;
        let profiles = self.repository.list_profiles()?;
        ensure_exactly_one_active(&profiles).map_err(AppError::Domain)?;
        Ok(profile)
    }

    pub fn handle_command(&self, command: CoreCommand) -> Result<CoreResponse, AppError> {
        match command {
            CoreCommand::GetSnapshot => {
                let state = self.repository.load_snapshot_state()?;
                ensure_exactly_one_active(&state.profiles).map_err(AppError::Domain)?;
                let assignments = state
                    .assignments
                    .into_iter()
                    .map(|assignment| SnapshotAssignment {
                        profile_id: assignment.profile_id,
                        control: assignment.control,
                        actions: assignment.actions,
                    })
                    .collect();
                Ok(CoreResponse::Snapshot {
                    snapshot: CoreSnapshot {
                        layout: MVP_DEVICE_LAYOUT,
                        profiles: state.profiles,
                        assignments,
                    },
                })
            }
            CoreCommand::ListProfiles => {
                let profiles = self.repository.list_profiles()?;
                ensure_exactly_one_active(&profiles).map_err(AppError::Domain)?;
                Ok(CoreResponse::Profiles { profiles })
            }
            CoreCommand::CreateProfile { name } => {
                let profile = self.repository.create_profile(name)?;
                Ok(CoreResponse::Profile { profile })
            }
            CoreCommand::RenameProfile { profile_id, name } => {
                let profile = self.repository.rename_profile(profile_id, name)?;
                Ok(CoreResponse::Profile { profile })
            }
            CoreCommand::SetActiveProfile { profile_id } => {
                self.repository.set_active_profile(profile_id)?;
                Ok(CoreResponse::Acknowledged)
            }
            CoreCommand::SaveAssignment { assignment } => {
                self.repository.save_assignment(&assignment)?;
                Ok(CoreResponse::Acknowledged)
            }
            CoreCommand::ClearAssignment {
                profile_id,
                control,
            } => {
                self.repository.clear_assignment(profile_id, control)?;
                Ok(CoreResponse::Acknowledged)
            }
            CoreCommand::VirtualControlInput { control } => {
                self.route_virtual_control(control)?;
                Ok(CoreResponse::Acknowledged)
            }
        }
    }

    pub fn route_virtual_control(&self, control: ControlId) -> Result<(), AppError> {
        let profile = self.active_profile()?;
        let Some(assignment) = self.repository.find_assignment(profile.id, control)? else {
            return Err(AppError::AssignmentNotFound);
        };
        let Some(action) = assignment.first_action() else {
            return Err(AppError::AssignmentHasNoAction);
        };

        let run_id = ActionRunId::new();
        self.event_sink.emit(CoreEvent::ActionRunning {
            run_id: run_id.clone(),
            profile_id: profile.id,
            control,
        })?;

        let result = match action {
            Action::OpenUrl { url } => self.url_opener.open_url(url).map_err(AppError::OpenUrl),
        };

        match result {
            Ok(()) => {
                self.event_sink
                    .emit(CoreEvent::ActionSucceeded { run_id })?;
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.event_sink.emit(CoreEvent::ActionFailed {
                    run_id,
                    failure: ActionFailure::OpenUrlFailed,
                    message,
                })?;
                Err(error)
            }
        }
    }

    fn active_profile(&self) -> Result<Profile, AppError> {
        let profiles = self.repository.list_profiles()?;
        ensure_exactly_one_active(&profiles).map_err(AppError::Domain)?;
        profiles
            .into_iter()
            .find(|profile| profile.is_active)
            .ok_or(AppError::ActiveProfileMissing)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] keyro_core_domain::DomainError),
    #[error(transparent)]
    OpenUrl(#[from] OpenUrlError),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("event sink error: {0}")]
    EventSink(String),
    #[error("active profile is missing")]
    ActiveProfileMissing,
    #[error("profile was not found")]
    ProfileNotFound,
    #[error("assignment was not found for the active profile and control")]
    AssignmentNotFound,
    #[error("assignment has no action")]
    AssignmentHasNoAction,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct OpenUrlError {
    message: String,
}

impl OpenUrlError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyro_core_domain::{EncoderOperation, SafeUrl};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MemoryRepository {
        profiles: Arc<Mutex<Vec<Profile>>>,
        assignments: Arc<Mutex<Vec<Assignment>>>,
    }

    impl MemoryRepository {
        fn new() -> Self {
            Self {
                profiles: Arc::new(Mutex::new(Vec::new())),
                assignments: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ProfileRepository for MemoryRepository {
        fn ensure_default_profile(&self) -> Result<Profile, AppError> {
            let mut profiles = self.profiles.lock().unwrap();
            if let Some(profile) = profiles.iter().find(|profile| profile.is_active) {
                return Ok(profile.clone());
            }

            let profile = Profile::new("Default", true)?;
            profiles.push(profile.clone());
            Ok(profile)
        }

        fn create_profile(&self, name: String) -> Result<Profile, AppError> {
            let profile = Profile::new(name, false)?;
            self.profiles.lock().unwrap().push(profile.clone());
            Ok(profile)
        }

        fn list_profiles(&self) -> Result<Vec<Profile>, AppError> {
            Ok(self.profiles.lock().unwrap().clone())
        }

        fn rename_profile(&self, profile_id: ProfileId, name: String) -> Result<Profile, AppError> {
            let mut profiles = self.profiles.lock().unwrap();
            let profile = profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
                .ok_or(AppError::ProfileNotFound)?;
            profile.rename(name)?;
            Ok(profile.clone())
        }

        fn set_active_profile(&self, profile_id: ProfileId) -> Result<(), AppError> {
            let mut profiles = self.profiles.lock().unwrap();
            if !profiles.iter().any(|profile| profile.id == profile_id) {
                return Err(AppError::ProfileNotFound);
            }
            for profile in profiles.iter_mut() {
                profile.is_active = profile.id == profile_id;
            }
            Ok(())
        }

        fn save_assignment(&self, assignment: &Assignment) -> Result<(), AppError> {
            if !self
                .profiles
                .lock()
                .unwrap()
                .iter()
                .any(|profile| profile.id == assignment.profile_id)
            {
                return Err(AppError::ProfileNotFound);
            }
            let mut assignments = self.assignments.lock().unwrap();
            assignments.retain(|candidate| {
                !(candidate.profile_id == assignment.profile_id
                    && candidate.control == assignment.control)
            });
            assignments.push(assignment.clone());
            Ok(())
        }

        fn clear_assignment(
            &self,
            profile_id: ProfileId,
            control: ControlId,
        ) -> Result<(), AppError> {
            if !self
                .profiles
                .lock()
                .unwrap()
                .iter()
                .any(|profile| profile.id == profile_id)
            {
                return Err(AppError::ProfileNotFound);
            }
            self.assignments.lock().unwrap().retain(|assignment| {
                !(assignment.profile_id == profile_id && assignment.control == control)
            });
            Ok(())
        }

        fn load_snapshot_state(&self) -> Result<SnapshotState, AppError> {
            Ok(SnapshotState {
                profiles: self.profiles.lock().unwrap().clone(),
                assignments: self.assignments.lock().unwrap().clone(),
            })
        }

        fn find_assignment(
            &self,
            profile_id: ProfileId,
            control: ControlId,
        ) -> Result<Option<Assignment>, AppError> {
            Ok(self
                .assignments
                .lock()
                .unwrap()
                .iter()
                .find(|assignment| {
                    assignment.profile_id == profile_id && assignment.control == control
                })
                .cloned())
        }
    }

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

    #[derive(Clone, Default)]
    struct RecordingEvents {
        events: Arc<Mutex<Vec<CoreEvent>>>,
    }

    impl EventSink for RecordingEvents {
        fn emit(&self, event: CoreEvent) -> Result<(), AppError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[test]
    fn virtual_control_routes_open_url_and_emits_events() {
        let repository = MemoryRepository::new();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::encoder(0, 0, EncoderOperation::Press).unwrap();
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
        let events = RecordingEvents::default();
        let service = CoreService::new(repository, opener.clone(), events.clone());

        service.route_virtual_control(control).unwrap();

        assert_eq!(
            opener.opened.lock().unwrap().as_slice(),
            ["https://example.com"]
        );
        let events = events.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        let CoreEvent::ActionRunning { run_id, .. } = &events[0] else {
            panic!("expected running event");
        };
        let CoreEvent::ActionSucceeded {
            run_id: succeeded_run_id,
        } = &events[1]
        else {
            panic!("expected succeeded event");
        };
        assert_eq!(succeeded_run_id, run_id);
    }

    #[test]
    fn snapshot_returns_layout_profiles_and_effective_assignments() {
        let repository = MemoryRepository::new();
        let profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        repository
            .save_assignment(&Assignment {
                profile_id: profile.id,
                control,
                actions: vec![
                    Action::OpenUrl {
                        url: SafeUrl::parse("https://example.com").unwrap(),
                    },
                    Action::OpenUrl {
                        url: SafeUrl::parse("https://example.com/second").unwrap(),
                    },
                ],
            })
            .unwrap();
        let service = CoreService::new(
            repository,
            RecordingOpener::default(),
            RecordingEvents::default(),
        );

        let CoreResponse::Snapshot { snapshot } =
            service.handle_command(CoreCommand::GetSnapshot).unwrap()
        else {
            panic!("expected snapshot response");
        };

        assert_eq!(snapshot.layout, MVP_DEVICE_LAYOUT);
        assert_eq!(snapshot.profiles, vec![profile]);
        assert_eq!(snapshot.assignments.len(), 1);
        assert_eq!(snapshot.assignments[0].control, control);
        assert_eq!(snapshot.assignments[0].actions.len(), 2);
    }

    #[test]
    fn profile_commands_create_rename_and_clear_assignments() {
        let repository = MemoryRepository::new();
        let default_profile = repository.ensure_default_profile().unwrap();
        let control = ControlId::key(0, 0).unwrap();
        repository
            .save_assignment(
                &Assignment::single(
                    default_profile.id,
                    control,
                    Action::OpenUrl {
                        url: SafeUrl::parse("https://example.com").unwrap(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let service = CoreService::new(
            repository,
            RecordingOpener::default(),
            RecordingEvents::default(),
        );

        let CoreResponse::Profile { profile: work } = service
            .handle_command(CoreCommand::CreateProfile {
                name: " Work ".to_owned(),
            })
            .unwrap()
        else {
            panic!("expected profile response");
        };
        assert_eq!(work.name, " Work ");
        assert!(!work.is_active);

        let CoreResponse::Profiles { profiles } =
            service.handle_command(CoreCommand::ListProfiles).unwrap()
        else {
            panic!("expected profiles response");
        };
        assert!(profiles.iter().any(|profile| profile.id == work.id));

        let CoreResponse::Profile { profile: renamed } = service
            .handle_command(CoreCommand::RenameProfile {
                profile_id: work.id,
                name: " Deep Work ".to_owned(),
            })
            .unwrap()
        else {
            panic!("expected profile response");
        };
        assert_eq!(renamed.id, work.id);
        assert_eq!(renamed.name, " Deep Work ");
        assert!(!renamed.is_active);
        assert_eq!(
            service
                .handle_command(CoreCommand::ClearAssignment {
                    profile_id: default_profile.id,
                    control
                })
                .unwrap(),
            CoreResponse::Acknowledged
        );

        let CoreResponse::Snapshot { snapshot } =
            service.handle_command(CoreCommand::GetSnapshot).unwrap()
        else {
            panic!("expected snapshot response");
        };
        assert!(snapshot
            .profiles
            .iter()
            .any(|profile| profile.name == " Deep Work " && !profile.is_active));
        assert!(snapshot.assignments.is_empty());
    }

    #[test]
    fn profile_commands_reject_invalid_inputs() {
        let repository = MemoryRepository::new();
        repository.ensure_default_profile().unwrap();
        let service = CoreService::new(
            repository,
            RecordingOpener::default(),
            RecordingEvents::default(),
        );
        let missing_profile = ProfileId::new();

        assert!(matches!(
            service
                .handle_command(CoreCommand::CreateProfile {
                    name: "   ".to_owned()
                })
                .unwrap_err(),
            AppError::Domain(keyro_core_domain::DomainError::EmptyProfileName)
        ));
        assert!(matches!(
            service
                .handle_command(CoreCommand::RenameProfile {
                    profile_id: missing_profile,
                    name: "Renamed".to_owned()
                })
                .unwrap_err(),
            AppError::ProfileNotFound
        ));
        assert!(matches!(
            service
                .handle_command(CoreCommand::ClearAssignment {
                    profile_id: missing_profile,
                    control: ControlId::key(0, 0).unwrap()
                })
                .unwrap_err(),
            AppError::ProfileNotFound
        ));
        assert!(matches!(
            service
                .handle_command(CoreCommand::SaveAssignment {
                    assignment: Assignment::single(
                        missing_profile,
                        ControlId::key(0, 0).unwrap(),
                        Action::OpenUrl {
                            url: SafeUrl::parse("https://example.com").unwrap()
                        },
                    )
                    .unwrap()
                })
                .unwrap_err(),
            AppError::ProfileNotFound
        ));
    }

    #[test]
    fn virtual_control_emits_failed_event_when_open_url_fails() {
        let repository = MemoryRepository::new();
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
        let events = RecordingEvents::default();
        let service = CoreService::new(repository, opener, events.clone());

        assert!(matches!(
            service.route_virtual_control(control).unwrap_err(),
            AppError::OpenUrl(_)
        ));

        let events = events.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        let CoreEvent::ActionRunning { run_id, .. } = &events[0] else {
            panic!("expected running event");
        };
        let CoreEvent::ActionFailed {
            run_id: failed_run_id,
            failure,
            message,
        } = &events[1]
        else {
            panic!("expected failed event");
        };
        assert_eq!(failed_run_id, run_id);
        assert_eq!(*failure, ActionFailure::OpenUrlFailed);
        assert_eq!(message, "open failed");
    }
}
