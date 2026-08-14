use keyro_core_app::{
    ActionFailure, AppError, CoreCommand, CoreEvent, CoreResponse, DeviceLayout, SnapshotAssignment,
};
use keyro_core_domain::{Action, Assignment, ControlId, EncoderOperation, ProfileId, SafeUrl};
use keyro_core_protocol::{
    decode_client_envelope, encode_server_message, handshake_response, ActionDto, ActionEventDto,
    ActionFailureCode, AssignmentDto, ClientMessage, ControlDto, DeviceLayoutDto,
    EncoderOperationDto, ErrorCode, ErrorDto, ProtocolError, ServerMessage, SnapshotAssignmentDto,
};
use serde_json::Value;

pub struct ProtocolSession {
    core_version: String,
    state: SessionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    AwaitingHandshake,
    Established,
}

pub enum ProtocolDispatch {
    Immediate(ServerMessage),
    Command {
        request_id: String,
        command: CoreCommand,
    },
}

impl ProtocolSession {
    pub fn new(core_version: impl Into<String>) -> Self {
        Self {
            core_version: core_version.into(),
            state: SessionState::AwaitingHandshake,
        }
    }

    pub fn receive_line(&mut self, line: &str) -> ProtocolDispatch {
        match decode_client_envelope(line) {
            Ok(envelope) => self.dispatch_envelope(envelope),
            Err(error) => {
                let request_id = extract_request_id(line);
                ProtocolDispatch::Immediate(protocol_error_message(request_id, &error))
            }
        }
    }

    fn dispatch_envelope(
        &mut self,
        envelope: keyro_core_protocol::ClientEnvelope,
    ) -> ProtocolDispatch {
        match envelope.message {
            ClientMessage::Handshake(_) if self.state == SessionState::Established => {
                ProtocolDispatch::Immediate(ServerMessage::Error {
                    request_id: Some(envelope.request_id),
                    error: ErrorDto {
                        code: ErrorCode::ValidationFailed,
                        message: "handshake already completed for this connection".to_owned(),
                    },
                })
            }
            ClientMessage::Handshake(message) => {
                let response = handshake_response(
                    envelope.request_id,
                    message.protocol,
                    self.core_version.clone(),
                );
                if matches!(response, ServerMessage::HandshakeAccepted { .. }) {
                    self.state = SessionState::Established;
                }
                ProtocolDispatch::Immediate(response)
            }
            message if self.state == SessionState::Established => {
                match command_from_message(message) {
                    Ok(command) => ProtocolDispatch::Command {
                        request_id: envelope.request_id,
                        command,
                    },
                    Err(error) => ProtocolDispatch::Immediate(ipc_error_message(
                        Some(envelope.request_id),
                        &error,
                    )),
                }
            }
            _ => ProtocolDispatch::Immediate(ServerMessage::Error {
                request_id: Some(envelope.request_id),
                error: ErrorDto {
                    code: ErrorCode::ValidationFailed,
                    message: "handshake required before command routing".to_owned(),
                },
            }),
        }
    }
}

#[cfg(test)]
fn decode_protocol_dispatch(line: &str, core_version: &str) -> ProtocolDispatch {
    match decode_client_envelope(line) {
        Ok(envelope) => match envelope.message {
            ClientMessage::Handshake(message) => ProtocolDispatch::Immediate(handshake_response(
                envelope.request_id,
                message.protocol,
                core_version.to_owned(),
            )),
            message => match command_from_message(message) {
                Ok(command) => ProtocolDispatch::Command {
                    request_id: envelope.request_id,
                    command,
                },
                Err(error) => ProtocolDispatch::Immediate(ipc_error_message(
                    Some(envelope.request_id),
                    &error,
                )),
            },
        },
        Err(error) => {
            let request_id = extract_request_id(line);
            ProtocolDispatch::Immediate(protocol_error_message(request_id, &error))
        }
    }
}

#[cfg(test)]
fn decode_protocol_command(line: &str) -> Result<(String, CoreCommand), IpcError> {
    let envelope = decode_client_envelope(line).map_err(IpcError::Protocol)?;
    Ok((envelope.request_id, command_from_message(envelope.message)?))
}

pub fn encode_protocol_message(message: &ServerMessage) -> Result<String, IpcError> {
    encode_server_message(message).map_err(IpcError::Protocol)
}

pub fn response_from_core(request_id: String, response: CoreResponse) -> ServerMessage {
    match response {
        CoreResponse::Snapshot { snapshot } => ServerMessage::Snapshot {
            request_id,
            layout: layout_to_dto(snapshot.layout),
            profiles: snapshot.profiles.into_iter().map(profile_to_dto).collect(),
            assignments: snapshot
                .assignments
                .into_iter()
                .map(snapshot_assignment_to_dto)
                .collect(),
        },
        CoreResponse::Profiles { profiles } => ServerMessage::Profiles {
            request_id,
            profiles: profiles.into_iter().map(profile_to_dto).collect(),
        },
        CoreResponse::Profile { profile } => ServerMessage::Profile {
            request_id,
            profile: profile_to_dto(profile),
        },
        CoreResponse::Acknowledged => ServerMessage::Acknowledged { request_id },
    }
}

pub fn event_from_core(event: CoreEvent) -> ServerMessage {
    let event = match event {
        CoreEvent::ActionRunning {
            run_id,
            profile_id,
            control,
        } => ActionEventDto::Running {
            execution_id: run_id.to_string(),
            profile_id: profile_id.to_string(),
            control: control_to_dto(control),
        },
        CoreEvent::ActionSucceeded { run_id } => ActionEventDto::Succeeded {
            execution_id: run_id.to_string(),
        },
        CoreEvent::ActionFailed {
            run_id,
            failure,
            message,
        } => ActionEventDto::Failed {
            execution_id: run_id.to_string(),
            code: action_failure_to_dto(failure),
            message,
        },
    };

    ServerMessage::ActionEvent { event }
}

fn protocol_error_message(request_id: Option<String>, error: &ProtocolError) -> ServerMessage {
    let code = match error {
        ProtocolError::Decode(_) => ErrorCode::UnknownMessage,
        ProtocolError::Encode(_) => ErrorCode::Internal,
        ProtocolError::EmptyRequestId => ErrorCode::ValidationFailed,
    };

    ServerMessage::Error {
        request_id,
        error: ErrorDto {
            code,
            message: error.to_string(),
        },
    }
}

fn ipc_error_message(request_id: Option<String>, error: &IpcError) -> ServerMessage {
    let code = match error {
        IpcError::Protocol(error) => match error {
            ProtocolError::Decode(_) => ErrorCode::UnknownMessage,
            ProtocolError::Encode(_) => ErrorCode::Internal,
            ProtocolError::EmptyRequestId => ErrorCode::ValidationFailed,
        },
        IpcError::Domain(_) | IpcError::IndexOutOfRange { .. } => ErrorCode::ValidationFailed,
        IpcError::HandshakeRequiresSession => ErrorCode::ValidationFailed,
    };

    ServerMessage::Error {
        request_id,
        error: ErrorDto {
            code,
            message: error.to_string(),
        },
    }
}

pub fn error_message(request_id: Option<String>, error: &AppError) -> ServerMessage {
    let (code, message) = match error {
        AppError::Domain(_)
        | AppError::ActiveProfileMissing
        | AppError::AssignmentNotFound
        | AppError::AssignmentHasNoAction => (ErrorCode::ValidationFailed, "validation failed"),
        AppError::ProfileNotFound => (ErrorCode::NotFound, "profile not found"),
        AppError::OpenUrl(_) | AppError::Storage(_) | AppError::EventSink(_) => {
            (ErrorCode::Internal, "command failed")
        }
    };

    ServerMessage::Error {
        request_id,
        error: ErrorDto {
            code,
            message: message.to_owned(),
        },
    }
}

fn extract_request_id(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|request_id| !request_id.trim().is_empty())
}

fn command_from_message(message: ClientMessage) -> Result<CoreCommand, IpcError> {
    match message {
        ClientMessage::Handshake(_) => Err(IpcError::HandshakeRequiresSession),
        ClientMessage::GetSnapshot(_) => Ok(CoreCommand::GetSnapshot),
        ClientMessage::ListProfiles(_) => Ok(CoreCommand::ListProfiles),
        ClientMessage::CreateProfile(message) => {
            Ok(CoreCommand::CreateProfile { name: message.name })
        }
        ClientMessage::RenameProfile(message) => Ok(CoreCommand::RenameProfile {
            profile_id: ProfileId::parse(&message.profile_id)?,
            name: message.name,
        }),
        ClientMessage::SetActiveProfile(message) => Ok(CoreCommand::SetActiveProfile {
            profile_id: ProfileId::parse(&message.profile_id)?,
        }),
        ClientMessage::SaveAssignment(message) => Ok(CoreCommand::SaveAssignment {
            assignment: assignment_from_dto(message.assignment)?,
        }),
        ClientMessage::ClearAssignment(message) => Ok(CoreCommand::ClearAssignment {
            profile_id: ProfileId::parse(&message.profile_id)?,
            control: control_from_dto(message.control)?,
        }),
        ClientMessage::VirtualControlInput(message) => Ok(CoreCommand::VirtualControlInput {
            control: control_from_dto(message.control)?,
        }),
    }
}

fn profile_to_dto(profile: keyro_core_domain::Profile) -> keyro_core_protocol::ProfileDto {
    keyro_core_protocol::ProfileDto {
        id: profile.id.to_string(),
        name: profile.name,
        is_active: profile.is_active,
    }
}

fn assignment_from_dto(assignment: AssignmentDto) -> Result<Assignment, IpcError> {
    Assignment::single(
        ProfileId::parse(&assignment.profile_id)?,
        control_from_dto(assignment.control)?,
        action_from_dto(assignment.action)?,
    )
    .map_err(IpcError::Domain)
}

fn action_from_dto(action: ActionDto) -> Result<Action, IpcError> {
    match action {
        ActionDto::OpenUrl { url } => Ok(Action::OpenUrl {
            url: SafeUrl::parse(url)?,
        }),
    }
}

fn action_to_dto(action: Action) -> ActionDto {
    match action {
        Action::OpenUrl { url } => ActionDto::OpenUrl {
            url: url.as_str().to_owned(),
        },
    }
}

fn layout_to_dto(layout: DeviceLayout) -> DeviceLayoutDto {
    DeviceLayoutDto {
        page_count: layout.page_count as u16,
        key_rows: layout.key_rows as u16,
        key_columns: layout.key_columns as u16,
        encoder_count: layout.encoder_count as u16,
    }
}

fn snapshot_assignment_to_dto(assignment: SnapshotAssignment) -> SnapshotAssignmentDto {
    SnapshotAssignmentDto {
        profile_id: assignment.profile_id.to_string(),
        control: control_to_dto(assignment.control),
        actions: assignment.actions.into_iter().map(action_to_dto).collect(),
    }
}

fn control_from_dto(control: ControlDto) -> Result<ControlId, IpcError> {
    match control {
        ControlDto::Key { page, key } => {
            ControlId::key(u8_index("page", page)?, u8_index("key", key)?).map_err(IpcError::Domain)
        }
        ControlDto::Encoder {
            page,
            encoder,
            operation,
        } => ControlId::encoder(
            u8_index("page", page)?,
            u8_index("encoder", encoder)?,
            encoder_operation_from_dto(operation),
        )
        .map_err(IpcError::Domain),
    }
}

fn u8_index(field: &'static str, value: u16) -> Result<u8, IpcError> {
    u8::try_from(value).map_err(|_| IpcError::IndexOutOfRange { field, value })
}

fn encoder_operation_from_dto(operation: EncoderOperationDto) -> EncoderOperation {
    match operation {
        EncoderOperationDto::Left => EncoderOperation::Left,
        EncoderOperationDto::Right => EncoderOperation::Right,
        EncoderOperationDto::Press => EncoderOperation::Press,
    }
}

fn control_to_dto(control: ControlId) -> ControlDto {
    match control {
        ControlId::Key { page, key } => ControlDto::Key {
            page: page.get() as u16,
            key: key.get() as u16,
        },
        ControlId::Encoder {
            page,
            encoder,
            operation,
        } => ControlDto::Encoder {
            page: page.get() as u16,
            encoder: encoder.get() as u16,
            operation: encoder_operation_to_dto(operation),
        },
    }
}

fn encoder_operation_to_dto(operation: EncoderOperation) -> EncoderOperationDto {
    match operation {
        EncoderOperation::Left => EncoderOperationDto::Left,
        EncoderOperation::Right => EncoderOperationDto::Right,
        EncoderOperation::Press => EncoderOperationDto::Press,
    }
}

fn action_failure_to_dto(failure: ActionFailure) -> ActionFailureCode {
    match failure {
        ActionFailure::OpenUrlFailed => ActionFailureCode::OpenUrlFailed,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Domain(#[from] keyro_core_domain::DomainError),
    #[error("handshake messages must be handled by the IPC session before command routing")]
    HandshakeRequiresSession,
    #[error("{field} index {value} is outside the supported protocol adapter range")]
    IndexOutOfRange { field: &'static str, value: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyro_core_app::{CoreSnapshot, MVP_DEVICE_LAYOUT};

    #[test]
    fn decodes_protocol_virtual_control_command() {
        let line = include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");

        let (request_id, command) = decode_protocol_command(line).unwrap();

        assert_eq!(request_id, "req-virtual-key-press");
        assert_eq!(
            command,
            CoreCommand::VirtualControlInput {
                control: ControlId::key(0, 0).unwrap()
            }
        );
    }

    #[test]
    fn decodes_protocol_assignment_command() {
        let line =
            include_str!("../../../protocol/test-vectors/v0.2.0/save-assignment-open-url.json");

        let (request_id, command) = decode_protocol_command(line).unwrap();

        assert_eq!(request_id, "req-save-assignment");
        assert!(matches!(command, CoreCommand::SaveAssignment { .. }));
    }

    #[test]
    fn decodes_protocol_snapshot_command() {
        let line = include_str!("../../../protocol/test-vectors/v0.2.0/get-snapshot.json");

        let (request_id, command) = decode_protocol_command(line).unwrap();

        assert_eq!(request_id, "req-snapshot");
        assert_eq!(command, CoreCommand::GetSnapshot);
    }

    #[test]
    fn decodes_protocol_profile_mutation_commands() {
        let (request_id, command) = decode_protocol_command(include_str!(
            "../../../protocol/test-vectors/v0.3.0/create-profile.json"
        ))
        .unwrap();
        assert_eq!(request_id, "req-create-profile");
        assert_eq!(
            command,
            CoreCommand::CreateProfile {
                name: "Work".to_owned()
            }
        );

        let (request_id, command) = decode_protocol_command(include_str!(
            "../../../protocol/test-vectors/v0.3.0/rename-profile.json"
        ))
        .unwrap();
        assert_eq!(request_id, "req-rename-profile");
        assert!(matches!(command, CoreCommand::RenameProfile { .. }));

        let (request_id, command) = decode_protocol_command(include_str!(
            "../../../protocol/test-vectors/v0.3.0/clear-assignment.json"
        ))
        .unwrap();
        assert_eq!(request_id, "req-clear-assignment");
        assert!(matches!(command, CoreCommand::ClearAssignment { .. }));
    }

    #[test]
    fn encodes_snapshot_response_with_layout_and_assignments() {
        let profile = keyro_core_domain::Profile {
            id: ProfileId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "Default".to_owned(),
            is_active: true,
        };
        let response = response_from_core(
            "req-snapshot".to_owned(),
            CoreResponse::Snapshot {
                snapshot: CoreSnapshot {
                    layout: MVP_DEVICE_LAYOUT,
                    profiles: vec![profile.clone()],
                    assignments: vec![SnapshotAssignment {
                        profile_id: profile.id,
                        control: ControlId::key(0, 0).unwrap(),
                        actions: vec![Action::OpenUrl {
                            url: SafeUrl::parse("https://example.com").unwrap(),
                        }],
                    }],
                },
            },
        );

        let ServerMessage::Snapshot {
            request_id,
            layout,
            profiles,
            assignments,
        } = response
        else {
            panic!("expected snapshot response");
        };

        assert_eq!(request_id, "req-snapshot");
        assert_eq!(layout.page_count, 4);
        assert_eq!(layout.key_rows, 3);
        assert_eq!(layout.key_columns, 4);
        assert_eq!(layout.encoder_count, 2);
        assert_eq!(profiles[0].id, profile.id.to_string());
        assert_eq!(assignments[0].profile_id, profile.id.to_string());
        assert_eq!(assignments[0].control, ControlDto::Key { page: 0, key: 0 });
        assert_eq!(assignments[0].actions.len(), 1);
    }

    #[test]
    fn encodes_profile_response() {
        let profile = keyro_core_domain::Profile {
            id: ProfileId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "Work".to_owned(),
            is_active: false,
        };

        let ServerMessage::Profile {
            request_id,
            profile: encoded,
        } = response_from_core(
            "req-create-profile".to_owned(),
            CoreResponse::Profile {
                profile: profile.clone(),
            },
        )
        else {
            panic!("expected profile response");
        };

        assert_eq!(request_id, "req-create-profile");
        assert_eq!(encoded.id, profile.id.to_string());
        assert_eq!(encoded.name, "Work");
        assert!(!encoded.is_active);
    }

    #[test]
    fn preserves_request_id_when_message_type_is_unknown() {
        let line = r#"{
            "request_id": "req-unknown",
            "message": {
                "type": "does_not_exist"
            }
        }"#;

        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            decode_protocol_dispatch(line, "core-test")
        else {
            panic!("expected immediate protocol error");
        };

        assert_eq!(request_id, Some("req-unknown".to_owned()));
        assert_eq!(error.code, ErrorCode::UnknownMessage);
    }

    #[test]
    fn rejects_protocol_indices_that_do_not_fit_domain_adapter() {
        let line = r#"{
            "request_id": "req-large-index",
            "message": {
                "type": "virtual_control_input",
                "control": {
                    "kind": "key",
                    "page": 300,
                    "key": 0
                }
            }
        }"#;

        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            decode_protocol_dispatch(line, "core-test")
        else {
            panic!("expected immediate validation error");
        };

        assert_eq!(request_id, Some("req-large-index".to_owned()));
        assert_eq!(error.code, ErrorCode::ValidationFailed);
    }

    #[test]
    fn session_accepts_handshake_before_commands() {
        let mut session = ProtocolSession::new("core-test");
        let handshake = include_str!("../../../protocol/test-vectors/v0.3.0/handshake.json");

        let ProtocolDispatch::Immediate(ServerMessage::HandshakeAccepted {
            request_id,
            core_version,
            ..
        }) = session.receive_line(handshake)
        else {
            panic!("expected handshake acceptance");
        };
        assert_eq!(request_id, "req-handshake");
        assert_eq!(core_version, "core-test");

        let command =
            include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");
        let ProtocolDispatch::Command {
            request_id,
            command,
        } = session.receive_line(command)
        else {
            panic!("expected routed command after handshake");
        };

        assert_eq!(request_id, "req-virtual-key-press");
        assert!(matches!(command, CoreCommand::VirtualControlInput { .. }));
    }

    #[test]
    fn session_rejects_commands_before_handshake() {
        let mut session = ProtocolSession::new("core-test");
        let command =
            include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");

        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            session.receive_line(command)
        else {
            panic!("expected immediate handshake-required error");
        };

        assert_eq!(request_id, Some("req-virtual-key-press".to_owned()));
        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("handshake required"));
    }

    #[test]
    fn incompatible_handshake_does_not_open_session() {
        let mut session = ProtocolSession::new("core-test");
        let handshake = r#"{
            "request_id": "req-handshake",
            "message": {
                "type": "handshake",
                "component": "studio",
                "component_version": "0.3.0",
                "protocol": {
                    "major": 0,
                    "minor": 4
                }
            }
        }"#;

        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            session.receive_line(handshake)
        else {
            panic!("expected incompatible protocol error");
        };
        assert_eq!(request_id, Some("req-handshake".to_owned()));
        assert_eq!(error.code, ErrorCode::IncompatibleProtocol);

        let command =
            include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");
        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            session.receive_line(command)
        else {
            panic!("expected command rejection after failed handshake");
        };

        assert_eq!(request_id, Some("req-virtual-key-press".to_owned()));
        assert_eq!(error.code, ErrorCode::ValidationFailed);
    }

    #[test]
    fn duplicate_handshake_is_rejected_without_closing_session() {
        let mut session = ProtocolSession::new("core-test");
        let handshake = include_str!("../../../protocol/test-vectors/v0.3.0/handshake.json");
        assert!(matches!(
            session.receive_line(handshake),
            ProtocolDispatch::Immediate(ServerMessage::HandshakeAccepted { .. })
        ));

        let incompatible_handshake = r#"{
            "request_id": "req-renegotiate",
            "message": {
                "type": "handshake",
                "component": "studio",
                "component_version": "0.3.0",
                "protocol": {
                    "major": 0,
                    "minor": 4
                }
            }
        }"#;
        let ProtocolDispatch::Immediate(ServerMessage::Error { request_id, error }) =
            session.receive_line(incompatible_handshake)
        else {
            panic!("expected duplicate handshake rejection");
        };
        assert_eq!(request_id, Some("req-renegotiate".to_owned()));
        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("already completed"));

        let command =
            include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");
        assert!(matches!(
            session.receive_line(command),
            ProtocolDispatch::Command { .. }
        ));
    }

    #[test]
    fn maps_application_failures_to_sanitized_protocol_errors() {
        let message = error_message(
            Some("req-storage".to_owned()),
            &AppError::Storage("database path /secret/keyro.sqlite failed".to_owned()),
        );

        let ServerMessage::Error { request_id, error } = message else {
            panic!("expected error message");
        };
        assert_eq!(request_id, Some("req-storage".to_owned()));
        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(error.message, "command failed");
    }

    #[test]
    fn maps_missing_profiles_to_not_found_protocol_errors() {
        let message = error_message(Some("req-missing".to_owned()), &AppError::ProfileNotFound);

        let ServerMessage::Error { request_id, error } = message else {
            panic!("expected error message");
        };
        assert_eq!(request_id, Some("req-missing".to_owned()));
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.message, "profile not found");
    }

    #[test]
    fn maps_core_action_events_to_protocol_events() {
        let control = ControlId::encoder(1, 0, EncoderOperation::Press).unwrap();
        let profile_id = ProfileId::new();

        let ServerMessage::ActionEvent {
            event:
                ActionEventDto::Running {
                    execution_id,
                    profile_id: encoded_profile_id,
                    control: encoded_control,
                },
        } = event_from_core(CoreEvent::ActionRunning {
            run_id: Default::default(),
            profile_id,
            control,
        })
        else {
            panic!("expected running action event");
        };

        assert!(!execution_id.is_empty());
        assert_eq!(encoded_profile_id, profile_id.to_string());
        assert_eq!(
            encoded_control,
            ControlDto::Encoder {
                page: 1,
                encoder: 0,
                operation: EncoderOperationDto::Press
            }
        );

        let ServerMessage::ActionEvent {
            event: ActionEventDto::Failed { code, message, .. },
        } = event_from_core(CoreEvent::ActionFailed {
            run_id: Default::default(),
            failure: ActionFailure::OpenUrlFailed,
            message: "open failed".to_owned(),
        })
        else {
            panic!("expected failed action event");
        };

        assert_eq!(code, ActionFailureCode::OpenUrlFailed);
        assert_eq!(message, "open failed");
    }
}
