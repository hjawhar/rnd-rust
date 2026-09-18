//! Server-operator configuration types and privilege model.

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use serde::Deserialize;

/// Privilege a server operator may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Privilege {
    /// Add/remove K-lines.
    Kline,
    /// Forcibly disconnect a user.
    Kill,
    /// Set modes on any channel.
    Samode,
    /// Reload the configuration.
    Rehash,
    /// Shut down the server.
    Die,
    /// View a user's real host behind a cloak.
    SeeRealhost,
    /// Prevent new connections.
    Lockdown,
    /// Override a user's cloak.
    Setcloak,
    /// Register accounts without email verification.
    RegisterBypass,
}

/// A configured oper block.
#[derive(Debug, Clone, Deserialize)]
pub struct OperBlock {
    /// Unique oper name.
    pub name: String,
    /// Argon2 password hash.
    pub password_hash: String,
    /// If set, the user must be logged into this account.
    #[serde(default)]
    pub require_account: Option<String>,
    /// Host masks the oper may connect from.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Name of the oper class granting privileges.
    pub class: String,
}

/// A configured oper class with its privilege set.
#[derive(Debug, Clone, Deserialize)]
pub struct OperClass {
    /// Privileges granted to opers of this class.
    pub privileges: Vec<Privilege>,
}

/// Verify an oper password against the stored argon2 hash.
pub fn verify_oper_password(block: &OperBlock, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(&block.password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Simple glob match: `*` at start/end of each component.
///
/// Splits `mask` on `@`, takes the host part, and compares against
/// `host`. If there is no `@`, treats the whole mask as a host pattern.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat = pattern.to_ascii_lowercase();
    let val = text.to_ascii_lowercase();

    if pat == val {
        return true;
    }
    if pat == "*" {
        return true;
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return val.starts_with(prefix);
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return val.ends_with(suffix);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_password_accepts_correct() {
        use argon2::password_hash::SaltString;
        use argon2::{Argon2, PasswordHasher};
        use base64::Engine;
        use rand::Rng;

        let raw: [u8; 16] = rand::rng().random();
        let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(raw);
        let salt = SaltString::from_b64(&encoded).unwrap();
        let hash = Argon2::default()
            .hash_password(b"hunter2", &salt)
            .unwrap()
            .to_string();

        let block = OperBlock {
            name: "admin".into(),
            password_hash: hash,
            require_account: None,
            allowed_hosts: vec![],
            class: "netadmin".into(),
        };

        assert!(verify_oper_password(&block, "hunter2"));
        assert!(!verify_oper_password(&block, "wrong"));
    }

    #[test]
    fn verify_password_rejects_bad_hash() {
        let block = OperBlock {
            name: "x".into(),
            password_hash: "not-a-hash".into(),
            require_account: None,
            allowed_hosts: vec![],
            class: "x".into(),
        };
        assert!(!verify_oper_password(&block, "anything"));
    }

    #[test]
    fn glob_match_works() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("192.168.*", "192.168.1.1"));
        assert!(!glob_match("192.168.*", "10.0.0.1"));
        assert!(glob_match("*.example.com", "foo.example.com"));
        assert!(!glob_match("*.example.com", "foo.example.org"));
        assert!(glob_match("exact.host", "exact.host"));
        assert!(!glob_match("exact.host", "other.host"));
        // case insensitive
        assert!(glob_match("*.Example.COM", "foo.example.com"));
    }
}
