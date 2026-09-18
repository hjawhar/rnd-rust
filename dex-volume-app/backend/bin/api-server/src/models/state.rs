use std::sync::Arc;
use vm_data::db::Database;
use vm_data::models::ws::WsBroadcastGeneric;

use super::ws_client_manager::WsClientManager;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub read_db: Option<Database>,
    pub ws_clients: Arc<WsClientManager>,
    pub nats_client: async_nats::Client,
}

impl AppState {
    pub fn get_db(&self) -> &Database {
        &self.db
    }

    /// Returns the read replica DB if configured, otherwise the primary.
    pub fn read_db(&self) -> &Database {
        self.read_db.as_ref().unwrap_or(&self.db)
    }

    /// Serialize a WsBroadcastGeneric once, then route to the appropriate clients.
    /// `project_id`: None = broadcast to all, Some(id) = only clients authorized for that project.
    pub async fn broadcast<T: serde::Serialize>(&self, payload: &WsBroadcastGeneric<T>, project_id: Option<i32>) {
        if let Ok(json) = serde_json::to_string(payload) {
            let json = Arc::new(json);
            match project_id {
                Some(pid) => self.ws_clients.send_to_project(pid, json).await,
                None => self.ws_clients.broadcast_all(json).await,
            }
        }
    }

    /// Send a payload to all connections of a specific user (e.g. initial price on WS connect).
    pub async fn send_to_user<T: serde::Serialize>(&self, payload: &WsBroadcastGeneric<T>, user_id: i32) {
        if let Ok(json) = serde_json::to_string(payload) {
            self.ws_clients.send_to_user(user_id, Arc::new(json)).await;
        }
    }
}
