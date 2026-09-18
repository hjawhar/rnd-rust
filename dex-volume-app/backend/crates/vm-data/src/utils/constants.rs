use once_cell::sync::Lazy;

use crate::utils::claims::Keys;

pub static KEYS: Lazy<Keys> = Lazy::new(|| {
    let key = std::env::var("ED25519_KEY").unwrap();
    let bytes = hex::decode(key).unwrap();
    let slice_bytes = &bytes[..];
    Keys::new(slice_bytes)
});

pub const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const WSOL: &str = "So11111111111111111111111111111111111111112";
pub const JITO_TIP_ACCOUNT: &str = "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe";
pub const METEORA_DYNAMIC_AMM: &str = "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB";
pub const METEORA_DYNAMIC_AMM_V2: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const METEORA_DYNAMIC_LMM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const METEORA_VAULT_PROGRAM: &str = "24Uqj9JCLxUeoC3hGfh5W3s9FM9uCHDS2SG3LYwBpyTi";
pub const METEORA_VAULT_AUTHORITY: &str = "29CrdHkjXSeV6wAWyHC1J8iahRbf5F4unfFtKgRz6hcf";
pub const METEORA_EVENTS_AUTHORITY: &str = "D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6";

pub const RAYDIUM_LIQUIDITY_POOL_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub const RAYDIUM_AUTHORITY_V4: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";

pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"; // spl_token::id()
pub const TOKEN_PROGRAM_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const MEMO_PROGRAM_V2: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

pub const PUMPFUN_AMM_PROGRAM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub const PUMPFUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

pub const RAYDIUM_CLMM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
