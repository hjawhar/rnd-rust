use serde::{Deserialize, Serialize};

use super::{bundle_types::BundleType, buy::BuyRequest, mpsc::LogsType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorTx {
    pub logs_type: LogsType,
    pub uuid: String,
    pub signature: String,
    pub bundle_hash: String,
    pub confirmed: bool,
    pub success: bool,
    pub checked: bool,
    pub slot: Option<u64>,
    pub bundle_type: BundleType,
    pub task: Option<BuyRequest>,
    pub timestamp: u128,
    pub timelapsed: u128
}
