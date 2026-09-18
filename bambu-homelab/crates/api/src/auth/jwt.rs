use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub username: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
}

pub fn create_token(
    user_id: Uuid,
    username: &str,
    role: &str,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        role: role.to_string(),
        exp: now + 86400, // 24 hours
        iat: now,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub fn validate_token(
    token: &str,
    secret: &str,
) -> Result<Claims, jsonwebtoken::errors::Error> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
