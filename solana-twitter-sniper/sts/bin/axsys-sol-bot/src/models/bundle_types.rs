use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BundleType {
    JITO,
    NEXTBLOCK,
    BLOXROUTE,
    #[allow(non_camel_case_types)]
    ZERO_SLOT
}
