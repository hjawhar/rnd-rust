use thiserror::Error;

/// Shared error type for bambu-homelab services.
#[derive(Debug, Error)]
pub enum BambuError {
    #[error("NATS connection failed: {0}")]
    NatsConnect(#[source] async_nats::ConnectError),

    #[error("NATS subscription failed: {0}")]
    NatsSubscribe(#[source] async_nats::SubscribeError),

    #[error("NATS publish failed: {0}")]
    NatsPublish(#[source] async_nats::PublishError),

    #[error("protobuf decode failed: {0}")]
    ProtoDecode(#[from] prost::DecodeError),

    #[error("configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("{0}")]
    Internal(String),
}
