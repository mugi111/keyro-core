use keyro_core_app::{CoreCommand, CoreResponse};
use keyro_core_domain::{Action, Assignment, ControlId, EncoderOperation, ProfileId, SafeUrl};
use keyro_core_protocol::{
    decode_client_envelope, encode_server_message, handshake_response, ActionDto, AssignmentDto,
    ClientMessage, ControlDto, EncoderOperationDto, ErrorCode, ErrorDto, ProtocolError,
    ServerMessage,
};
use serde_json::Value;

pub enum ProtocolDispatch {
    Immediate(ServerMessage),
    Command {
        request_id: String,
        command: CoreCommand,
    },
}

pub fn decode_protocol_dispatch(line: &str, core_version: &str) -> ProtocolDispatch {
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

pub fn decode_protocol_command(line: &str) -> Result<(String, CoreCommand), IpcError> {
    let envelope = decode_client_envelope(line).map_err(IpcError::Protocol)?;
    Ok((envelope.request_id, command_from_message(envelope.message)?))
}

pub fn encode_protocol_message(message: &ServerMessage) -> Result<String, IpcError> {
    encode_server_message(message).map_err(IpcError::Protocol)
}

pub fn response_from_core(request_id: String, response: CoreResponse) -> ServerMessage {
    match response {
        CoreResponse::Profiles { profiles } => ServerMessage::Profiles {
            request_id,
            profiles: profiles
                .into_iter()
                .map(|profile| keyro_core_protocol::ProfileDto {
                    id: profile.id.to_string(),
                    name: profile.name,
                    is_active: profile.is_active,
                })
                .collect(),
        },
        CoreResponse::Acknowledged => ServerMessage::Acknowledged { request_id },
    }
}

pub fn error_message(request_id: Option<String>, error: impl std::fmt::Display) -> ServerMessage {
    ServerMessage::Error {
        request_id,
        error: ErrorDto {
            code: ErrorCode::ValidationFailed,
            message: error.to_string(),
        },
    }
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
        ClientMessage::ListProfiles(_) => Ok(CoreCommand::ListProfiles),
        ClientMessage::SetActiveProfile(message) => Ok(CoreCommand::SetActiveProfile {
            profile_id: ProfileId::parse(&message.profile_id)?,
        }),
        ClientMessage::SaveAssignment(message) => Ok(CoreCommand::SaveAssignment {
            assignment: assignment_from_dto(message.assignment)?,
        }),
        ClientMessage::VirtualControlInput(message) => Ok(CoreCommand::VirtualControlInput {
            control: control_from_dto(message.control)?,
        }),
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

    #[test]
    fn decodes_protocol_virtual_control_command() {
        let line = include_str!("../../../protocol/test-vectors/v0.1.0/virtual-control-input.json");

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
            include_str!("../../../protocol/test-vectors/v0.1.0/save-assignment-open-url.json");

        let (request_id, command) = decode_protocol_command(line).unwrap();

        assert_eq!(request_id, "req-save-assignment");
        assert!(matches!(command, CoreCommand::SaveAssignment { .. }));
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
            decode_protocol_dispatch(line, "0.1.0")
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
            decode_protocol_dispatch(line, "0.1.0")
        else {
            panic!("expected immediate validation error");
        };

        assert_eq!(request_id, Some("req-large-index".to_owned()));
        assert_eq!(error.code, ErrorCode::ValidationFailed);
    }
}
