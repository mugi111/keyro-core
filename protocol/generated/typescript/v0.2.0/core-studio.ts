export const KEYRO_PROTOCOL_VERSION = "0.2.0" as const;

export const KEYRO_PROTOCOL_HANDSHAKE_VERSION = {
  major: 0,
  minor: 2
} as const;

export interface ProtocolVersion {
  major: number;
  minor: number;
}

export interface ClientEnvelope {
  request_id: string;
  message: ClientMessage;
}

export type ClientMessage =
  | HandshakeMessage
  | GetSnapshotMessage
  | ListProfilesMessage
  | SetActiveProfileMessage
  | SaveAssignmentMessage
  | VirtualControlInputMessage;

export interface HandshakeMessage {
  type: "handshake";
  component: "studio";
  component_version: string;
  protocol: ProtocolVersion;
}

export interface GetSnapshotMessage {
  type: "get_snapshot";
}

export interface ListProfilesMessage {
  type: "list_profiles";
}

export interface SetActiveProfileMessage {
  type: "set_active_profile";
  profile_id: string;
}

export interface SaveAssignmentMessage {
  type: "save_assignment";
  assignment: AssignmentDto;
}

export interface VirtualControlInputMessage {
  type: "virtual_control_input";
  control: ControlDto;
}

export interface AssignmentDto {
  profile_id: string;
  control: ControlDto;
  action: ActionDto;
}

export type ControlDto =
  | { kind: "key"; page: number; key: number }
  | {
      kind: "encoder";
      page: number;
      encoder: number;
      operation: EncoderOperationDto;
    };

export type EncoderOperationDto = "left" | "right" | "press";

export type ActionDto = {
  kind: "open_url";
  url: string;
};

export type ServerMessage =
  | HandshakeAcceptedMessage
  | ProfilesMessage
  | SnapshotMessage
  | AcknowledgedMessage
  | ActionEventMessage
  | ErrorMessage;

export interface HandshakeAcceptedMessage {
  type: "handshake_accepted";
  request_id: string;
  core_version: string;
  protocol: ProtocolVersion;
}

export interface ProfilesMessage {
  type: "profiles";
  request_id: string;
  profiles: ProfileDto[];
}

export interface SnapshotMessage {
  type: "snapshot";
  request_id: string;
  layout: DeviceLayoutDto;
  profiles: ProfileDto[];
  assignments: SnapshotAssignmentDto[];
}

export interface AcknowledgedMessage {
  type: "acknowledged";
  request_id: string;
}

export interface ActionEventMessage {
  type: "action_event";
  event: ActionEventDto;
}

export interface ErrorMessage {
  type: "error";
  request_id: string | null;
  error: ErrorDto;
}

export interface ProfileDto {
  id: string;
  name: string;
  is_active: boolean;
}

export interface DeviceLayoutDto {
  page_count: number;
  key_rows: number;
  key_columns: number;
  encoder_count: number;
}

export interface SnapshotAssignmentDto {
  profile_id: string;
  control: ControlDto;
  actions: ActionDto[];
}

export type ActionEventDto =
  | {
      state: "running";
      execution_id: string;
      profile_id: string;
      control: ControlDto;
    }
  | {
      state: "succeeded";
      execution_id: string;
    }
  | {
      state: "failed";
      execution_id: string;
      code: ActionFailureCode;
      message: string;
    };

export type ActionFailureCode =
  | "open_url_failed"
  | "no_action_assigned"
  | "validation_failed"
  | "internal";

export interface ErrorDto {
  code: ErrorCode;
  message: string;
}

export type ErrorCode =
  | "incompatible_protocol"
  | "unknown_message"
  | "validation_failed"
  | "internal";
