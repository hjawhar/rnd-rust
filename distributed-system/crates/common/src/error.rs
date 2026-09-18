use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("NATS error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("request failed: {0}")]
    Request(String),
}
