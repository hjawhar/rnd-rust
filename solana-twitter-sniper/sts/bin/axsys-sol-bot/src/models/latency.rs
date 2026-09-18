use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Latency {
    pub ping: i128,
    pub pong: i128,
}
