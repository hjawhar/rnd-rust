//! TLS configuration loader for IRC listeners.

use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::error::{ConfigError, ServerError};

/// Load a TLS [`ServerConfig`] from PEM-encoded certificate chain and
/// private key files.
///
/// Returns a fully-configured `ServerConfig` with safe defaults and no
/// client authentication.
///
/// # Errors
///
/// Returns [`ServerError::Config`] if the files cannot be read, contain
/// no valid PEM items, or if `rustls` rejects the resulting material.
pub fn load_tls_config(
    cert_path: &Path,
    key_path: &Path,
) -> Result<Arc<ServerConfig>, ServerError> {
    let cert_chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| {
            ConfigError::Invalid(format!(
                "cannot read TLS cert at {}: {e}",
                cert_path.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ConfigError::Invalid(format!("invalid PEM cert chain: {e}")))?;

    if cert_chain.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "no certificates found in {}",
            cert_path.display()
        ))
        .into());
    }

    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|e| {
        ConfigError::Invalid(format!(
            "cannot read TLS key at {}: {e}",
            key_path.display()
        ))
    })?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .map_err(|e| ConfigError::Invalid(format!("TLS config rejected: {e}")))?;

    Ok(Arc::new(config))
}
