use dotenv::dotenv;
use std::sync::OnceLock;

use ring::aead::AES_256_GCM;
use ring::aead::Aad;
use ring::aead::BoundKey;
use ring::aead::NONCE_LEN;
use ring::aead::Nonce;
use ring::aead::NonceSequence;
use ring::aead::OpeningKey;
use ring::aead::SealingKey;
use ring::aead::UnboundKey;
use ring::error::Unspecified;
use ring::rand::SecureRandom;
use ring::rand::SystemRandom;
use ring::signature::Ed25519KeyPair;

/// AES key bytes loaded once at startup — avoids env var read + hex decode per call.
static AES_KEY_BYTES: OnceLock<Vec<u8>> = OnceLock::new();

fn get_aes_key_bytes() -> &'static [u8] {
    AES_KEY_BYTES.get_or_init(|| {
        dotenv().ok();
        let key = std::env::var("AES256_GCM_KEY").expect("AES256_GCM_KEY is required");
        hex::decode(key).expect("AES256_GCM_KEY must be valid hex")
    })
}

struct CounterNonceSequence(u32);

impl NonceSequence for CounterNonceSequence {
    // called once for each seal operation
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        let mut nonce_bytes = vec![0; NONCE_LEN];

        let bytes = self.0.to_be_bytes();
        nonce_bytes[8..].copy_from_slice(&bytes);

        self.0 += 1; // advance the counter
        Nonce::try_assume_unique_for_key(&nonce_bytes)
    }
}

pub fn encrypt(input: String) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let key_bytes = get_aes_key_bytes();
    let data = input.as_bytes().to_vec();
    let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| "encryption: invalid key")?;
    let nonce_sequence = CounterNonceSequence(1);
    let mut sealing_key = SealingKey::new(unbound_key, nonce_sequence);
    let associated_data = Aad::empty();
    let mut in_out = data.clone();
    let tag = sealing_key.seal_in_place_separate_tag(associated_data, &mut in_out).map_err(|_| "encryption: seal failed")?;
    let final_hash = format!("{}{}", hex::encode(in_out), hex::encode(tag.as_ref()));

    Ok(final_hash)
}

pub fn decrypt(input: String) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let key_bytes = get_aes_key_bytes();
    let in_out = &hex::decode(input).unwrap()[..];
    let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| "decryption: invalid key")?;
    let nonce_sequence = CounterNonceSequence(1);
    let associated_data = Aad::empty();
    let mut opening_key = OpeningKey::new(unbound_key, nonce_sequence);
    let mut cypher_text_with_tag = [in_out].concat();
    let decrypted_data = opening_key.open_in_place(associated_data, &mut cypher_text_with_tag).map_err(|_| "decryption: open failed")?;
    Ok(String::from_utf8(decrypted_data.to_vec()).unwrap())
}

pub fn generate_signing_pk() -> String {
    let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let aa = doc.as_ref();
    hex::encode(aa)
}

pub fn generate_encryption_key() -> Result<String, Unspecified> {
    let rand = SystemRandom::new();
    let mut key_bytes = vec![0; AES_256_GCM.key_len()];
    rand.fill(&mut key_bytes)?;
    Ok(hex::encode(&key_bytes))
}

pub fn _test_encryption() -> Result<(), Unspecified> {
    let key_bytes = get_aes_key_bytes();
    let data = b"tel7as tize";
    {
        let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes)?;
        let nonce_sequence = CounterNonceSequence(1);
        let mut sealing_key = SealingKey::new(unbound_key, nonce_sequence);
        let associated_data = Aad::empty();

        println!("data = {}", String::from_utf8(data.to_vec()).unwrap());
        println!("init data {:?}", data);
        println!("init data length {}", data.len());

        let mut in_out = *data;
        let tag = sealing_key.seal_in_place_separate_tag(associated_data, &mut in_out)?;
        let final_hash = format!(
            "{}{}",
            hex::encode(in_out.as_ref()),
            hex::encode(tag.as_ref())
        );
        println!("{}", final_hash);
    }
    println!("-----------------");
    {
        let in_out =
            &hex::decode("000b5da730250adc55253483e917c1e1c2ca4b5d6d91b210d5f4e2").unwrap()[..];

        let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes)?;
        let nonce_sequence = CounterNonceSequence(1);
        let associated_data = Aad::empty();

        let mut opening_key = OpeningKey::new(unbound_key, nonce_sequence);
        let mut cypher_text_with_tag = [in_out].concat();
        let decrypted_data =
            opening_key.open_in_place(associated_data, &mut cypher_text_with_tag)?;
        println!(
            "decrypted_data = {}",
            String::from_utf8(decrypted_data.to_vec()).unwrap()
        );

        assert_eq!(data, decrypted_data);
    }
    Ok(())
}

pub fn _generate_pk_2() {
    {
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key_bytes = doc.as_ref();
        let in_out = hex::encode(key_bytes);
        println!("hex: {in_out}");
    }
    {
        let bb: Vec<u8> = vec![
            48, 81, 2, 1, 1, 48, 5, 6, 3, 43, 101, 112, 4, 34, 4, 32, 34, 61, 165, 182, 130, 45,
            180, 147, 87, 159, 228, 68, 24, 246, 194, 34, 7, 4, 8, 157, 167, 30, 231, 43, 175, 157,
            113, 120, 38, 160, 111, 136, 129, 33, 0, 224, 154, 84, 147, 97, 252, 192, 56, 69, 23,
            140, 3, 175, 169, 210, 195, 193, 132, 41, 230, 48, 236, 40, 221, 142, 167, 56, 18, 3,
            36, 87, 80,
        ];
        let key_bytes = &bb[..];
        let in_out = hex::encode(key_bytes);
        println!("hex: {in_out}");
    }
}

pub fn _generate_pk_3() {
    {
        let bytes = hex::decode("3051020101300506032b657004220420223da5b6822db493579fe44418f6c2220704089da71ee72baf9d717826a06f88812100e09a549361fcc03845178c03afa9d2c3c18429e630ec28dd8ea7381203245750").unwrap();
        println!("{:#?}", bytes);

        let in_out = hex::encode(bytes);
        println!("hex: {in_out}");
    }
    {
        let bb: Vec<u8> = vec![
            48, 81, 2, 1, 1, 48, 5, 6, 3, 43, 101, 112, 4, 34, 4, 32, 34, 61, 165, 182, 130, 45,
            180, 147, 87, 159, 228, 68, 24, 246, 194, 34, 7, 4, 8, 157, 167, 30, 231, 43, 175, 157,
            113, 120, 38, 160, 111, 136, 129, 33, 0, 224, 154, 84, 147, 97, 252, 192, 56, 69, 23,
            140, 3, 175, 169, 210, 195, 193, 132, 41, 230, 48, 236, 40, 221, 142, 167, 56, 18, 3,
            36, 87, 80,
        ];
        let key_bytes = &bb[..];
        let in_out = hex::encode(key_bytes);
        println!("hex: {in_out}");
    }
}
