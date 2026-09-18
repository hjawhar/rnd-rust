use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub id: String,
    pub name: String,
}
