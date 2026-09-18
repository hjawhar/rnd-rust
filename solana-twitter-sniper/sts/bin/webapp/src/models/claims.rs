use jsonwebtoken::{DecodingKey, EncodingKey};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};

pub struct Keys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
}

impl Keys {
    pub fn new(secret: &[u8]) -> Self {
        let encoding_key = EncodingKey::from_ed_der(secret);
        let pair = Ed25519KeyPair::from_pkcs8(secret).unwrap();
        let decoding_key = DecodingKey::from_ed_der(pair.public_key().as_ref());

        Self {
            encoding: encoding_key,
            decoding: decoding_key,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub exp: u64,
    pub sub: String,
    pub company: String,
    pub id: i32,
    pub address: String,
    pub group_id: i32,
    pub whitelisted: bool,
}

#[derive(Debug, Serialize)]
pub struct AuthBody {
    pub access_token: String,
    pub token_type: String,
}

#[derive(Debug)]
pub enum AuthError {
    WrongCredentials,
    MissingCredentials,
    TokenCreation,
    InvalidToken,
}
