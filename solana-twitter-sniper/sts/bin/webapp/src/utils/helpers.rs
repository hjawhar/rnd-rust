use bigdecimal::{BigDecimal, ToPrimitive};
use std::{
    ops::Mul,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn f64_to_u64(input: f64) -> u64 {
    input.floor().to_string().parse::<u64>().unwrap()
}

pub fn mul_f64_and_u64_to_u64(x: f64, y: u64) -> u64 {
    (x.mul(y as f64).floor())
        .to_string()
        .parse::<u64>()
        .unwrap()
}

pub fn f64_to_big_int(input: Option<f64>) -> Option<BigDecimal> {
    if let Some(input) = input {
        Some(BigDecimal::from_str(&input.to_string()).unwrap())
    } else {
        None
    }
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
