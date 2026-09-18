use chrono::{Timelike, Utc};

/// Hourly volume distribution weights based on typical crypto market patterns.
/// These weights represent the proportion of daily volume that occurs in each hour (UTC).
/// Sum = 1.0
///
/// Pattern rationale:
/// - Lowest: 02:00-04:00 UTC (Asia late night, US sleeping)
/// - Rising: 06:00-09:00 UTC (EU market opens)
/// - Peak: 14:00-17:00 UTC (US market hours, EU/US overlap)
/// - Declining: 18:00-23:00 UTC (US evening, Asia early)
const HOURLY_VOLUME_WEIGHTS: [f64; 24] = [
    0.030, // 00:00 UTC - Asia late evening
    0.025, // 01:00 UTC
    0.020, // 02:00 UTC - Lowest activity
    0.020, // 03:00 UTC - Lowest activity
    0.022, // 04:00 UTC
    0.028, // 05:00 UTC - Asia morning
    0.035, // 06:00 UTC - EU early morning
    0.042, // 07:00 UTC - EU morning
    0.050, // 08:00 UTC - EU active
    0.055, // 09:00 UTC - EU peak
    0.052, // 10:00 UTC - EU peak
    0.048, // 11:00 UTC - EU active
    0.045, // 12:00 UTC - EU lunch / US pre-market
    0.050, // 13:00 UTC - US pre-market
    0.058, // 14:00 UTC - US market open
    0.065, // 15:00 UTC - Peak (US active, EU still open)
    0.060, // 16:00 UTC - US active
    0.055, // 17:00 UTC - US active
    0.050, // 18:00 UTC - US afternoon
    0.045, // 19:00 UTC - US late afternoon
    0.040, // 20:00 UTC - US evening
    0.038, // 21:00 UTC - US late evening
    0.035, // 22:00 UTC - Asia early morning
    0.032, // 23:00 UTC - Asia early morning
];

/// Get the volume multiplier for the current hour.
///
/// Returns a multiplier where:
/// - 1.0 = average hourly volume (1/24 of daily)
/// - < 1.0 = below average (low activity hours like 02:00-04:00 UTC)
/// - > 1.0 = above average (high activity hours like 14:00-17:00 UTC)
///
/// The multiplier preserves total daily volume while distributing it organically.
///
/// # Examples
/// - Hour 3 (03:00 UTC): returns ~0.48 (48% of base trade size)
/// - Hour 15 (15:00 UTC): returns ~1.56 (156% of base trade size)
pub fn get_volume_multiplier(hour: u32) -> f64 {
    let hour = (hour % 24) as usize;
    let weight = HOURLY_VOLUME_WEIGHTS[hour];
    // Multiply by 24 to normalize: average weight (1/24) * 24 = 1.0
    weight * 24.0
}

/// Get the volume multiplier for the current UTC hour.
pub fn get_current_volume_multiplier() -> f64 {
    let current_hour = Utc::now().hour();
    get_volume_multiplier(current_hour)
}

/// Get the hourly weight (as percentage of daily volume) for display/logging.
pub fn get_hourly_weight_percent(hour: u32) -> f64 {
    let hour = (hour % 24) as usize;
    HOURLY_VOLUME_WEIGHTS[hour] * 100.0
}

/// Result of calculating a capped trade value
#[derive(Debug, Clone)]
pub struct CappedTradeValue {
    /// The trade value to use (may be capped)
    pub value_usdc: f64,
    /// Whether the trade was capped due to daily limit
    pub was_capped: bool,
    /// Remaining daily budget after this trade
    pub remaining_budget: f64,
    /// Whether daily budget is exhausted (skip this trade)
    pub skip_trade: bool,
}

/// Calculate trade value with daily budget cap
///
/// # Arguments
/// * `proposed_value_usdc` - The proposed trade value after hourly multiplier
/// * `remaining_budget_usdc` - Remaining daily volume budget
/// * `min_trade_threshold_usdc` - Minimum trade size (trades below this are skipped)
///
/// # Returns
/// * `CappedTradeValue` - The capped value and metadata
pub fn cap_trade_to_daily_budget(
    proposed_value_usdc: f64,
    remaining_budget_usdc: f64,
    min_trade_threshold_usdc: f64,
) -> CappedTradeValue {
    // If no budget remaining, skip the trade
    if remaining_budget_usdc <= min_trade_threshold_usdc {
        return CappedTradeValue {
            value_usdc: 0.0,
            was_capped: true,
            remaining_budget: remaining_budget_usdc,
            skip_trade: true,
        };
    }

    // If proposed value exceeds remaining budget, cap it
    if proposed_value_usdc > remaining_budget_usdc {
        // Only trade if the capped amount is above minimum threshold
        if remaining_budget_usdc >= min_trade_threshold_usdc {
            return CappedTradeValue {
                value_usdc: remaining_budget_usdc,
                was_capped: true,
                remaining_budget: 0.0,
                skip_trade: false,
            };
        } else {
            return CappedTradeValue {
                value_usdc: 0.0,
                was_capped: true,
                remaining_budget: remaining_budget_usdc,
                skip_trade: true,
            };
        }
    }

    // Normal case: trade within budget
    CappedTradeValue {
        value_usdc: proposed_value_usdc,
        was_capped: false,
        remaining_budget: remaining_budget_usdc - proposed_value_usdc,
        skip_trade: false,
    }
}
