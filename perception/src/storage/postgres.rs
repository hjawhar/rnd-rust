//! Optional PostgreSQL sync (requires `postgres` feature).

#[cfg(feature = "postgres")]
pub struct PostgresSync { /* TODO */ }

#[cfg(feature = "postgres")]
impl PostgresSync {
    /// Connect to PostgreSQL and set up periodic sync.
    pub async fn new(_url: &str, _interval_secs: u64) -> crate::error::Result<Self> {
        todo!("PostgresSync not yet implemented")
    }
}
