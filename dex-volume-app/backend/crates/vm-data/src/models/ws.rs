use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsBroadcastGeneric<StreamType> {
    pub id: Option<i32>,
    pub message: Option<String>,
    pub r#type: String,
    pub data: StreamType,
}
