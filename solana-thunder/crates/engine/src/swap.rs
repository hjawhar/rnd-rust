//! Transaction assembly: builds a complete unsigned VersionedTransaction.
//!
//! All swaps use the **router** path: a single compact `ExecuteRouteArgs`
//! instruction targeting the thunder-router on-chain program. The engine only
//! collects accounts in adapter order; the router program CPIs into each DEX,
//! reads balances, and chains actual output amounts between hops.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_pubkey::Pubkey;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{v0, VersionedMessage},
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_ix;
use spl_associated_token_account::{
    get_associated_token_address_with_program_id,
    instruction::create_associated_token_account_idempotent,
};
use spl_token::instruction::{close_account, sync_native};
use thunder_aggregator::types::Route;
use thunder_core::{calculate_min_amount_out, GenericError, WSOL, TOKEN_PROGRAM, TOKEN_PROGRAM_2022};

use crate::account_store::AccountStore;
use crate::pool_registry::PoolRegistry;

// ---------------------------------------------------------------------------
// Router instruction types (mirrors crates/router-program/src/lib.rs V2)
//
// Duplicated here to avoid cross-crate dependency conflicts between
// solana-program 2.x (router-program) and solana-sdk 3.x (engine).
// ---------------------------------------------------------------------------

#[derive(BorshSerialize)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
enum DexType {
    MeteoraDAMMV1 = 0,
    MeteoraDAMMV2 = 1,
    MeteoraDLMM = 2,
    RaydiumCLMM = 3,
    RaydiumAMMV4 = 4,
    PumpfunBuy = 5,
    PumpfunSell = 6,
}

/// One CPI into one DEX pool. `weight` is the share (0..=100) of the hop's
/// input routed through this leg; the last leg of a hop takes the remainder.
#[derive(BorshSerialize)]
struct SwapLeg {
    dex_type: DexType,
    num_accounts: u8,
    weight: u8,
}

#[derive(BorshSerialize)]
struct HopArgs {
    legs: Vec<SwapLeg>,
}

#[derive(BorshSerialize)]
struct PathArgs {
    hops: Vec<HopArgs>,
}

/// Wire format is forward-compatible with two-level split routing;
/// v1 always sends one path with single-leg hops.
#[derive(BorshSerialize)]
struct ExecuteRouteArgs {
    amount_in: u64,
    min_amount_out: u64,
    amounts: Vec<u64>,
    paths: Vec<PathArgs>,
}

// ---------------------------------------------------------------------------
// Program / authority constants
// ---------------------------------------------------------------------------

pub(crate) const DAMM_V1_PROGRAM: &str = "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB";
pub(crate) const DAMM_V2_PROGRAM: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub(crate) const DAMM_V2_POOL_AUTHORITY: &str = "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC";
pub(crate) const DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub(crate) const DLMM_EVENT_AUTHORITY: &str = "D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6";
pub(crate) const CLMM_PROGRAM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
pub(crate) const RAY_V4_PROGRAM: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub(crate) const RAY_V4_AUTHORITY: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";
pub(crate) const PUMPFUN_PROGRAM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub(crate) const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
/// Meteora dynamic vault program (DAMM V1 pools keep funds in these vaults).
pub(crate) const VAULT_PROGRAM: &str = meteora_damm::METEORA_VAULT_PROGRAM;
pub(crate) const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub(crate) const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// PDA of the Pumpfun AMM `GlobalConfig` account
/// (`["global_config"]` under the pAMMBay program).
pub fn pumpfun_global_config_pda() -> Pubkey {
    let dex = Pubkey::from_str_const(PUMPFUN_PROGRAM);
    Pubkey::find_program_address(&[b"global_config"], &dex).0
}

/// PDA of a Raydium CLMM pool's tick-array bitmap extension account.
pub fn clmm_bitmap_extension_pda(pool: &Pubkey) -> Pubkey {
    raydium_clmm::tick_arrays::pda_array_bitmap_address(pool)
        .expect("bitmap extension PDA derivation is infallible")
        .0
}

/// The deployed thunder-router program, from the `ROUTER_PROGRAM_ID` env var
/// (read once). LiteSVM tests, surfpool, and mainnet each use their own deploy.
fn router_program_id() -> Result<Pubkey, GenericError> {
    static ID: std::sync::OnceLock<Option<Pubkey>> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("ROUTER_PROGRAM_ID")
            .ok()
            .and_then(|s| s.parse::<Pubkey>().ok())
    })
    .ok_or_else(|| "ROUTER_PROGRAM_ID env var not set or invalid".into())
}

// ---------------------------------------------------------------------------
// Compute budget
// ---------------------------------------------------------------------------

/// Default compute-unit price when the caller asks for "auto" (µ-lamports).
pub const DEFAULT_COMPUTE_UNIT_PRICE: u64 = 100_000;
/// Compute-unit limit clamp bounds.
pub const MIN_COMPUTE_UNIT_LIMIT: u32 = 120_000;
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
/// Maximum serialized transaction size (IPv6 MTU minus headers).
pub const MAX_TX_SIZE: usize = 1232;

/// Measured per-hop router-instruction CU (LiteSVM M2 fixtures, re-measured
/// against live pool state on a surfpool fork in M3). DAMM V1's dynamic-vault
/// deposit/withdraw path and pumpfun's fee/cashback transfers consume
/// noticeably more on live state than on the pinned fixtures (fixtures
/// measured 90.3k / 100.8k; live runs exceeded both), so those two carry the
/// live-observed figures. The pumpfun figure includes the one-time
/// user-volume-accumulator init.
fn hop_compute_units(dex_name: &str) -> u64 {
    match dex_name {
        "Raydium AMM V4" => 21_200,
        "Meteora DAMM V2" => 22_500,
        "Meteora DLMM" => 45_600,
        "Raydium CLMM" => 110_000,
        "Meteora DAMM V1" => 115_000,
        "Pumpfun AMM" => 130_000,
        // Unknown DEX: be generous.
        _ => 150_000,
    }
}

/// Compute-unit limit for a route: 30k base (router overhead + ATA creation
/// + WSOL wrap) plus the measured per-hop CU with a 1.2x safety margin,
/// clamped to [120k, 1.4M].
pub fn route_compute_unit_limit(route: &Route) -> u32 {
    let hops_cu: u64 = route.hops.iter().map(|h| hop_compute_units(&h.dex_name)).sum();
    let cu = 30_000 + hops_cu * 12 / 10;
    (cu as u32).clamp(MIN_COMPUTE_UNIT_LIMIT, MAX_COMPUTE_UNIT_LIMIT)
}

const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// ComputeBudget::SetComputeUnitLimit (discriminant 2).
pub fn set_compute_unit_limit_ix(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2u8);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: Pubkey::from_str_const(COMPUTE_BUDGET_PROGRAM),
        accounts: vec![],
        data,
    }
}

/// ComputeBudget::SetComputeUnitPrice (discriminant 3), µ-lamports per CU.
pub fn set_compute_unit_price_ix(micro_lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3u8);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction {
        program_id: Pubkey::from_str_const(COMPUTE_BUDGET_PROGRAM),
        accounts: vec![],
        data,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Knobs for `build_swap_instructions`.
#[derive(Debug, Clone)]
pub struct SwapOptions {
    /// Wrap SOL into WSOL before the swap and close the WSOL ATA afterwards
    /// (both input and output side). Default true.
    pub wrap_and_unwrap_sol: bool,
    /// Compute-unit price in µ-lamports. Default `DEFAULT_COMPUTE_UNIT_PRICE`.
    pub compute_unit_price_micro_lamports: u64,
    /// Explicit compute-unit limit override; `None` uses the per-hop table.
    pub compute_unit_limit: Option<u32>,
}

impl Default for SwapOptions {
    fn default() -> Self {
        Self {
            wrap_and_unwrap_sol: true,
            compute_unit_price_micro_lamports: DEFAULT_COMPUTE_UNIT_PRICE,
            compute_unit_limit: None,
        }
    }
}

/// The instruction bundle for one swap, split Jupiter-style so clients can
/// recompose it (`POST /swap-instructions`) or compile it directly
/// (`compile_to_transaction`).
#[derive(Debug, Clone)]
pub struct SwapIxBundle {
    /// SetComputeUnitLimit + SetComputeUnitPrice.
    pub compute_budget: Vec<Instruction>,
    /// WSOL wrap (when input is SOL) + idempotent ATA creation for each
    /// intermediate/output mint.
    pub setup: Vec<Instruction>,
    /// The single compact thunder-router instruction.
    pub swap: Instruction,
    /// WSOL ATA close (input and/or output side).
    pub cleanup: Vec<Instruction>,
    /// The compute-unit limit set in `compute_budget`.
    pub compute_unit_limit: u32,
    /// The compute-unit price set in `compute_budget` (µ-lamports).
    pub compute_unit_price_micro_lamports: u64,
}

/// Build the full instruction bundle for a swap route.
///
/// `min_amount_out` is enforced on-chain by the router; callers using a
/// quoted route derive it via `calculate_min_amount_out`, Jupiter-mirror
/// callers pass `otherAmountThreshold` verbatim.
pub fn build_swap_instructions(
    route: &Route,
    user: &Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    store: &AccountStore,
    registry: &PoolRegistry,
    opts: &SwapOptions,
) -> Result<SwapIxBundle, GenericError> {
    if route.hops.is_empty() {
        return Err("Route has no hops".into());
    }

    let wsol = Pubkey::from_str_const(WSOL);
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let input_mint = route.hops.first().unwrap().input_mint;
    let output_mint = route.hops.last().unwrap().output_mint;
    let input_is_sol = input_mint == wsol && opts.wrap_and_unwrap_sol;
    let output_is_sol = output_mint == wsol && opts.wrap_and_unwrap_sol;

    // Detect token programs for all mints.
    let mut all_mints: Vec<Pubkey> = Vec::new();
    for hop in &route.hops {
        if !all_mints.contains(&hop.input_mint) { all_mints.push(hop.input_mint); }
        if !all_mints.contains(&hop.output_mint) { all_mints.push(hop.output_mint); }
    }
    let mint_programs = detect_mint_programs(store, &all_mints);

    // --- Compute budget ---
    let compute_unit_limit = opts
        .compute_unit_limit
        .unwrap_or_else(|| route_compute_unit_limit(route));
    let compute_budget = vec![
        set_compute_unit_limit_ix(compute_unit_limit),
        set_compute_unit_price_ix(opts.compute_unit_price_micro_lamports),
    ];

    // --- Setup ---
    let mut setup: Vec<Instruction> = Vec::new();

    // WSOL wrap.
    if input_is_sol {
        let wsol_ata = get_associated_token_address_with_program_id(user, &wsol, &tp);
        setup.push(create_associated_token_account_idempotent(user, user, &wsol, &tp));
        setup.push(system_ix::transfer(user, &wsol_ata, amount_in));
        setup.push(sync_native(&tp, &wsol_ata).map_err(|e| format!("sync_native: {e}"))?);
    }

    // ATA creation for intermediate and output mints (idempotent; skips a
    // WSOL output ATA already created by the wrap above).
    let mut created: Vec<Pubkey> = if input_is_sol { vec![wsol] } else { Vec::new() };
    for hop in &route.hops {
        if created.contains(&hop.output_mint) {
            continue;
        }
        created.push(hop.output_mint);
        let prog = mint_program(&mint_programs, &hop.output_mint, &tp);
        setup.push(create_associated_token_account_idempotent(user, user, &hop.output_mint, &prog));
    }

    // --- The compact router instruction ---
    let swap = build_router_instruction(route, user, amount_in, min_amount_out, store, registry)?;

    // --- Cleanup: close the WSOL ATA (once, even for SOL round trips) ---
    let mut cleanup: Vec<Instruction> = Vec::new();
    if input_is_sol || output_is_sol {
        let wsol_ata = get_associated_token_address_with_program_id(user, &wsol, &tp);
        cleanup.push(
            close_account(&tp, &wsol_ata, user, user, &[])
                .map_err(|e| format!("close_account: {e}"))?,
        );
    }

    Ok(SwapIxBundle {
        compute_budget,
        setup,
        swap,
        cleanup,
        compute_unit_limit,
        compute_unit_price_micro_lamports: opts.compute_unit_price_micro_lamports,
    })
}

/// Compile a bundle into an unsigned v0 `VersionedTransaction`, resolving
/// addresses through the given lookup tables. Fails with a descriptive error
/// when the serialized transaction exceeds `MAX_TX_SIZE` (1232 bytes).
pub fn compile_to_transaction(
    bundle: &SwapIxBundle,
    user: &Pubkey,
    alts: &[solana_sdk::message::AddressLookupTableAccount],
    recent_blockhash: Hash,
) -> Result<VersionedTransaction, GenericError> {
    let mut ixs: Vec<Instruction> =
        Vec::with_capacity(bundle.compute_budget.len() + bundle.setup.len() + 1 + bundle.cleanup.len());
    ixs.extend(bundle.compute_budget.iter().cloned());
    ixs.extend(bundle.setup.iter().cloned());
    ixs.push(bundle.swap.clone());
    ixs.extend(bundle.cleanup.iter().cloned());

    let message = v0::Message::try_compile(user, &ixs, alts, recent_blockhash)
        .map_err(|e| format!("compile message: {e}"))?;
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(message),
    };

    let size = bincode::serialize(&tx)
        .map_err(|e| format!("serialize transaction: {e}"))?
        .len();
    if size > MAX_TX_SIZE {
        return Err(format!(
            "transaction too large: {size} bytes > {MAX_TX_SIZE} \
             ({} swap accounts, {} lookup table{})",
            bundle.swap.accounts.len(),
            alts.len(),
            if alts.len() == 1 { "" } else { "s" },
        )
        .into());
    }
    Ok(tx)
}

/// Build a complete unsigned VersionedTransaction for a swap route
/// (no ALT; default options). Thin wrapper over `build_swap_instructions`
/// + `compile_to_transaction`.
pub fn build_swap_transaction(
    route: &Route,
    user: &Pubkey,
    amount_in: u64,
    slippage_bps: u64,
    store: &AccountStore,
    registry: &PoolRegistry,
    recent_blockhash: Hash,
) -> Result<VersionedTransaction, GenericError> {
    let min_amount_out = calculate_min_amount_out(route.output_amount, slippage_bps);
    let bundle = build_swap_instructions(
        route, user, amount_in, min_amount_out, store, registry, &SwapOptions::default(),
    )?;
    compile_to_transaction(&bundle, user, &[], recent_blockhash)
}

// ---------------------------------------------------------------------------
// Router hop builder
// ---------------------------------------------------------------------------

/// Build the single compact router `Instruction` for a route: borsh
/// `ExecuteRouteArgs` data plus every hop's adapter accounts in order.
/// Exposed for tests (negative hardening tests mutate the account list).
pub fn build_router_instruction(
    route: &Route,
    user: &Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    store: &AccountStore,
    registry: &PoolRegistry,
) -> Result<Instruction, GenericError> {
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let mut all_mints: Vec<Pubkey> = Vec::new();
    for hop in &route.hops {
        if !all_mints.contains(&hop.input_mint) { all_mints.push(hop.input_mint); }
        if !all_mints.contains(&hop.output_mint) { all_mints.push(hop.output_mint); }
    }
    let mint_programs = detect_mint_programs(store, &all_mints);

    let (hops, all_accounts) = build_router_hops(
        route, user, store, registry, &mint_programs, &tp,
    )?;

    let args = ExecuteRouteArgs {
        amount_in,
        min_amount_out,
        amounts: vec![amount_in],
        paths: vec![PathArgs { hops }],
    };
    let data = borsh::to_vec(&args).map_err(|e| format!("borsh serialize: {e}"))?;

    Ok(Instruction {
        program_id: router_program_id()?,
        accounts: all_accounts,
        data,
    })
}

/// Iterate route hops, collect per-adapter accounts, and produce the compact
/// hop descriptors the router program expects.
fn build_router_hops(
    route: &Route,
    user: &Pubkey,
    store: &AccountStore,
    registry: &PoolRegistry,
    mint_programs: &[(Pubkey, Pubkey)],
    tp: &Pubkey,
) -> Result<(Vec<HopArgs>, Vec<AccountMeta>), GenericError> {
    let mut hops = Vec::new();
    let mut all_accounts = Vec::new();

    for hop in &route.hops {
        let pool_pubkey = hop.pool_address.parse::<Pubkey>()
            .map_err(|e| format!("invalid pool address {}: {e}", hop.pool_address))?;
        let pool_data = store.get_data(&pool_pubkey)
            .ok_or_else(|| format!("no pool data for {}", pool_pubkey))?;

        let in_prog = mint_program(mint_programs, &hop.input_mint, tp);
        let out_prog = mint_program(mint_programs, &hop.output_mint, tp);
        let user_in = get_associated_token_address_with_program_id(user, &hop.input_mint, &in_prog);
        let user_out = get_associated_token_address_with_program_id(user, &hop.output_mint, &out_prog);

        let (dex_type, accounts) = match hop.dex_name.as_str() {
            "Meteora DAMM V2" => {
                let metas = collect_damm_v2_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out, &hop.input_mint,
                );
                (DexType::MeteoraDAMMV2, metas)
            }
            "Meteora DAMM V1" => {
                let a_vault = pubkey_at(&pool_data, 104);
                let b_vault = pubkey_at(&pool_data, 136);
                let a_vault_data = store.get_data(&a_vault)
                    .ok_or_else(|| format!("DAMM V1 a_vault {a_vault} not in account store"))?;
                let b_vault_data = store.get_data(&b_vault)
                    .ok_or_else(|| format!("DAMM V1 b_vault {b_vault} not in account store"))?;
                let metas = collect_damm_v1_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out, &hop.input_mint,
                    &a_vault_data, &b_vault_data,
                )?;
                (DexType::MeteoraDAMMV1, metas)
            }
            "Meteora DLMM" => {
                let bitmap_ext = registry.get_pool(&pool_pubkey.to_string())
                    .and_then(|info| info.bitmap_ext);
                let metas = collect_dlmm_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out,
                    &hop.input_mint, in_prog, out_prog, bitmap_ext,
                )?;
                (DexType::MeteoraDLMM, metas)
            }
            "Raydium CLMM" => {
                let ext_data = store.get_data(&clmm_bitmap_extension_pda(&pool_pubkey));
                let metas = collect_clmm_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out,
                    &hop.input_mint, ext_data.as_deref(),
                )?;
                (DexType::RaydiumCLMM, metas)
            }
            "Raydium AMM V4" => {
                let metas = collect_ray_v4_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out,
                );
                (DexType::RaydiumAMMV4, metas)
            }
            "Pumpfun AMM" => {
                let quote_mint = pubkey_at(&pool_data, 75);
                let is_buy = hop.input_mint == quote_mint;
                let dex = if is_buy { DexType::PumpfunBuy } else { DexType::PumpfunSell };
                let config_data = store.get_data(&pumpfun_global_config_pda())
                    .ok_or("pumpfun global config not in account store")?;
                let metas = collect_pumpfun_accounts(
                    &pool_data, pool_pubkey, *user, user_in, user_out,
                    &hop.input_mint, in_prog, out_prog, &config_data,
                )?;
                (dex, metas)
            }
            other => return Err(format!("unsupported DEX: {other}").into()),
        };

        hops.push(HopArgs {
            legs: vec![SwapLeg {
                dex_type,
                num_accounts: accounts.len() as u8,
                weight: 100,
            }],
        });
        all_accounts.extend(accounts);
    }

    Ok((hops, all_accounts))
}

// ---------------------------------------------------------------------------
// Per-DEX account collection
// ---------------------------------------------------------------------------

/// DAMM V2 — 13 accounts.
pub fn collect_damm_v2_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    _input_mint: &Pubkey,
) -> Vec<AccountMeta> {
    let dex = Pubkey::from_str_const(DAMM_V2_PROGRAM);
    let pool_authority = Pubkey::from_str_const(DAMM_V2_POOL_AUTHORITY);
    let (event_auth, _) = Pubkey::find_program_address(&[b"__event_authority"], &dex);

    let token_a_mint = pubkey_at(pool_data, 168);
    let token_b_mint = pubkey_at(pool_data, 200);
    let token_a_vault = pubkey_at(pool_data, 232);
    let token_b_vault = pubkey_at(pool_data, 264);

    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let tp22 = Pubkey::from_str_const(TOKEN_PROGRAM_2022);
    let token_a_program = if pool_data[482] == 1 { tp22 } else { tp };
    let token_b_program = if pool_data[483] == 1 { tp22 } else { tp };

    vec![
        AccountMeta::new_readonly(dex, false),             // [0]  dex_program
        AccountMeta::new(user, true),                       // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),             // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),            // [3]  swap_dest_token
        AccountMeta::new_readonly(pool_authority, false),   // [4]  pool_authority
        AccountMeta::new(pool_pubkey, false),               // [5]  pool
        AccountMeta::new(token_a_vault, false),             // [6]  token_a_vault
        AccountMeta::new(token_b_vault, false),             // [7]  token_b_vault
        AccountMeta::new_readonly(token_a_mint, false),     // [8]  token_a_mint
        AccountMeta::new_readonly(token_b_mint, false),     // [9]  token_b_mint
        AccountMeta::new_readonly(token_a_program, false),  // [10] token_a_program
        AccountMeta::new_readonly(token_b_program, false),  // [11] token_b_program
        AccountMeta::new_readonly(event_auth, false),       // [12] event_authority
    ]
}

/// DAMM V1 — 16 accounts.
///
/// `a_vault_data` / `b_vault_data` are the raw account bytes of the pool's
/// two dynamic-vault state accounts (their `token_vault` and `lp_mint`
/// fields are read directly; the lp mint is not always PDA-derivable).
pub fn collect_damm_v1_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    input_mint: &Pubkey,
    a_vault_data: &[u8],
    b_vault_data: &[u8],
) -> Result<Vec<AccountMeta>, GenericError> {
    let dex = Pubkey::from_str_const(DAMM_V1_PROGRAM);
    let vault_program = Pubkey::from_str_const(VAULT_PROGRAM);
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);

    let token_a_mint = pubkey_at(pool_data, 40);
    let a_vault = pubkey_at(pool_data, 104);
    let b_vault = pubkey_at(pool_data, 136);

    // Vault state layout (after the 8-byte discriminator): enabled(1),
    // bumps(2), total_amount(8), token_vault @19, fee_vault @51,
    // token_mint @83, lp_mint @115.
    if a_vault_data.len() < 147 || b_vault_data.len() < 147 {
        return Err(format!("DAMM V1 pool {pool_pubkey}: vault account data too short").into());
    }
    let a_token_vault = pubkey_at(a_vault_data, 19);
    let a_vault_lp_mint = pubkey_at(a_vault_data, 115);
    let b_token_vault = pubkey_at(b_vault_data, 19);
    let b_vault_lp_mint = pubkey_at(b_vault_data, 115);

    let a_vault_lp = pubkey_at(pool_data, 168);
    let b_vault_lp = pubkey_at(pool_data, 200);
    // Protocol fee token account of the *input* token:
    // protocol_token_a_fee @234, protocol_token_b_fee @266.
    let protocol_token_fee = if *input_mint == token_a_mint {
        pubkey_at(pool_data, 234)
    } else {
        pubkey_at(pool_data, 266)
    };

    Ok(vec![
        AccountMeta::new_readonly(dex, false),            // [0]  dex_program
        AccountMeta::new(user, true),                      // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),            // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),           // [3]  swap_dest_token
        AccountMeta::new(pool_pubkey, false),              // [4]  pool
        AccountMeta::new(a_vault, false),                  // [5]  a_vault
        AccountMeta::new(b_vault, false),                  // [6]  b_vault
        AccountMeta::new(a_token_vault, false),            // [7]  a_token_vault
        AccountMeta::new(b_token_vault, false),            // [8]  b_token_vault
        AccountMeta::new(a_vault_lp_mint, false),          // [9]  a_vault_lp_mint
        AccountMeta::new(b_vault_lp_mint, false),          // [10] b_vault_lp_mint
        AccountMeta::new(a_vault_lp, false),               // [11] a_vault_lp
        AccountMeta::new(b_vault_lp, false),               // [12] b_vault_lp
        AccountMeta::new(protocol_token_fee, false),       // [13] admin_token_fee
        AccountMeta::new_readonly(vault_program, false),   // [14] vault_program
        AccountMeta::new_readonly(tp, false),              // [15] token_program
    ])
}

/// DLMM Swap2 — 16 fixed accounts + 1..=3 direction-ordered bin arrays.
pub fn collect_dlmm_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    input_mint: &Pubkey,
    in_program: Pubkey,
    out_program: Pubkey,
    bitmap_ext: Option<Pubkey>,
) -> Result<Vec<AccountMeta>, GenericError> {
    let dex = Pubkey::from_str_const(DLMM_PROGRAM);
    let event_auth = Pubkey::from_str_const(DLMM_EVENT_AUTHORITY);
    let memo = Pubkey::from_str_const(MEMO_PROGRAM);

    let pool: meteora_dlmm::MeteoraDLMMPool = deser_anchor(pool_data)
        .map_err(|e| format!("DLMM pool {pool_pubkey}: {e}"))?;

    // Bitmap extension: swap2 declares it `mut`, so pass it writable when
    // present; the dex program id doubles as Anchor's None marker (readonly).
    let bitmap_meta = match bitmap_ext {
        Some(ext) => AccountMeta::new(ext, false),
        None => AccountMeta::new_readonly(dex, false),
    };

    // Token program ordering depends on which mint is token_x.
    let (token_x_program, token_y_program) = if *input_mint == pool.token_x_mint {
        (in_program, out_program)
    } else {
        (out_program, in_program)
    };

    // Oracle PDA.
    let (oracle, _) = Pubkey::find_program_address(
        &[b"oracle", pool_pubkey.as_ref()], &dex,
    );

    // Direction-ordered bin arrays: swapping X in (for Y) walks the active
    // bin downward; swapping Y in walks upward. Pass up to 3 initialized
    // arrays starting from the active one, per the lb_pair's in-pool bitmap.
    let swap_for_y = *input_mint == pool.token_x_mint;
    let bin_arrays = derive_dlmm_swap_bin_arrays(
        &pool.bin_array_bitmap,
        pool.active_id,
        swap_for_y,
        &pool_pubkey,
        &dex,
    );
    if bin_arrays.is_empty() {
        return Err(format!("DLMM pool {pool_pubkey}: no initialized bin arrays").into());
    }

    let mut metas = vec![
        AccountMeta::new_readonly(dex, false),             // [0]  dex_program
        AccountMeta::new(user, true),                       // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),             // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),            // [3]  swap_dest_token
        AccountMeta::new(pool_pubkey, false),               // [4]  lb_pair
        bitmap_meta,                                        // [5]  bitmap_extension
        AccountMeta::new(pool.reserve_x, false),            // [6]  reserve_x
        AccountMeta::new(pool.reserve_y, false),            // [7]  reserve_y
        AccountMeta::new_readonly(pool.token_x_mint, false),// [8]  token_x_mint
        AccountMeta::new_readonly(pool.token_y_mint, false),// [9]  token_y_mint
        AccountMeta::new(oracle, false),                    // [10] oracle (writable!)
        AccountMeta::new(dex, false),                       // [11] host_fee_in (writable!)
        AccountMeta::new_readonly(token_x_program, false),  // [12] token_x_program
        AccountMeta::new_readonly(token_y_program, false),  // [13] token_y_program
        AccountMeta::new_readonly(memo, false),             // [14] memo_program
        AccountMeta::new_readonly(event_auth, false),       // [15] event_authority
    ];
    for bin_array in bin_arrays {
        metas.push(AccountMeta::new(bin_array, false));     // [16..] bin arrays
    }
    Ok(metas)
}

/// Walk the lb_pair's in-pool bin array bitmap (indices -512..=511, bit =
/// index + 512) in the swap direction and return up to 3 initialized bin
/// array PDAs, starting from the active one.
fn derive_dlmm_swap_bin_arrays(
    bitmap: &[u64; 16],
    active_id: i32,
    swap_for_y: bool,
    pool_pubkey: &Pubkey,
    dex: &Pubkey,
) -> Vec<Pubkey> {
    let start = (active_id as i64).div_euclid(70);
    let step: i64 = if swap_for_y { -1 } else { 1 };
    let mut out = Vec::with_capacity(3);
    let mut idx = start;
    while out.len() < 3 && (-512..=511).contains(&idx) {
        let pos = (idx + 512) as usize;
        if bitmap[pos / 64] & (1u64 << (pos % 64)) != 0 {
            let (pda, _) = Pubkey::find_program_address(
                &[b"bin_array", pool_pubkey.as_ref(), &idx.to_le_bytes()], dex,
            );
            out.push(pda);
        }
        idx += step;
    }
    out
}

/// Raydium CLMM SwapV2 — 15 fixed accounts + 1..=3 swap-ordered tick arrays.
///
/// `bitmap_ext_data` is the raw account data of the pool's
/// TickArrayBitmapExtension (PDA `["pool_tick_array_bitmap_extension", pool]`),
/// needed to find initialized tick arrays outside the in-pool bitmap (±512).
pub fn collect_clmm_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    input_mint: &Pubkey,
    bitmap_ext_data: Option<&[u8]>,
) -> Result<Vec<AccountMeta>, GenericError> {
    let dex = Pubkey::from_str_const(CLMM_PROGRAM);
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let tp22 = Pubkey::from_str_const(TOKEN_PROGRAM_2022);
    let memo = Pubkey::from_str_const(MEMO_PROGRAM);

    let pool: raydium_clmm::RaydiumCLMMPool = deser_anchor(pool_data)
        .map_err(|e| format!("CLMM pool {pool_pubkey}: {e}"))?;

    // Determine direction: input == mint0 means zero-for-one (a→b, price and
    // tick decrease), which is `is_buy` in compute_clmm_remaining_accounts'
    // vocabulary.
    let zero_for_one = *input_mint == pool.token_mint_0;
    let (input_vault, output_vault, input_mint_key, output_mint_key) = if zero_for_one {
        (pool.token_vault_0, pool.token_vault_1, pool.token_mint_0, pool.token_mint_1)
    } else {
        (pool.token_vault_1, pool.token_vault_0, pool.token_mint_1, pool.token_mint_0)
    };

    // Swap-ordered initialized tick arrays (1..=3), walked from the current
    // tick in the swap direction via the pool bitmap (+ extension).
    let tick_arrays = raydium_clmm::tick_arrays::compute_clmm_remaining_accounts(
        &pool, &pool_pubkey, zero_for_one, bitmap_ext_data,
    ).map_err(|e| format!("CLMM pool {pool_pubkey}: {e}"))?;

    let ex_bitmap = clmm_bitmap_extension_pda(&pool_pubkey);

    let mut metas = vec![
        AccountMeta::new_readonly(dex, false),             // [0]  dex_program
        AccountMeta::new(user, true),                       // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),             // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),            // [3]  swap_dest_token
        AccountMeta::new_readonly(pool.amm_config, false),  // [4]  amm_config
        AccountMeta::new(pool_pubkey, false),               // [5]  pool
        AccountMeta::new(input_vault, false),               // [6]  input_vault
        AccountMeta::new(output_vault, false),              // [7]  output_vault
        AccountMeta::new(pool.observation_key, false),      // [8]  observation
        AccountMeta::new_readonly(tp, false),               // [9]  token_program
        AccountMeta::new_readonly(tp22, false),             // [10] token_program_2022
        AccountMeta::new_readonly(memo, false),             // [11] memo_program
        AccountMeta::new_readonly(input_mint_key, false),   // [12] input_vault_mint
        AccountMeta::new_readonly(output_mint_key, false),  // [13] output_vault_mint
        AccountMeta::new(ex_bitmap, false),                 // [14] tick_array_bitmap_extension
    ];
    for tick_array in tick_arrays {
        metas.push(AccountMeta::new(tick_array, false));    // [15..] tick arrays
    }
    Ok(metas)
}

/// Raydium AMM V4 — 9 accounts (`SwapBaseInV2`, the orderbook-free swap).
///
/// The engine only routes status-6 (SwapOnly) pools, for which the on-chain
/// program rejects the legacy orderbook path and requires this 8-account CPI
/// form (no OpenBook market accounts at all).
pub fn collect_ray_v4_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
) -> Vec<AccountMeta> {
    let dex = Pubkey::from_str_const(RAY_V4_PROGRAM);
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let authority = Pubkey::from_str_const(RAY_V4_AUTHORITY);

    let coin_vault = pubkey_at(pool_data, 336);
    let pc_vault = pubkey_at(pool_data, 368);

    vec![
        AccountMeta::new_readonly(dex, false),             // [0]  dex_program
        AccountMeta::new(user, true),                       // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),             // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),            // [3]  swap_dest_token
        AccountMeta::new_readonly(tp, false),               // [4]  token_program
        AccountMeta::new(pool_pubkey, false),               // [5]  amm_id
        AccountMeta::new_readonly(authority, false),        // [6]  amm_authority
        AccountMeta::new(coin_vault, false),                // [7]  pool_coin_vault
        AccountMeta::new(pc_vault, false),                  // [8]  pool_pc_vault
    ]
}

/// Pumpfun AMM — 25 accounts for buys, 23 for sells (buys additionally carry
/// the two volume-accumulator accounts). Layout verified against live
/// mainnet pAMMBay buy/sell transactions plus the on-chain IDL.
///
/// `global_config_data` is the raw account data of the AMM's GlobalConfig
/// PDA; the protocol- and buyback-fee recipients are read from it.
pub fn collect_pumpfun_accounts(
    pool_data: &[u8],
    pool_pubkey: Pubkey,
    user: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    input_mint: &Pubkey,
    in_program: Pubkey,
    out_program: Pubkey,
    global_config_data: &[u8],
) -> Result<Vec<AccountMeta>, GenericError> {
    let dex = Pubkey::from_str_const(PUMPFUN_PROGRAM);
    let fee_program = Pubkey::from_str_const(pumpfun_amm::PUMPFUN_FEE_PROGRAM);
    let system_program = Pubkey::from_str_const(SYSTEM_PROGRAM);
    let ata_program = Pubkey::from_str_const(ATA_PROGRAM);
    let (event_auth, _) = Pubkey::find_program_address(&[b"__event_authority"], &dex);

    let base_mint = pubkey_at(pool_data, 43);
    let quote_mint = pubkey_at(pool_data, 75);
    let pool_base_vault = pubkey_at(pool_data, 139);
    let pool_quote_vault = pubkey_at(pool_data, 171);
    let coin_creator = pubkey_at(pool_data, 211);
    let is_mayhem_mode = pool_data.get(243).copied().unwrap_or(0) == 1;

    // GlobalConfig layout (after the 8-byte discriminator): admin(32),
    // lp_fee_bps(8), protocol_fee_bps(8), disable_flags(1),
    // protocol_fee_recipients[8](256) @57, coin_creator_fee_bps(8),
    // admin_set_coin_creator_authority(32), whitelist_pda(32),
    // reserved_fee_recipient(32) @385, mayhem_mode_enabled(1),
    // reserved_fee_recipients[7](224), is_cashback_enabled(1),
    // buyback_fee_recipients[8](256) @643, buyback_basis_points(8).
    if global_config_data.len() < 907 {
        return Err(format!(
            "pumpfun global config: expected >= 907 bytes, got {}",
            global_config_data.len()
        ).into());
    }
    // Mayhem-mode pools must pay protocol fees to a reserved recipient.
    let protocol_fee_recipient = if is_mayhem_mode {
        pubkey_at(global_config_data, 385)
    } else {
        pubkey_at(global_config_data, 57)
    };
    let buyback_fee_recipient = pubkey_at(global_config_data, 643);

    // Direction determines token program ordering: buy means input=quote.
    let is_buy = *input_mint == quote_mint;
    let (base_token_program, quote_token_program) = if is_buy {
        (out_program, in_program)
    } else {
        (in_program, out_program)
    };

    let protocol_fee_recipient_ata = get_associated_token_address_with_program_id(
        &protocol_fee_recipient, &quote_mint, &quote_token_program,
    );
    let buyback_fee_recipient_ata = get_associated_token_address_with_program_id(
        &buyback_fee_recipient, &quote_mint, &quote_token_program,
    );
    let creator_vault_authority =
        pumpfun_amm::pda::get_pumpfun_creator_vault_authority_pda(coin_creator);
    let creator_vault_ata = get_associated_token_address_with_program_id(
        &creator_vault_authority, &quote_mint, &quote_token_program,
    );

    let mut metas = vec![
        AccountMeta::new_readonly(dex, false),               // [0]  dex_program
        AccountMeta::new(user, true),                         // [1]  swap_authority (signer)
        AccountMeta::new(user_token_in, false),               // [2]  swap_source_token
        AccountMeta::new(user_token_out, false),              // [3]  swap_dest_token
        AccountMeta::new(pool_pubkey, false),                 // [4]  pool
        AccountMeta::new_readonly(pumpfun_global_config_pda(), false), // [5] global_config
        AccountMeta::new_readonly(base_mint, false),          // [6]  base_mint
        AccountMeta::new_readonly(quote_mint, false),         // [7]  quote_mint
        AccountMeta::new(pool_base_vault, false),             // [8]  pool_base_vault
        AccountMeta::new(pool_quote_vault, false),            // [9]  pool_quote_vault
        AccountMeta::new_readonly(protocol_fee_recipient, false), // [10] protocol_fee_recipient
        AccountMeta::new(protocol_fee_recipient_ata, false),  // [11] protocol_fee_recipient_ata
        AccountMeta::new_readonly(base_token_program, false), // [12] base_token_program
        AccountMeta::new_readonly(quote_token_program, false),// [13] quote_token_program
        AccountMeta::new_readonly(system_program, false),     // [14] system_program
        AccountMeta::new_readonly(ata_program, false),        // [15] associated_token_program
        AccountMeta::new_readonly(event_auth, false),         // [16] event_authority
        AccountMeta::new(creator_vault_ata, false),           // [17] coin_creator_vault_ata
        AccountMeta::new_readonly(creator_vault_authority, false), // [18] coin_creator_vault_authority
    ];
    if is_buy {
        metas.push(AccountMeta::new_readonly(
            pumpfun_amm::pda::get_global_volume_accumulator_pda(), false,
        ));                                                   // [19] global_volume_accumulator
        metas.push(AccountMeta::new(
            pumpfun_amm::pda::get_user_volume_accumulator_pda(user), false,
        ));                                                   // [20] user_volume_accumulator
    }
    metas.push(AccountMeta::new_readonly(
        pumpfun_amm::pda::get_pumpfun_config_pda(), false,
    ));                                                       // fee_config
    metas.push(AccountMeta::new_readonly(fee_program, false)); // fee_program
    metas.push(AccountMeta::new_readonly(buyback_fee_recipient, false)); // buyback_fee_recipient
    metas.push(AccountMeta::new(buyback_fee_recipient_ata, false)); // buyback_fee_recipient_ata
    Ok(metas)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a 32-byte Pubkey from raw account data at the given byte offset.
fn pubkey_at(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

/// Deserialize an Anchor account, skipping the 8-byte discriminator.
/// Tolerates trailing bytes.
fn deser_anchor<T: BorshDeserialize>(data: &[u8]) -> Result<T, GenericError> {
    if data.len() < 8 {
        return Err("account data too short".into());
    }
    let mut slice = &data[8..];
    T::deserialize(&mut slice).map_err(|e| e.into())
}

fn detect_mint_programs(store: &AccountStore, mints: &[Pubkey]) -> Vec<(Pubkey, Pubkey)> {
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let tp22 = Pubkey::from_str_const(TOKEN_PROGRAM_2022);
    mints.iter().map(|mint| {
        let prog = store.get(mint)
            .map(|acc| if acc.owner == tp22 { tp22 } else { tp })
            .unwrap_or(tp);
        (*mint, prog)
    }).collect()
}

fn mint_program(programs: &[(Pubkey, Pubkey)], mint: &Pubkey, default: &Pubkey) -> Pubkey {
    programs.iter()
        .find(|(m, _)| m == mint)
        .map(|(_, p)| *p)
        .unwrap_or(*default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden wire bytes shared with crates/router-program/src/lib.rs.
    /// If this test or its router-program twin fails, the two borsh
    /// definitions have drifted.
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

    #[test]
    fn golden_bytes_match_router_program() {
        let args = ExecuteRouteArgs {
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
        };
        let ser = borsh::to_vec(&args).unwrap();
        assert_eq!(ser.as_slice(), GOLDEN_BYTES);
    }
}
