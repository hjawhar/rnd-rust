use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint,
    entrypoint::ProgramResult, msg, program_error::ProgramError, pubkey::Pubkey,
};

pub mod adapters;
pub mod checks;
use adapters::common;

entrypoint!(process_instruction);

// Error codes (ProgramError::Custom).
pub const ERR_SLIPPAGE: u32 = 1;
pub const ERR_DEX_PROGRAM_MISMATCH: u32 = 2;
pub const ERR_CHAIN_MISMATCH: u32 = 3;
pub const ERR_BAD_AUTHORITY: u32 = 4;
pub const ERR_ZERO_OUTPUT: u32 = 5;
pub const ERR_INPUT_OVERDRAW: u32 = 6;
pub const ERR_SPLITS_UNSUPPORTED: u32 = 7;
pub const ERR_AMOUNTS_MISMATCH: u32 = 8;

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy)]
pub enum DexType {
    MeteoraDAMMV1,  // 0
    MeteoraDAMMV2,  // 1
    MeteoraDLMM,    // 2
    RaydiumCLMM,    // 3
    RaydiumAMMV4,   // 4
    PumpfunBuy,     // 5
    PumpfunSell,    // 6
}

/// One CPI into one DEX pool. `weight` is the share (0..=100) of the hop's
/// input routed through this leg; the last leg of a hop takes the remainder.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct SwapLeg {
    pub dex_type: DexType,
    pub num_accounts: u8,
    pub weight: u8,
}

/// One hop of a path: source mint -> dest mint, possibly split across pools.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct HopArgs {
    pub legs: Vec<SwapLeg>,
}

/// One independent path from input mint to output mint.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct PathArgs {
    pub hops: Vec<HopArgs>,
}

/// Wire format is forward-compatible with two-level split routing:
/// `amounts[i]` is the input for `paths[i]` (level-1 split, must sum to
/// `amount_in`); each hop's `legs` split that hop's amount by weight
/// (level-2). v1 execution accepts exactly one path with single-leg hops.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct ExecuteRouteArgs {
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub amounts: Vec<u64>,
    pub paths: Vec<PathArgs>,
}

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let args = ExecuteRouteArgs::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    if args.paths.is_empty() || args.amounts.len() != args.paths.len() {
        return Err(ProgramError::InvalidInstructionData);
    }
    // Invariant enforced from day one so split execution can rely on it.
    let total: u64 = args
        .amounts
        .iter()
        .try_fold(0u64, |acc, a| acc.checked_add(*a))
        .ok_or(ProgramError::Custom(ERR_AMOUNTS_MISMATCH))?;
    if total != args.amount_in {
        return Err(ProgramError::Custom(ERR_AMOUNTS_MISMATCH));
    }
    // v1: single path, single leg per hop. Wire format already carries splits.
    if args.paths.len() != 1 {
        return Err(ProgramError::Custom(ERR_SPLITS_UNSUPPORTED));
    }
    let path = &args.paths[0];
    if path.hops.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    msg!(
        "Thunder Router: {} hops, amount_in={}, min_out={}",
        path.hops.len(),
        args.amount_in,
        args.min_amount_out
    );

    let mut offset: usize = 0;
    let mut current_amount = args.amount_in;
    let mut prev_dest_mint: Option<[u8; 32]> = None;

    for (i, hop) in path.hops.iter().enumerate() {
        if hop.legs.len() != 1 {
            return Err(ProgramError::Custom(ERR_SPLITS_UNSUPPORTED));
        }
        let leg = &hop.legs[0];
        let n = leg.num_accounts as usize;
        if accounts.len() < offset + n {
            msg!(
                "Hop {}: need {} accounts at offset {}, have {}",
                i,
                n,
                offset,
                accounts.len()
            );
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        let hop_accounts = &accounts[offset..offset + n];

        // Uniform prefix: [2] is always swap_source_token, [3] is always
        // swap_destination_token.
        let dest_mint = checks::before_hop(hop_accounts, leg.dex_type, prev_dest_mint.as_ref())?;
        let source_account = &hop_accounts[2];
        let dest_account = &hop_accounts[3];
        let source_before = common::read_token_balance(source_account);
        let balance_before = common::read_token_balance(dest_account);

        msg!(
            "Hop {}: {:?} ({} accounts, amount={})",
            i,
            leg.dex_type,
            n,
            current_amount
        );

        match leg.dex_type {
            DexType::MeteoraDAMMV1 => {
                adapters::meteora_damm_v1::swap(hop_accounts, current_amount)?
            }
            DexType::MeteoraDAMMV2 => {
                adapters::meteora_damm_v2::swap(hop_accounts, current_amount)?
            }
            DexType::MeteoraDLMM => adapters::meteora_dlmm::swap(hop_accounts, current_amount)?,
            DexType::RaydiumCLMM => adapters::raydium_clmm::swap(hop_accounts, current_amount)?,
            DexType::RaydiumAMMV4 => adapters::raydium_v4::swap(hop_accounts, current_amount)?,
            DexType::PumpfunBuy => adapters::pumpfun::buy(hop_accounts, current_amount)?,
            DexType::PumpfunSell => adapters::pumpfun::sell(hop_accounts, current_amount)?,
        };

        // Post-swap: the DEX may not have pulled more than the chained amount
        // from the source account.
        let source_after = common::read_token_balance(source_account);
        let actual_in = source_before.saturating_sub(source_after);
        if actual_in > current_amount {
            msg!("Hop {}: pulled {} but only {} chained", i, actual_in, current_amount);
            return Err(ProgramError::Custom(ERR_INPUT_OVERDRAW));
        }

        let balance_after = common::read_token_balance(dest_account);
        current_amount = balance_after.saturating_sub(balance_before);
        if current_amount == 0 {
            return Err(ProgramError::Custom(ERR_ZERO_OUTPUT));
        }

        msg!("Hop {}: output={}", i, current_amount);
        offset += n;
        prev_dest_mint = Some(dest_mint);
    }

    if current_amount < args.min_amount_out {
        msg!(
            "Slippage exceeded: got {}, minimum {}",
            current_amount,
            args.min_amount_out
        );
        return Err(ProgramError::Custom(ERR_SLIPPAGE));
    }

    msg!("Route complete: final_output={}", current_amount);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden wire bytes shared with the engine-side mirror types
    /// (crates/engine/src/swap.rs). If this test or its engine twin fails,
    /// the two borsh definitions have drifted.
    const GOLDEN_BYTES: &[u8] = &[
        0x40, 0x42, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, // amount_in = 1_000_000
        0xA0, 0xBB, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x00, // min_amount_out = 900_000
        0x01, 0x00, 0x00, 0x00, // amounts.len = 1
        0x40, 0x42, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, // amounts[0] = 1_000_000
        0x01, 0x00, 0x00, 0x00, // paths.len = 1
        0x02, 0x00, 0x00, 0x00, // paths[0].hops.len = 2
        0x01, 0x00, 0x00, 0x00, // hop0.legs.len = 1
        0x01, 0x0D, 0x64, // DAMM V2, 13 accounts, weight 100
        0x01, 0x00, 0x00, 0x00, // hop1.legs.len = 1
        0x02, 0x13, 0x64, // DLMM, 19 accounts, weight 100
    ];

    fn golden_args() -> ExecuteRouteArgs {
        ExecuteRouteArgs {
            amount_in: 1_000_000,
            min_amount_out: 900_000,
            amounts: vec![1_000_000],
            paths: vec![PathArgs {
                hops: vec![
                    HopArgs {
                        legs: vec![SwapLeg {
                            dex_type: DexType::MeteoraDAMMV2,
                            num_accounts: 13,
                            weight: 100,
                        }],
                    },
                    HopArgs {
                        legs: vec![SwapLeg {
                            dex_type: DexType::MeteoraDLMM,
                            num_accounts: 19,
                            weight: 100,
                        }],
                    },
                ],
            }],
        }
    }

    #[test]
    fn golden_bytes_roundtrip() {
        let ser = borsh::to_vec(&golden_args()).unwrap();
        assert_eq!(ser.as_slice(), GOLDEN_BYTES);
        let de = ExecuteRouteArgs::try_from_slice(GOLDEN_BYTES).unwrap();
        assert_eq!(de.amount_in, 1_000_000);
        assert_eq!(de.amounts, vec![1_000_000]);
        assert_eq!(de.paths[0].hops.len(), 2);
    }
}
