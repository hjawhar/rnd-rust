use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    pub ip: String,
}
