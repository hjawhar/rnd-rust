use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrCondifence {
    pub text: String,
    pub confidence: f64,
    pub image: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OcrCondifenceEnum {
    String(String),
    Number(f64),
}