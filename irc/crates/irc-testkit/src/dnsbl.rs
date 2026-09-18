//! DNSBL resolver abstraction.
//!
//! Production uses `hickory-resolver`; tests use [`StaticDnsblResolver`]
//! to seed deterministic listings without making real DNS queries.

use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;

/// Errors returned by a [`DnsblResolver`].
pub type DnsblError = Box<dyn Error + Send + Sync>;

/// DNSBL lookup contract.
///
/// `lookup` returns the matched zone name when `ip` is listed and `None`
/// otherwise. The trait is sync in Phase 0; the production resolver moves
/// to async alongside the tokio integration in Phase 6.
pub trait DnsblResolver: Send + Sync + 'static {
    /// Resolve a single IP against the configured zones.
    fn lookup(&self, ip: IpAddr) -> Result<Option<String>, DnsblError>;
}

/// Resolver that never lists any address.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopDnsblResolver;

impl DnsblResolver for NoopDnsblResolver {
    fn lookup(&self, _ip: IpAddr) -> Result<Option<String>, DnsblError> {
        Ok(None)
    }
}

/// Resolver backed by a static map for tests.
#[derive(Debug, Default, Clone)]
pub struct StaticDnsblResolver {
    listings: HashMap<IpAddr, String>,
}

impl StaticDnsblResolver {
    /// Construct an empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `ip` as listed in `zone`.
    #[must_use]
    pub fn with_listing(mut self, ip: IpAddr, zone: impl Into<String>) -> Self {
        self.listings.insert(ip, zone.into());
        self
    }
}

impl DnsblResolver for StaticDnsblResolver {
    fn lookup(&self, ip: IpAddr) -> Result<Option<String>, DnsblError> {
        Ok(self.listings.get(&ip).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::{DnsblResolver, NoopDnsblResolver, StaticDnsblResolver};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn noop_never_lists() {
        let r = NoopDnsblResolver;
        assert!(r.lookup(IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap().is_none());
    }

    #[test]
    fn static_resolver_returns_seeded_zone() {
        let bad = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 5));
        let good = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 6));
        let r = StaticDnsblResolver::new().with_listing(bad, "dnsbl.example.org");
        assert_eq!(r.lookup(bad).unwrap().as_deref(), Some("dnsbl.example.org"));
        assert!(r.lookup(good).unwrap().is_none());
    }
}
