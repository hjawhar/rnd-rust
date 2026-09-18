use crate::markets::generic::market_pair::{MarketEnum, MarketPair};

/// Result of market impact calculation
#[derive(Debug, Clone)]
pub struct MarketImpactResult {
    /// Market impact in basis points (100 = 1%)
    pub impact_bps: u16,
    /// Expected output after impact
    pub expected_output: f64,
    /// Ideal output if there was no impact
    pub ideal_output: f64,
}

/// Error type for market impact calculation
#[derive(Debug, Clone)]
pub enum MarketImpactError {
    /// No reserve data available for calculation
    NoReserveData(String),
    /// Unsupported market type
    UnsupportedMarket(String),
    /// Impact exceeds maximum allowed
    ImpactTooHigh { actual_bps: u16, max_bps: u16 },
}

impl std::fmt::Display for MarketImpactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketImpactError::NoReserveData(msg) => write!(f, "No reserve data: {}", msg),
            MarketImpactError::UnsupportedMarket(msg) => write!(f, "Unsupported market: {}", msg),
            MarketImpactError::ImpactTooHigh { actual_bps, max_bps } => {
                write!(
                    f,
                    "Market impact too high: {}bps exceeds max {}bps ({}% > {}%)",
                    actual_bps,
                    max_bps,
                    *actual_bps as f64 / 100.0,
                    *max_bps as f64 / 100.0
                )
            }
        }
    }
}

impl std::error::Error for MarketImpactError {}

/// Calculate market impact for a buy order (SOL → Token)
///
/// # Arguments
/// * `market_pair` - The market pair containing pool state
/// * `input_sol` - Amount of SOL being swapped in (in SOL units, e.g., 1.0 = 1 SOL)
/// * `quote_vault_balance` - Optional: SOL balance in the pool's quote vault in SOL units (for constant product AMMs)
///
/// # Returns
/// * `Ok(MarketImpactResult)` - Impact calculation successful
/// * `Err(MarketImpactError)` - Could not calculate impact
///
/// # Note
/// The impact calculation uses ratios, so the units must match between input_sol and quote_vault_balance.
pub fn calculate_buy_impact(
    market_pair: &MarketPair,
    input_sol: f64,
    quote_vault_balance: Option<f64>,
) -> Result<MarketImpactResult, MarketImpactError> {
    match &market_pair.market {
        MarketEnum::PumpfunAMM(pool) => {
            // Use virtual reserves only if bonding curve exists and is NOT complete
            if pool.bonding_curve.is_some() && !pool.bonding_curve.clone().unwrap().complete {
                let bc = pool.bonding_curve.as_ref().unwrap();
                // Pre-migration: Pumpfun uses virtual reserves for pricing
                let virtual_sol = bc.virtual_sol_reserves as f64 / 1e9;

                if virtual_sol <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Pumpfun virtual_sol_reserves is zero".into(),
                    ));
                }

                // Constant product formula: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_sol / (input_sol + virtual_sol);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate expected vs ideal output
                let virtual_tokens = bc.virtual_token_reserves as f64 / 1e6; // 6 decimals for pumpfun tokens
                let current_price = virtual_sol / virtual_tokens; // SOL per token
                let ideal_output = input_sol / current_price;
                let expected_output = ideal_output * (1.0 - impact_ratio);

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                // Post-migration (bonding curve complete or missing): use vault balances
                if let Some(reserve_sol) = quote_vault_balance {
                    if reserve_sol <= 0.0 {
                        return Err(MarketImpactError::NoReserveData(
                            "Pumpfun AMM quote vault balance is zero or negative".into(),
                        ));
                    }

                    let impact_ratio = input_sol / (input_sol + reserve_sol);
                    let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                    Ok(MarketImpactResult {
                        impact_bps,
                        expected_output: 0.0,
                        ideal_output: 0.0,
                    })
                } else {
                    Err(MarketImpactError::NoReserveData(
                        "No quote vault balance provided for post-migration Pumpfun AMM".into(),
                    ))
                }
            }
        }

        MarketEnum::RaydiumCLMM(pool) => {
            // CLMM uses concentrated liquidity
            let liquidity = pool.liquidity as f64;

            if liquidity <= 0.0 {
                return Err(MarketImpactError::NoReserveData(
                    "CLMM liquidity is zero".into(),
                ));
            }

            let sqrt_price_x64 = pool.sqrt_price_x64 as f64;
            let sqrt_price = sqrt_price_x64 / ((1u128 << 64) as f64);

            if sqrt_price <= 0.0 {
                return Err(MarketImpactError::NoReserveData(
                    "CLMM sqrt_price is zero".into(),
                ));
            }

            // For CLMM, price impact depends on how much liquidity is available
            // Simplified approximation: delta_sqrt_price = amount / liquidity
            // Price impact ≈ 2 * delta_sqrt_price / sqrt_price
            let input_amount = input_sol * 1e9; // Convert to lamports for calculation
            let delta_sqrt_price = input_amount / liquidity;
            let impact_ratio = (2.0 * delta_sqrt_price / sqrt_price).min(1.0);
            let impact_bps = (impact_ratio * 10000.0) as u16;

            // Current price from sqrt_price: price = sqrt_price^2
            let current_price = sqrt_price * sqrt_price;
            let ideal_output = input_sol / current_price;
            let expected_output = ideal_output * (1.0 - impact_ratio);

            Ok(MarketImpactResult {
                impact_bps,
                expected_output,
                ideal_output,
            })
        }

        MarketEnum::RaydiumAMMV4(_) => {
            // Constant product AMM: x * y = k
            if let Some(reserve_sol) = quote_vault_balance {
                if reserve_sol <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Quote vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_sol / (input_sol + reserve_sol);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                // For buy: tokens_out = reserve_tokens - (k / (reserve_sol + input_sol))
                // Simplified: tokens_out = reserve_tokens * impact_ratio
                // Without reserve_tokens, we approximate using impact_ratio
                let expected_output = input_sol * (1.0 - impact_ratio);
                let ideal_output = input_sol;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No quote vault balance provided for Raydium AMM V4".into(),
                ))
            }
        }

        MarketEnum::MeteoraDAMM(_) => {
            // Constant product AMM: x * y = k
            if let Some(reserve_sol) = quote_vault_balance {
                if reserve_sol <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Quote vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_sol / (input_sol + reserve_sol);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                let expected_output = input_sol * (1.0 - impact_ratio);
                let ideal_output = input_sol;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No quote vault balance provided for Meteora DAMM".into(),
                ))
            }
        }

        MarketEnum::MeteoraDLMM(_) => {
            // Dynamic liquidity market maker - similar to constant product for impact
            if let Some(reserve_sol) = quote_vault_balance {
                if reserve_sol <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Quote vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_sol / (input_sol + reserve_sol);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                let expected_output = input_sol * (1.0 - impact_ratio);
                let ideal_output = input_sol;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No quote vault balance provided for Meteora DLMM".into(),
                ))
            }
        }

        MarketEnum::MeteoraDAMMV2(_) => Err(MarketImpactError::UnsupportedMarket(
            "MeteoraDAMMV2 not yet supported".into(),
        )),
    }
}

/// Calculate market impact for a sell order (Token → SOL)
///
/// # Arguments
/// * `market_pair` - The market pair containing pool state
/// * `input_tokens` - Amount of tokens being sold (in raw token units, same units as vault balance)
/// * `base_vault_balance` - Optional: Token balance in the pool's base vault in raw units (for constant product AMMs)
///
/// # Returns
/// * `Ok(MarketImpactResult)` - Impact calculation successful
/// * `Err(MarketImpactError)` - Could not calculate impact
///
/// # Note
/// The impact calculation uses ratios, so raw units and decimal-adjusted units produce the same result.
pub fn calculate_sell_impact(
    market_pair: &MarketPair,
    input_tokens: f64,
    base_vault_balance: Option<f64>,
) -> Result<MarketImpactResult, MarketImpactError> {
    match &market_pair.market {
        MarketEnum::PumpfunAMM(pool) => {
            // Use virtual reserves only if bonding curve exists and is NOT complete
            if pool.bonding_curve.is_some() && !pool.bonding_curve.clone().unwrap().complete {
                let bc = pool.bonding_curve.as_ref().unwrap();
                // Pre-migration: use virtual reserves
                let virtual_tokens = bc.virtual_token_reserves as f64 / 1e6;

                if virtual_tokens <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Pumpfun virtual_token_reserves is zero".into(),
                    ));
                }

                let impact_ratio = input_tokens / (input_tokens + virtual_tokens);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                let virtual_sol = bc.virtual_sol_reserves as f64 / 1e9;
                let current_price = virtual_sol / virtual_tokens;
                let ideal_output = input_tokens * current_price;
                let expected_output = ideal_output * (1.0 - impact_ratio);

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                // Post-migration (bonding curve complete or missing): use vault balances
                if let Some(reserve_tokens) = base_vault_balance {
                    if reserve_tokens <= 0.0 {
                        return Err(MarketImpactError::NoReserveData(
                            "Pumpfun AMM base vault balance is zero or negative".into(),
                        ));
                    }

                    let impact_ratio = input_tokens / (input_tokens + reserve_tokens);
                    let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                    Ok(MarketImpactResult {
                        impact_bps,
                        expected_output: 0.0,
                        ideal_output: 0.0,
                    })
                } else {
                    Err(MarketImpactError::NoReserveData(
                        "No base vault balance provided for post-migration Pumpfun AMM".into(),
                    ))
                }
            }
        }

        MarketEnum::RaydiumCLMM(pool) => {
            let liquidity = pool.liquidity as f64;

            if liquidity <= 0.0 {
                return Err(MarketImpactError::NoReserveData(
                    "CLMM liquidity is zero".into(),
                ));
            }

            let sqrt_price_x64 = pool.sqrt_price_x64 as f64;
            let sqrt_price = sqrt_price_x64 / ((1u128 << 64) as f64);

            if sqrt_price <= 0.0 {
                return Err(MarketImpactError::NoReserveData(
                    "CLMM sqrt_price is zero".into(),
                ));
            }

            // For selling tokens, we're moving price down
            // Similar approximation as buy but in reverse direction
            let delta_sqrt_price = input_tokens * sqrt_price / liquidity;
            let impact_ratio = (2.0 * delta_sqrt_price / sqrt_price).min(1.0);
            let impact_bps = (impact_ratio * 10000.0) as u16;

            let current_price = sqrt_price * sqrt_price;
            let ideal_output = input_tokens * current_price;
            let expected_output = ideal_output * (1.0 - impact_ratio);

            Ok(MarketImpactResult {
                impact_bps,
                expected_output,
                ideal_output,
            })
        }

        MarketEnum::RaydiumAMMV4(_) => {
            // Constant product AMM: x * y = k
            if let Some(reserve_tokens) = base_vault_balance {
                if reserve_tokens <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Base vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_tokens / (input_tokens + reserve_tokens);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                // For sell: sol_out = reserve_sol * impact_ratio
                // We approximate SOL output relative to token input
                let expected_output = input_tokens * (1.0 - impact_ratio);
                let ideal_output = input_tokens;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No base vault balance provided for Raydium AMM V4".into(),
                ))
            }
        }

        MarketEnum::MeteoraDAMM(_) => {
            // Constant product AMM: x * y = k
            if let Some(reserve_tokens) = base_vault_balance {
                if reserve_tokens <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Base vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_tokens / (input_tokens + reserve_tokens);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                let expected_output = input_tokens * (1.0 - impact_ratio);
                let ideal_output = input_tokens;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No base vault balance provided for Meteora DAMM".into(),
                ))
            }
        }

        MarketEnum::MeteoraDLMM(_) => {
            // Dynamic liquidity market maker - similar to constant product for impact
            if let Some(reserve_tokens) = base_vault_balance {
                if reserve_tokens <= 0.0 {
                    return Err(MarketImpactError::NoReserveData(
                        "Base vault balance is zero or negative".into(),
                    ));
                }

                // Constant product: impact = amount_in / (amount_in + reserve_in)
                let impact_ratio = input_tokens / (input_tokens + reserve_tokens);
                let impact_bps = (impact_ratio * 10000.0).min(10000.0) as u16;

                // Calculate outputs using constant product formula
                let expected_output = input_tokens * (1.0 - impact_ratio);
                let ideal_output = input_tokens;

                Ok(MarketImpactResult {
                    impact_bps,
                    expected_output,
                    ideal_output,
                })
            } else {
                Err(MarketImpactError::NoReserveData(
                    "No base vault balance provided for Meteora DLMM".into(),
                ))
            }
        }

        MarketEnum::MeteoraDAMMV2(_) => Err(MarketImpactError::UnsupportedMarket(
            "MeteoraDAMMV2 not yet supported".into(),
        )),
    }
}

/// Validate that market impact is within acceptable limits
///
/// # Arguments
/// * `impact_result` - The calculated market impact
/// * `max_impact_bps` - Maximum allowed impact in basis points
///
/// # Returns
/// * `Ok(())` - Impact is within limits
/// * `Err(MarketImpactError::ImpactTooHigh)` - Impact exceeds limit
pub fn validate_impact(
    impact_result: &MarketImpactResult,
    max_impact_bps: u16,
) -> Result<(), MarketImpactError> {
    if impact_result.impact_bps > max_impact_bps {
        Err(MarketImpactError::ImpactTooHigh {
            actual_bps: impact_result.impact_bps,
            max_bps: max_impact_bps,
        })
    } else {
        Ok(())
    }
}

