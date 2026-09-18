//! Account utilities: password hashing, token generation.

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

/// Hash a password with Argon2id and a random salt.
///
/// # Errors
///
/// Returns an error if the underlying hasher fails (should not happen
/// with default parameters and a valid salt).
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    // Generate 16 random bytes, encode as base64 for a valid salt.
    // We avoid `SaltString::generate` because `argon2` re-exports `rand_core 0.6`
    // which is incompatible with `rand 0.9`'s `rand_core 0.9`.
    use base64::Engine;
    use rand::Rng;
    let raw: [u8; 16] = rand::rng().random();
    let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(raw);
    let salt = SaltString::from_b64(&encoded)?;
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against an Argon2 hash string.
///
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, `Err` if the
/// hash string is malformed.
pub fn verify_password(hash: &str, password: &str) -> Result<bool, argon2::password_hash::Error> {
    let parsed = PasswordHash::new(hash)?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Generate a random 32-character hex verification token.
#[must_use]
pub fn generate_verify_token() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::rng().random();
    bytes.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_verify_roundtrip() {
        let password = "hunter2";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(&hash, password).unwrap());
        assert!(!verify_password(&hash, "wrong").unwrap());
    }

    #[test]
    fn verify_token_length() {
        let token = generate_verify_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
