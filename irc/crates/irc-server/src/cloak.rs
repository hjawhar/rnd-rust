//! Host-cloaking engine.
//!
//! Replaces a user's real IP address with an HMAC-derived opaque
//! string so that IP addresses are never leaked in protocol messages
//! (WHOIS, JOIN prefixes, etc.).

use std::net::IpAddr;

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HMAC-based host cloaking engine.
#[derive(Debug)]
pub struct CloakEngine {
    secret: Vec<u8>,
}

impl CloakEngine {
    /// Create a new engine with the given HMAC secret.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
        }
    }

    /// Produce a deterministic cloaked hostname from an IP address.
    ///
    /// The output is three 8-char hex segments separated by dots,
    /// suffixed with `.IP`: `a1b2c3d4.e5f6a7b8.c9d0e1f2.IP`.
    #[must_use]
    pub fn cloak_ip(&self, ip: &IpAddr) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(ip.to_string().as_bytes());
        let result = mac.finalize().into_bytes();
        let hex = hex_encode(&result);
        // Take first 24 hex chars → three 8-char segments.
        format!("{}.{}.{}.IP", &hex[..8], &hex[8..16], &hex[16..24],)
    }

    /// Produce a cloaked hostname for an authenticated account.
    ///
    /// Account-based cloaks override IP-based cloaks once the user
    /// has been verified (`SASL`, `NickServ`, etc.).
    #[must_use]
    pub fn cloak_account(account: &str) -> String {
        format!("user/{account}")
    }
}

/// Lower-case hex-encode the first 12 bytes (24 hex chars is all we need).
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn deterministic_cloak() {
        let engine = CloakEngine::new(b"test-secret");
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let a = engine.cloak_ip(&ip);
        let b = engine.cloak_ip(&ip);
        assert_eq!(a, b, "same IP + same secret must produce identical cloak");
    }

    #[test]
    fn different_ips_different_cloaks() {
        let engine = CloakEngine::new(b"test-secret");
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert_ne!(engine.cloak_ip(&ip1), engine.cloak_ip(&ip2));
    }

    #[test]
    fn cloak_format_three_segments_dot_ip() {
        let engine = CloakEngine::new(b"secret");
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let cloak = engine.cloak_ip(&ip);
        let parts: Vec<&str> = cloak.split('.').collect();
        assert_eq!(parts.len(), 4, "expected 3 segments + IP suffix");
        assert_eq!(parts[3], "IP");
        for seg in &parts[..3] {
            assert_eq!(seg.len(), 8, "each segment must be 8 hex chars");
            assert!(
                seg.chars().all(|c| c.is_ascii_hexdigit()),
                "segment must be hex"
            );
        }
    }

    #[test]
    fn ipv6_works() {
        let engine = CloakEngine::new(b"v6secret");
        let ip = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let cloak = engine.cloak_ip(&ip);
        assert!(
            std::path::Path::new(&cloak)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ip"))
        );
        assert_eq!(cloak.split('.').count(), 4);
    }

    #[test]
    fn account_cloak_format() {
        assert_eq!(CloakEngine::cloak_account("alice"), "user/alice");
        assert_eq!(CloakEngine::cloak_account("bob123"), "user/bob123");
    }
}
