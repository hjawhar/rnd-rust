use bigdecimal::{BigDecimal, ToPrimitive};
use rand::{Rng, seq::SliceRandom};
use std::{
    ops::Mul,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn mul_f64_and_u64_to_u64(x: f64, y: u64) -> u64 {
    let result = x.mul(y as f64).floor();
    if result.is_nan() || result.is_infinite() || result < 0.0 {
        return 0;
    }
    result as u64
}

pub fn f64_to_big_int(input: f64) -> BigDecimal {
    BigDecimal::from_str(&input.to_string()).unwrap()
}

pub fn big_int_to_f64(input: BigDecimal) -> f64 {
    input.to_f64().unwrap()
}

pub fn get_current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}


pub fn modify_by_random_percent(number: f64, min_percent: f64, max_percent: f64) -> f64 {
    let mut rng = rand::rng();

    // Generate random percentage between min and max
    let random_percent = rng.random_range(min_percent..=max_percent);

    // Randomly choose to add or subtract
    let is_positive = rng.random_bool(0.5);

    if is_positive {
        number * (1.0 + random_percent / 100.0)
    } else {
        number * (1.0 - random_percent / 100.0)
    }
}

pub fn shuffle_array<T: Clone>(input: &[T]) -> Result<Vec<T>, &'static str> {
    let mut rng = rand::rng();
    let mut selected = input.to_vec();
    selected.shuffle(&mut rng);

    Ok(selected)
}

pub fn random_between_i32(min: i32, max: i32) -> Result<i32, &'static str> {
    if min > max {
        return Err("Minimum value cannot be greater than maximum value");
    }

    let mut rng = rand::rng();
    Ok(rng.random_range(min..=max))
}


/// Validate an EVM address (0x-prefixed, 42 hex chars = 20 bytes).
/// Returns the address as-is when valid, `None` otherwise.
pub fn validate_evm_address(addr: &str) -> Option<String> {
    if !addr.starts_with("0x") || addr.len() != 42 {
        return None;
    }
    if addr[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        Some(addr.to_string())
    } else {
        None
    }
}