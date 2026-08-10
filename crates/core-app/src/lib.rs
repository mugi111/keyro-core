use std::fmt;

use keyro_core_domain::SafeUrl;
use keyro_core_domain::{
    ensure_exactly_one_active, Action, Assignment, ControlId, Profile, ProfileId,
};
use uuid::Uuid;

pub trait ProfileRepository: Send + Sync {
    fn ensure_default_profile(&self) -> Result<Profile, AppError>;
    fn list_profiles(&self) -> Result<Vec<Profile>, AppError>;
    fn set_active_profile(&self, profile_id: ProfileId) -> Result<(), AppError>;
    fn save_assignment(&self, assignment: &Assignment) -> Result<(), AppError>;
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
    ListProfiles,
    SetActiveProfile { profile_id: ProfileId },
    SaveAssignment { assignment: Assignment },
    VirtualControlInput { control: ControlId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreResponse {
    Profiles { profiles: Vec<Profile> },
    Acknowledged,
}

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
            CoreCommand::ListProfiles => {
                let profiles = self.repository.list_profiles()?;
                ensure_exactly_one_active(&profiles).map_err(AppError::Domain)?;
                Ok(CoreResponse::Profiles { profiles })
            }
            CoreCommand::SetActiveProfile { profile_id } => {
                self.repository.set_active_profile(profile_id)?;
                Ok(CoreResponse::Acknowledged)
            }
            CoreCommand::SaveAssignment { assignment } => {
                self.repository.save_assignment(&assignment)?;
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

        fn list_profiles(&self) -> Result<Vec<Profile>, AppError> {
            Ok(self.profiles.lock().unwrap().clone())
        }

        fn set_active_profile(&self, profile_id: ProfileId) -> Result<(), AppError> {
            let mut profiles = self.profiles.lock().unwrap();
            for profile in profiles.iter_mut() {
                profile.is_active = profile.id == profile_id;
            }
            Ok(())
        }

        fn save_assignment(&self, assignment: &Assignment) -> Result<(), AppError> {
            self.assignments.lock().unwrap().push(assignment.clone());
            Ok(())
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
