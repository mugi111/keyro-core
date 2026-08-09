use keyro_core_app::{CoreCommand, CoreResponse};
use keyro_core_domain::{Action, Assignment, ControlId, EncoderOperation, ProfileId, SafeUrl};
use serde::{Deserialize, Serialize};

/// Development-only local message envelope.
///
/// `keyro-protocol` is the future source of truth for wire contracts. This
/// type exists only so Core application behavior can be exercised before the
/// generated protocol crate and shared test vectors are available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevEnvelope {
    pub request_id: String,
    pub command: DevCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DevCommand {
    ListProfiles,
    SetActiveProfile { profile_id: String },
    SaveAssignment { assignment: DevAssignment },
    VirtualControlInput { control: DevControl },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevAssignment {
    pub profile_id: String,
    pub control: DevControl,
    pub action: DevAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DevAction {
    OpenUrl { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DevControl {
    Key {
        page: u8,
        key: u8,
    },
    Encoder {
        page: u8,
        encoder: u8,
        operation: DevEncoderOperation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevEncoderOperation {
    Left,
    Right,
    Press,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DevReply {
    Response {
        request_id: String,
        response: DevResponse,
    },
    Error {
        request_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DevResponse {
    Profiles { count: usize },
    Acknowledged,
}

pub fn decode_dev_command(line: &str) -> Result<(String, CoreCommand), IpcError> {
    let envelope: DevEnvelope = serde_json::from_str(line).map_err(IpcError::Decode)?;
    Ok((envelope.request_id, envelope.command.try_into()?))
}

pub fn encode_dev_reply(reply: &DevReply) -> Result<String, IpcError> {
    serde_json::to_string(reply).map_err(IpcError::Encode)
}

pub fn dev_response_from_core(response: CoreResponse) -> DevResponse {
    match response {
        CoreResponse::Profiles { profiles } => DevResponse::Profiles {
            count: profiles.len(),
        },
        CoreResponse::Acknowledged => DevResponse::Acknowledged,
    }
}

impl TryFrom<DevCommand> for CoreCommand {
    type Error = IpcError;

    fn try_from(command: DevCommand) -> Result<Self, Self::Error> {
        match command {
            DevCommand::ListProfiles => Ok(CoreCommand::ListProfiles),
            DevCommand::SetActiveProfile { profile_id } => Ok(CoreCommand::SetActiveProfile {
                profile_id: ProfileId::parse(&profile_id)?,
            }),
            DevCommand::SaveAssignment { assignment } => Ok(CoreCommand::SaveAssignment {
                assignment: assignment.try_into()?,
            }),
            DevCommand::VirtualControlInput { control } => Ok(CoreCommand::VirtualControlInput {
                control: control.try_into()?,
            }),
        }
    }
}

impl TryFrom<DevAssignment> for Assignment {
    type Error = IpcError;

    fn try_from(assignment: DevAssignment) -> Result<Self, Self::Error> {
        Assignment::single(
            ProfileId::parse(&assignment.profile_id)?,
            assignment.control.try_into()?,
            assignment.action.try_into()?,
        )
        .map_err(IpcError::Domain)
    }
}

impl TryFrom<DevAction> for Action {
    type Error = IpcError;

    fn try_from(action: DevAction) -> Result<Self, Self::Error> {
        match action {
            DevAction::OpenUrl { url } => Ok(Action::OpenUrl {
                url: SafeUrl::parse(url)?,
            }),
        }
    }
}

impl TryFrom<DevControl> for ControlId {
    type Error = IpcError;

    fn try_from(control: DevControl) -> Result<Self, Self::Error> {
        match control {
            DevControl::Key { page, key } => ControlId::key(page, key).map_err(IpcError::Domain),
            DevControl::Encoder {
                page,
                encoder,
                operation,
            } => ControlId::encoder(page, encoder, operation.into()).map_err(IpcError::Domain),
        }
    }
}

impl From<DevEncoderOperation> for EncoderOperation {
    fn from(operation: DevEncoderOperation) -> Self {
        match operation {
            DevEncoderOperation::Left => Self::Left,
            DevEncoderOperation::Right => Self::Right,
            DevEncoderOperation::Press => Self::Press,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("failed to decode development IPC envelope: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("failed to encode development IPC reply: {0}")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Domain(#[from] keyro_core_domain::DomainError),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_local_dev_command_without_claiming_protocol_compatibility() {
        let line = r#"{"request_id":"1","command":{"type":"virtual_control_input","control":{"kind":"key","page":0,"key":0}}}"#;

        let (request_id, command) = decode_dev_command(line).unwrap();

        assert_eq!(request_id, "1");
        assert_eq!(
            command,
            CoreCommand::VirtualControlInput {
                control: ControlId::key(0, 0).unwrap()
            }
        );
    }

    #[test]
    fn decodes_local_dev_assignment_command() {
        let profile_id = "550e8400-e29b-41d4-a716-446655440000";
        let line = format!(
            r#"{{
                "request_id":"2",
                "command":{{
                    "type":"save_assignment",
                    "assignment":{{
                        "profile_id":"{profile_id}",
                        "control":{{"kind":"encoder","page":0,"encoder":1,"operation":"right"}},
                        "action":{{"kind":"open_url","url":"https://example.com"}}
                    }}
                }}
            }}"#
        );

        let (request_id, command) = decode_dev_command(&line).unwrap();

        assert_eq!(request_id, "2");
        assert!(matches!(command, CoreCommand::SaveAssignment { .. }));
    }
}
