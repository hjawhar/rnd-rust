use jsonwebtoken::{DecodingKey, EncodingKey};
use ring::signature::{Ed25519KeyPair, KeyPair};

pub struct Keys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
}

impl Keys {
    pub fn new(secret: &[u8]) -> Self {
        let encoding_key = EncodingKey::from_ed_der(secret);
        let pair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(secret).unwrap();
        let decoding_key = DecodingKey::from_ed_der(pair.public_key().as_ref());

        Self {
            encoding: encoding_key,
            decoding: decoding_key,
        }
    }
}
