use tokio::sync::broadcast;

use crate::db::Database;

use super::server::Server;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub tx_broadcast: broadcast::Sender<(String, bool)>,
    pub x_api_key: String,
    pub rpc_endpoint: String,
    pub axsys_external_api_key: String,
    pub axsys_ws_endpoint: String,
    pub axsys_api_key: String,
    pub axsys_api_key_normal: String,
    pub server_name: String,
    pub servers: Vec<Server>,
    pub block_leaders: Vec<String>,
}

impl AppState {
    pub fn get_db(&self) -> &Database {
        return &self.db;
    }
}
