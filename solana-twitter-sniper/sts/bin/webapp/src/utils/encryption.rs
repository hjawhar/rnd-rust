use dotenv::dotenv;

use alloy::hex;
use ring::aead::Aad;
// use ring::aead::Algorithm;
use jsonwebtoken::{
    decode, encode, get_current_timestamp, Algorithm, DecodingKey, EncodingKey, Validation,
};
use ring::aead::BoundKey;
use ring::aead::Nonce;
use ring::aead::NonceSequence;
use ring::aead::OpeningKey;
use ring::aead::SealingKey;
use ring::aead::UnboundKey;
use ring::aead::AES_256_GCM;
use ring::aead::NONCE_LEN;
use ring::error::Unspecified;
use ring::rand::SecureRandom;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use crate::models::claims::Claims;

struct CounterNonceSequence(u32);

impl NonceSequence for CounterNonceSequence {
    // called once for each seal operation
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        let mut nonce_bytes = vec![0; NONCE_LEN];

        let bytes = self.0.to_be_bytes();
        nonce_bytes[8..].copy_from_slice(&bytes);
        // println!("nonce_bytes = {}", hex::encode(&nonce_bytes));

        self.0 += 1; // advance the counter
        Nonce::try_assume_unique_for_key(&nonce_bytes)
    }
}

pub fn _test_encryption() -> Result<(), Unspecified> {
    dotenv().ok();
    // Create a new instance of SystemRandom to be used as the single source of entropy
    // let rand = SystemRandom::new();

    // Generate a new symmetric encryption key
    // let mut key_bytes = vec![0; AES_256_GCM.key_len()];
    // rand.fill(&mut key_bytes)?;
    // println!("key_bytes = {}", hex::encode(&key_bytes)); // don't print this in production code
    let key = std::env::var("AES256_GCM_KEY").unwrap();
    let key_bytes = hex::decode(key).unwrap();
    let data = b"tel7as tize";
    {
        // Create a new AEAD key without a designated role or nonce sequence
        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;

        // Create a new NonceSequence type which generates nonces
        let nonce_sequence = CounterNonceSequence(1);

        // Create a new AEAD key for encrypting and signing ("sealing"), bound to a nonce sequence
        // The SealingKey can be used multiple times, each time a new nonce will be used
        let mut sealing_key = SealingKey::new(unbound_key, nonce_sequence);

        // This data will be authenticated but not encrypted
        let associated_data = Aad::empty(); // is optional so can be empty
                                            // let associated_data = Aad::from(b"additional public data");

        // Data to be encrypted
        println!("data = {}", String::from_utf8(data.to_vec()).unwrap());

        println!("init data {:?}", data);
        println!("init data length {}", data.len());
        // Create a mutable copy of the data that will be encrypted in place
        let mut in_out = data.clone();

        // Encrypt the data with AEAD using the AES_256_GCM algorithm
        let tag = sealing_key.seal_in_place_separate_tag(associated_data, &mut in_out)?;
        // println!("{:#?}", tag.as_ref());
        // println!("{:#?}", tag.as_ref().to_vec());
        // println!("final data {:?}", in_out);
        // println!("final data length {}", in_out.len());
        // // don't print this in production code
        // let s = String::from_utf8_lossy(in_out.as_ref());
        // println!("encrypted text: {:X?}", in_out.as_ref());
        // println!("In out = {}", hex::encode(&in_out.as_ref())); // don't print this in production code
        // println!("Tag = {}", hex::encode(&tag.as_ref())); // don't print this in production code
        let final_hash = format!(
            "{}{}",
            hex::encode(&in_out.as_ref()),
            hex::encode(&tag.as_ref())
        );
        println!("{}", final_hash);
    }
    println!("-----------------");
    {
        let in_out =
            &hex::decode("000b5da730250adc55253483e917c1e1c2ca4b5d6d91b210d5f4e2").unwrap()[..];
        // let tag = &hex::decode("a7c0c1b283ecab941590daa8bf32ffa4").unwrap()[..];

        // Recreate the previously moved variables
        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;
        let nonce_sequence = CounterNonceSequence(1);
        let associated_data = Aad::empty(); // supplying the wrong data causes the decryption to fail
                                            // let associated_data = Aad::from(b"additional public data");

        // Create a new AEAD key for decrypting and verifying the authentication tag
        let mut opening_key = OpeningKey::new(unbound_key, nonce_sequence);

        // Decrypt the data by passing in the associated data and the cypher text with the authentication tag appended
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

pub fn encrypt(input: String) -> Result<String, Unspecified> {
    dotenv().ok();
    let key = std::env::var("AES256_GCM_KEY").unwrap();
    let key_bytes = hex::decode(key).unwrap();
    let data = input.as_bytes().to_vec();
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;
    let nonce_sequence = CounterNonceSequence(1);
    let mut sealing_key = SealingKey::new(unbound_key, nonce_sequence);
    let associated_data = Aad::empty();
    let mut in_out = data.clone();
    let tag = sealing_key.seal_in_place_separate_tag(associated_data, &mut in_out)?;
    let final_hash = format!("{}{}", hex::encode(in_out), hex::encode(&tag.as_ref()));

    Ok(final_hash)
}

pub fn decrypt(input: String) -> Result<String, Unspecified> {
    dotenv().ok();
    let key = std::env::var("AES256_GCM_KEY").unwrap();
    let key_bytes = hex::decode(key).unwrap();
    let in_out = &hex::decode(input).unwrap()[..];
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;
    let nonce_sequence = CounterNonceSequence(1);
    let associated_data = Aad::empty();
    let mut opening_key = OpeningKey::new(unbound_key, nonce_sequence);
    let mut cypher_text_with_tag = [in_out].concat();
    let decrypted_data = opening_key.open_in_place(associated_data, &mut cypher_text_with_tag)?;
    Ok(String::from_utf8(decrypted_data.to_vec()).unwrap())
}

pub fn _generate_pk() {
    // Private key bytes removed for security. Generate your own with:
    // let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    // println!("{:#?}", doc.as_ref());
}

pub fn _generate_pk_2() {
    // Generate a new Ed25519 keypair
    let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let key_bytes = doc.as_ref();
    let in_out = hex::encode(key_bytes);
    println!("hex: {in_out}");
}

pub fn _generate_pk_3() {
    // Private key material removed for security.
}

pub fn generate_signing_pk() -> String {
    let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let aa = &doc.as_ref()[..];
    let in_out = hex::encode(aa);
    // println!("{:#?}", in_out);
    // println!("{:#?}", 123123123);
    in_out
}

pub fn generate_encryption_key() -> Result<String, Unspecified> {
    let rand = SystemRandom::new();

    // Generate a new symmetric encryption key
    let mut key_bytes = vec![0; AES_256_GCM.key_len()];
    rand.fill(&mut key_bytes)?;
    println!("key_bytes = {}", hex::encode(&key_bytes));

    Ok(hex::encode(&key_bytes))
}
