//! Persistence layer: async [`Store`] trait and concrete backends.
//!
//! [`SqliteStore`] wraps a [`sqlx::SqlitePool`] for production use.
//! [`InMemoryStore`] backs every field with a `Vec` behind a
//! [`parking_lot::RwLock`] so unit tests never touch SQLite.

use parking_lot::RwLock;
use thiserror::Error;

/// A row from the `accounts` table.
#[derive(Debug, Clone)]
pub struct AccountRow {
    /// Unique account name (case-preserved).
    pub name: String,
    /// Email address associated with the account.
    pub email: String,
    /// Argon2 password hash.
    pub password_hash: String,
    /// Whether the account has been verified via token.
    pub verified: bool,
    /// Optional TLS client certificate fingerprint.
    pub cert_fingerprint: Option<String>,
}

/// Errors returned by [`Store`] implementations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// An error from the SQLite driver.
    #[error("sqlite: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// The requested account already exists.
    #[error("account already exists")]
    AlreadyExists,
}

/// Async persistence interface for account data.
///
/// Implementations must be cheaply cloneable (wrap pools / `Arc`s).
pub trait Store: Send + Sync + 'static {
    /// Insert a new account in unverified state.
    fn account_create(
        &self,
        name: &str,
        email: &str,
        password_hash: &str,
        verify_token: &str,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// Verify an account by matching the stored token. Returns `true`
    /// if the token matched and the account was marked verified.
    fn account_verify(
        &self,
        name: &str,
        token: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// Look up an account by name.
    fn account_get(
        &self,
        name: &str,
    ) -> impl std::future::Future<Output = Result<Option<AccountRow>, StoreError>> + Send;

    /// Look up an account by TLS client certificate fingerprint.
    fn account_by_fingerprint(
        &self,
        fp: &str,
    ) -> impl std::future::Future<Output = Result<Option<AccountRow>, StoreError>> + Send;

    /// Health check / connection probe.
    fn ping(&self) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

/// Production store backed by a [`sqlx::SqlitePool`].
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: sqlx::SqlitePool,
}

impl SqliteStore {
    /// Wrap an already-connected pool.
    #[must_use]
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }
}

impl Store for SqliteStore {
    async fn account_create(
        &self,
        name: &str,
        email: &str,
        password_hash: &str,
        verify_token: &str,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "INSERT INTO accounts (name, email, password_hash, verify_token) VALUES (?, ?, ?, ?)",
        )
        .bind(name)
        .bind(email)
        .bind(password_hash)
        .bind(verify_token)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(ref e)) if e.is_unique_violation() => {
                Err(StoreError::AlreadyExists)
            }
            Err(e) => Err(StoreError::Sqlx(e)),
        }
    }

    async fn account_verify(&self, name: &str, token: &str) -> Result<bool, StoreError> {
        let rows = sqlx::query(
            "UPDATE accounts SET verified = 1, verify_token = NULL \
             WHERE name = ? AND verify_token = ? AND verified = 0",
        )
        .bind(name)
        .bind(token)
        .execute(&self.pool)
        .await?;
        Ok(rows.rows_affected() > 0)
    }

    async fn account_get(&self, name: &str) -> Result<Option<AccountRow>, StoreError> {
        let row = sqlx::query_as::<_, (String, String, String, bool, Option<String>)>(
            "SELECT name, email, password_hash, verified, cert_fingerprint \
             FROM accounts WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(name, email, password_hash, verified, cert_fingerprint)| AccountRow {
                name,
                email,
                password_hash,
                verified,
                cert_fingerprint,
            },
        ))
    }

    async fn account_by_fingerprint(&self, fp: &str) -> Result<Option<AccountRow>, StoreError> {
        let row = sqlx::query_as::<_, (String, String, String, bool, Option<String>)>(
            "SELECT name, email, password_hash, verified, cert_fingerprint \
             FROM accounts WHERE cert_fingerprint = ?",
        )
        .bind(fp)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(name, email, password_hash, verified, cert_fingerprint)| AccountRow {
                name,
                email,
                password_hash,
                verified,
                cert_fingerprint,
            },
        ))
    }

    async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// In-memory backend (tests)
// ---------------------------------------------------------------------------

/// Row with token included, for in-memory verification.
#[derive(Debug, Clone)]
struct InMemoryRow {
    row: AccountRow,
    verify_token: Option<String>,
}

/// In-memory store for tests. No SQLite required.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    accounts: RwLock<Vec<InMemoryRow>>,
}

impl InMemoryStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for InMemoryStore {
    async fn account_create(
        &self,
        name: &str,
        email: &str,
        password_hash: &str,
        verify_token: &str,
    ) -> Result<(), StoreError> {
        let mut accounts = self.accounts.write();
        if accounts.iter().any(|a| a.row.name == name) {
            return Err(StoreError::AlreadyExists);
        }
        accounts.push(InMemoryRow {
            row: AccountRow {
                name: name.to_owned(),
                email: email.to_owned(),
                password_hash: password_hash.to_owned(),
                verified: false,
                cert_fingerprint: None,
            },
            verify_token: Some(verify_token.to_owned()),
        });
        drop(accounts);
        Ok(())
    }

    async fn account_verify(&self, name: &str, token: &str) -> Result<bool, StoreError> {
        let mut accounts = self.accounts.write();
        for entry in &mut *accounts {
            if entry.row.name == name
                && !entry.row.verified
                && entry.verify_token.as_deref() == Some(token)
            {
                entry.row.verified = true;
                entry.verify_token = None;
                return Ok(true);
            }
        }
        drop(accounts);
        Ok(false)
    }

    async fn account_get(&self, name: &str) -> Result<Option<AccountRow>, StoreError> {
        let accounts = self.accounts.read();
        Ok(accounts
            .iter()
            .find(|a| a.row.name == name)
            .map(|a| a.row.clone()))
    }

    async fn account_by_fingerprint(&self, fp: &str) -> Result<Option<AccountRow>, StoreError> {
        let accounts = self.accounts.read();
        Ok(accounts
            .iter()
            .find(|a| a.row.cert_fingerprint.as_deref() == Some(fp))
            .map(|a| a.row.clone()))
    }

    async fn ping(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Type-erased dispatch enum
// ---------------------------------------------------------------------------

/// Concrete enum wrapping all supported store backends.
///
/// This avoids the need for `dyn Store` (which is not object-safe with
/// native async traits). [`ServerState`](crate::state::ServerState) holds
/// an `Arc<AnyStore>`.
#[derive(Debug)]
pub enum AnyStore {
    /// SQLite-backed production store.
    Sqlite(SqliteStore),
    /// In-memory store for tests.
    InMemory(InMemoryStore),
}

impl Store for AnyStore {
    async fn account_create(
        &self,
        name: &str,
        email: &str,
        password_hash: &str,
        verify_token: &str,
    ) -> Result<(), StoreError> {
        match self {
            Self::Sqlite(s) => {
                s.account_create(name, email, password_hash, verify_token)
                    .await
            }
            Self::InMemory(s) => {
                s.account_create(name, email, password_hash, verify_token)
                    .await
            }
        }
    }

    async fn account_verify(&self, name: &str, token: &str) -> Result<bool, StoreError> {
        match self {
            Self::Sqlite(s) => s.account_verify(name, token).await,
            Self::InMemory(s) => s.account_verify(name, token).await,
        }
    }

    async fn account_get(&self, name: &str) -> Result<Option<AccountRow>, StoreError> {
        match self {
            Self::Sqlite(s) => s.account_get(name).await,
            Self::InMemory(s) => s.account_get(name).await,
        }
    }

    async fn account_by_fingerprint(&self, fp: &str) -> Result<Option<AccountRow>, StoreError> {
        match self {
            Self::Sqlite(s) => s.account_by_fingerprint(fp).await,
            Self::InMemory(s) => s.account_by_fingerprint(fp).await,
        }
    }

    async fn ping(&self) -> Result<(), StoreError> {
        match self {
            Self::Sqlite(s) => s.ping().await,
            Self::InMemory(s) => s.ping().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemoryStore, Store};

    #[tokio::test]
    async fn in_memory_ping() {
        let store = InMemoryStore::new();
        store.ping().await.unwrap();
    }

    #[tokio::test]
    async fn create_and_get() {
        let store = InMemoryStore::new();
        store
            .account_create("alice", "a@b.c", "hash", "tok123")
            .await
            .unwrap();
        let acct = store.account_get("alice").await.unwrap().unwrap();
        assert_eq!(acct.name, "alice");
        assert!(!acct.verified);
    }

    #[tokio::test]
    async fn duplicate_name_rejected() {
        let store = InMemoryStore::new();
        store
            .account_create("alice", "a@b.c", "hash", "tok")
            .await
            .unwrap();
        let err = store
            .account_create("alice", "x@y.z", "hash2", "tok2")
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn verify_with_correct_token() {
        let store = InMemoryStore::new();
        store
            .account_create("bob", "b@b.c", "hash", "secret")
            .await
            .unwrap();
        assert!(store.account_verify("bob", "secret").await.unwrap());
        let acct = store.account_get("bob").await.unwrap().unwrap();
        assert!(acct.verified);
    }

    #[tokio::test]
    async fn verify_with_wrong_token() {
        let store = InMemoryStore::new();
        store
            .account_create("bob", "b@b.c", "hash", "secret")
            .await
            .unwrap();
        assert!(!store.account_verify("bob", "wrong").await.unwrap());
    }
}
