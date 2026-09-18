use alloy::hex;
use once_cell::sync::Lazy;

use crate::models::claims::Keys;

pub static KEYS: Lazy<Keys> = Lazy::new(|| {
    let key = std::env::var("ED25519_KEY").unwrap();
    let bytes = hex::decode(key).unwrap();
    let slice_bytes = &bytes[..];
    Keys::new(slice_bytes)
});
