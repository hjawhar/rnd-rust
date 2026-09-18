use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::db;
use crate::models::WsServerMessage;

#[derive(Clone)]
pub struct AppState {
    pub db: db::Pool,
    pub jwt_secret: String,
    pub telemetry_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    pub online_status: Arc<RwLock<HashMap<String, std::time::Instant>>>,
    pub ws_broadcast: broadcast::Sender<WsServerMessage>,
    pub nats: async_nats::Client,
}
