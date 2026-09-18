use serde::{Deserialize, Serialize};

/// Stored printer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterConfig {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub serial: String,
    pub access_code: String,
    pub model: String,
}

/// Request to register a printer.
#[derive(Debug, Deserialize)]
pub struct AddPrinterRequest {
    pub ip: String,
    pub serial: String,
    pub access_code: String,
    pub name: String,
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_model() -> String {
    "X1C".to_string()
}

/// Printer with current status.
#[derive(Debug, Serialize)]
pub struct PrinterWithStatus {
    #[serde(flatten)]
    pub config: PrinterConfig,
    pub online: bool,
    pub status: Option<serde_json::Value>,
}

/// Command request.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandRequest {
    pub command: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// WebSocket message from client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "subscribe")]
    Subscribe { printer_ids: Vec<String> },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { printer_ids: Vec<String> },
}

/// WebSocket message to client.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    #[serde(rename = "telemetry")]
    Telemetry {
        printer_id: String,
        data: serde_json::Value,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "assignment_added")]
    AssignmentAdded {
        printer_id: String,
        printer_name: String,
        printer_model: String,
        #[serde(skip)]
        target_user_id: uuid::Uuid,
    },
    #[serde(rename = "assignment_removed")]
    AssignmentRemoved {
        printer_id: String,
        #[serde(skip)]
        target_user_id: uuid::Uuid,
    },
}


/// Login request.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub role: String,
}

/// Change password request.
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Request to assign a user to a printer.
#[derive(Debug, Deserialize)]
pub struct AssignPrinterRequest {
    pub user_id: String,
}

/// Assignment info returned from API.
#[derive(Debug, Serialize)]
pub struct AssignmentInfo {
    pub user_id: String,
    pub username: String,
    pub assigned_at: String,
}