export interface CommandRequest {
  command: string;
  params?: Record<string, unknown>;
}

export interface WsAuthMessage {
  type: 'auth';
  token: string;
}

export interface WsSubscribeMessage {
  type: 'subscribe';
  printer_ids: string[];
}

export interface WsUnsubscribeMessage {
  type: 'unsubscribe';
  printer_ids: string[];
}

export type WsClientMessage = WsAuthMessage | WsSubscribeMessage | WsUnsubscribeMessage;

export interface WsTelemetryMessage {
  type: 'telemetry';
  printer_id: string;
  data: import('./printer.model').TelemetrySnapshot;
}

export interface WsErrorMessage {
  type: 'error';
  message: string;
}

export interface WsAssignmentAddedMessage {
  type: 'assignment_added';
  printer_id: string;
  printer_name: string;
  printer_model: string;
}

export interface WsAssignmentRemovedMessage {
  type: 'assignment_removed';
  printer_id: string;
}

export type WsServerMessage =
  | WsTelemetryMessage
  | WsErrorMessage
  | WsAssignmentAddedMessage
  | WsAssignmentRemovedMessage;
