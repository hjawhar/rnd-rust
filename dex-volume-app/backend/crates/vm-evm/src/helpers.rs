use alloy::{
    primitives::{I256, U256},
    signers::local::PrivateKeySigner,
};
use bigdecimal::{BigDecimal, ToPrimitive};
use std::{error::Error, ops::Mul, str::FromStr};

pub fn str_to_pk(private_key: &str) -> Result<PrivateKeySigner, Box<dyn Error + Send + Sync>> {
    let kp = PrivateKeySigner::from_str(private_key)
        .map_err(|err| format!("Error decoding keypair: {err:#?}"))?;
    Ok(kp)
}


pub fn f64_to_wei(input: f64) -> U256 {
    let n1 = BigDecimal::from_str(&input.to_string()).unwrap();
    let n2 = BigDecimal::from_str(&(1e18f64).to_string()).unwrap();
    let multiplied = n1.mul(n2);
    let u128_n = multiplied.to_u128().unwrap();
    U256::from(u128_n)
}

pub fn f64_to_token_units(input: f64, decimals: u8) -> U256 {
    let n1 = BigDecimal::from_str(&input.to_string()).unwrap();
    let n2 = BigDecimal::from_str(&10f64.powi(decimals as i32).to_string()).unwrap();
    let multiplied = n1.mul(n2);
    let u128_n = multiplied.to_u128().unwrap_or(0);
    U256::from(u128_n)
}

pub fn u256_to_f64(input: U256) -> f64 {
    input.to_string().parse().unwrap_or(0.0)
}

pub fn i256_to_f64(input: I256) -> f64 {
    input.to_string().parse().unwrap_or(0.0)
}

pub fn wei_to_f64(input: U256) -> f64 {
    let n = BigDecimal::from_str(&input.to_string()).unwrap();
    let divisor = BigDecimal::from_str(&(1e18f64).to_string()).unwrap();
    let result = n / divisor;
    result.to_f64().unwrap_or(0.0)
}

pub fn token_units_to_f64(input: U256, decimals: u8) -> f64 {
    let n = BigDecimal::from_str(&input.to_string()).unwrap();
    let divisor = BigDecimal::from_str(&10f64.powi(decimals as i32).to_string()).unwrap();
    let result = n / divisor;
    result.to_f64().unwrap_or(0.0)
}

pub fn split_array_ranges(start: u64, end: u64, amount: u64) -> Vec<(u64, u64)> {
    if start > end || amount == 0 {
        return vec![];
    }
    let mut result = Vec::new();
    let mut current = start;
    while current <= end {
        let chunk_end = std::cmp::min(current + amount - 1, end);
        result.push((current, chunk_end));
        current = chunk_end + 1;
    }
    result
}
