use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsBroadcastGeneric<T> {
    pub server: String,
    pub r#type: String,
    pub data: T,
}
