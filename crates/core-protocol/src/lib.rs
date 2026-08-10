use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "0.2.0";
pub const PROTOCOL_MAJOR: u16 = 0;
pub const PROTOCOL_MINOR: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientEnvelope {
    pub request_id: String,
    pub message: ClientMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Handshake(HandshakeMessage),
    GetSnapshot(GetSnapshotMessage),
    ListProfiles(ListProfilesMessage),
    SetActiveProfile(SetActiveProfileMessage),
    SaveAssignment(SaveAssignmentMessage),
    VirtualControlInput(VirtualControlInputMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeMessage {
    pub component: ClientComponent,
    pub component_version: String,
    pub protocol: ProtocolVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetSnapshotMessage {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListProfilesMessage {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetActiveProfileMessage {
    pub profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAssignmentMessage {
    pub assignment: AssignmentDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualControlInputMessage {
    pub control: ControlDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientComponent {
    Studio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const fn current() -> Self {
        Self {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
        }
    }

    pub const fn is_exactly(self, other: Self) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentDto {
    pub profile_id: String,
    pub control: ControlDto,
    pub action: ActionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionDto {
    OpenUrl { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlDto {
    Key {
        page: u16,
        key: u16,
    },
    Encoder {
        page: u16,
        encoder: u16,
        operation: EncoderOperationDto,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderOperationDto {
    Left,
    Right,
    Press,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    HandshakeAccepted {
        request_id: String,
        core_version: String,
        protocol: ProtocolVersion,
    },
    Profiles {
        request_id: String,
        profiles: Vec<ProfileDto>,
    },
    Snapshot {
        request_id: String,
        layout: DeviceLayoutDto,
        profiles: Vec<ProfileDto>,
        assignments: Vec<SnapshotAssignmentDto>,
    },
    Acknowledged {
        request_id: String,
    },
    ActionEvent {
        event: ActionEventDto,
    },
    Error {
        request_id: Option<String>,
        error: ErrorDto,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDto {
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceLayoutDto {
    pub page_count: u16,
    pub key_rows: u16,
    pub key_columns: u16,
    pub encoder_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotAssignmentDto {
    pub profile_id: String,
    pub control: ControlDto,
    pub actions: Vec<ActionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ActionEventDto {
    Running {
        execution_id: String,
        profile_id: String,
        control: ControlDto,
    },
    Succeeded {
        execution_id: String,
    },
    Failed {
        execution_id: String,
        code: ActionFailureCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionFailureCode {
    OpenUrlFailed,
    NoActionAssigned,
    ValidationFailed,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorDto {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    IncompatibleProtocol,
    UnknownMessage,
    ValidationFailed,
    Internal,
}

pub fn decode_client_envelope(line: &str) -> Result<ClientEnvelope, ProtocolError> {
    let envelope: ClientEnvelope = serde_json::from_str(line).map_err(ProtocolError::Decode)?;
    if envelope.request_id.trim().is_empty() {
        return Err(ProtocolError::EmptyRequestId);
    }
    Ok(envelope)
}

pub fn encode_server_message(message: &ServerMessage) -> Result<String, ProtocolError> {
    serde_json::to_string(message).map_err(ProtocolError::Encode)
}

pub fn handshake_response(
    request_id: String,
    protocol: ProtocolVersion,
    core_version: String,
) -> ServerMessage {
    if protocol.is_exactly(ProtocolVersion::current()) {
        ServerMessage::HandshakeAccepted {
            request_id,
            core_version,
            protocol,
        }
    } else {
        ServerMessage::Error {
            request_id: Some(request_id),
            error: ErrorDto {
                code: ErrorCode::IncompatibleProtocol,
                message: "unsupported protocol version".to_owned(),
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("failed to decode client protocol envelope: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("failed to encode server protocol message: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("request_id cannot be empty")]
    EmptyRequestId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_virtual_control_test_vector() {
        let input =
            include_str!("../../../protocol/test-vectors/v0.2.0/virtual-control-input.json");

        let envelope = decode_client_envelope(input).unwrap();

        assert_eq!(envelope.request_id, "req-virtual-key-press");
        assert_eq!(
            envelope.message,
            ClientMessage::VirtualControlInput(VirtualControlInputMessage {
                control: ControlDto::Key { page: 0, key: 0 }
            })
        );
    }

    #[test]
    fn decodes_save_assignment_test_vector() {
        let input =
            include_str!("../../../protocol/test-vectors/v0.2.0/save-assignment-open-url.json");

        let envelope = decode_client_envelope(input).unwrap();

        assert_eq!(envelope.request_id, "req-save-assignment");
        assert!(matches!(envelope.message, ClientMessage::SaveAssignment(_)));
    }

    #[test]
    fn decodes_handshake_test_vector() {
        let input = include_str!("../../../protocol/test-vectors/v0.2.0/handshake.json");

        let envelope = decode_client_envelope(input).unwrap();

        assert_eq!(envelope.request_id, "req-handshake");
        assert_eq!(
            envelope.message,
            ClientMessage::Handshake(HandshakeMessage {
                component: ClientComponent::Studio,
                component_version: "0.2.0".to_owned(),
                protocol: ProtocolVersion::current(),
            })
        );
    }

    #[test]
    fn decodes_get_snapshot_test_vector() {
        let input = include_str!("../../../protocol/test-vectors/v0.2.0/get-snapshot.json");

        let envelope = decode_client_envelope(input).unwrap();

        assert_eq!(envelope.request_id, "req-snapshot");
        assert_eq!(
            envelope.message,
            ClientMessage::GetSnapshot(GetSnapshotMessage {})
        );
    }

    #[test]
    fn decodes_snapshot_response_test_vector() {
        let input = include_str!("../../../protocol/test-vectors/v0.2.0/snapshot-response.json");

        let message: ServerMessage = serde_json::from_str(input).unwrap();

        let ServerMessage::Snapshot {
            request_id,
            layout,
            profiles,
            assignments,
        } = message
        else {
            panic!("expected snapshot response");
        };
        assert_eq!(request_id, "req-snapshot");
        assert_eq!(layout.page_count, 4);
        assert_eq!(layout.key_rows, 3);
        assert_eq!(layout.key_columns, 4);
        assert_eq!(layout.encoder_count, 2);
        assert_eq!(profiles.len(), 1);
        assert_eq!(assignments.len(), 1);
    }

    #[test]
    fn encodes_structured_error_message() {
        let encoded = encode_server_message(&ServerMessage::Error {
            request_id: Some("req-error".to_owned()),
            error: ErrorDto {
                code: ErrorCode::UnknownMessage,
                message: "unknown message type".to_owned(),
            },
        })
        .unwrap();

        assert!(encoded.contains("\"type\":\"error\""));
        assert!(encoded.contains("\"code\":\"unknown_message\""));
    }

    #[test]
    fn rejects_unknown_fields() {
        let input = r#"{
            "request_id": "req-extra",
            "message": {
                "type": "list_profiles",
                "unexpected": true
            }
        }"#;

        assert!(decode_client_envelope(input).is_err());
    }

    #[test]
    fn rejects_empty_request_id() {
        let input = r#"{
            "request_id": "",
            "message": {
                "type": "list_profiles"
            }
        }"#;

        assert!(matches!(
            decode_client_envelope(input).unwrap_err(),
            ProtocolError::EmptyRequestId
        ));
    }

    #[test]
    fn protocol_v0_requires_exact_version_match() {
        assert!(ProtocolVersion { major: 0, minor: 2 }
            .is_exactly(ProtocolVersion { major: 0, minor: 2 }));
        assert!(!ProtocolVersion { major: 0, minor: 2 }
            .is_exactly(ProtocolVersion { major: 0, minor: 3 }));
    }
}
