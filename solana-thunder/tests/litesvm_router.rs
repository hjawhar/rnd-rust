//! LiteSVM integration tests: production `build_swap_transaction` executed
//! against mainnet fixtures through the on-chain thunder-router.
//!
//! Prerequisites (skipped gracefully when absent, so CI stays green):
//!   1. `scripts/build-router.sh` (router .so)
//!   2. `cargo run --bin thunder-fixtures -- <damm_v2_pool> <dlmm_pool>`
//!      (dumps fixtures/accounts, fixtures/manifest.json, fixtures/programs)
//!
//! Run: `cargo test --test litesvm_router -- --nocapture`

#[allow(dead_code)]
mod helpers;

use helpers::litesvm_env::FixtureEnv;
use litesvm::types::TransactionMetadata;
use solana_pubkey::Pubkey;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    message::{v0, AddressLookupTableAccount, VersionedMessage},
    signature::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_ix;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use spl_token::instruction::sync_native;
use thunder_core::{calculate_min_amount_out, TOKEN_PROGRAM, WSOL};
use thunder_engine::swap::{
    build_router_instruction, build_swap_instructions, build_swap_transaction,
    compile_to_transaction, SwapOptions,
};

const SLIPPAGE_BPS: u64 = 500;
const ONE_SOL: u64 = 1_000_000_000;
/// LiteSVM base fee for a single-signature transaction.
const TX_FEE: u64 = 5_000;
/// Rent for the 137-byte pumpfun user_volume_accumulator the AMM creates
/// (init_if_needed, paid by the user) on the first buy.
const PUMPFUN_UVA_RENT: u64 = 1_844_400;

// ---------------------------------------------------------------------------
// Verified adapters (DAMM V2 + DLMM)
// ---------------------------------------------------------------------------

#[test]
fn damm_v2_single_hop_sol_to_usdc() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Meteora DAMM V2") else {
        eprintln!("skipping: no Meteora DAMM V2 pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

#[test]
fn dlmm_single_hop_sol_to_usdc() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Meteora DLMM") else {
        eprintln!("skipping: no Meteora DLMM pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

/// 2-hop cross-DEX chain: SOL -> USDC on DAMM V2, then USDC -> SOL on DLMM.
/// Exercises the router's actual-output delta-chaining between hops.
#[test]
fn two_hop_damm_v2_then_dlmm() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let (Some(damm_v2), Some(dlmm)) = (
        env.pool_by_dex("Meteora DAMM V2"),
        env.pool_by_dex("Meteora DLMM"),
    ) else {
        eprintln!("skipping: need both a DAMM V2 and a DLMM pool in fixtures");
        return;
    };

    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[&damm_v2, &dlmm], wsol, ONE_SOL);
    assert_eq!(route.output_mint, wsol, "round trip must end in WSOL");
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);

    let tx = build_swap_transaction(
        &route,
        &env.user.pubkey(),
        ONE_SOL,
        SLIPPAGE_BPS,
        &env.store,
        &env.registry,
        env.svm.latest_blockhash(),
    )
    .expect("build_swap_transaction");

    // Output is WSOL: the builder closes the WSOL ATA post-swap, so the
    // output lands back in the user's lamport balance.
    let lamports_before = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);
    let meta = env.sign_and_send(tx).expect("2-hop swap failed");
    let lamports_after = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);

    println!(
        "2-hop DAMM V2 -> DLMM: in={ONE_SOL} quoted={} min_out={} lamports_delta={} compute_units_consumed={}",
        route.output_amount,
        min_out,
        lamports_after as i128 - lamports_before as i128,
        meta.compute_units_consumed,
    );

    // delta >= min_out, modulo the wrapped input and the tx fee (both user
    // ATAs pre-exist or are rent-round-tripped, so no other rent is paid).
    assert!(
        lamports_after + ONE_SOL + TX_FEE >= lamports_before + min_out,
        "output below min_out: before={lamports_before} after={lamports_after} min_out={min_out}"
    );
}

/// Shared body for the SOL -> token single-hop tests.
fn single_hop_from_sol(env: &mut FixtureEnv, pool: &str) {
    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[pool], wsol, ONE_SOL);
    assert_ne!(route.output_mint, wsol, "expected a SOL/token pool");
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);

    let tx = build_swap_transaction(
        &route,
        &env.user.pubkey(),
        ONE_SOL,
        SLIPPAGE_BPS,
        &env.store,
        &env.registry,
        env.svm.latest_blockhash(),
    )
    .expect("build_swap_transaction");

    let tx_size = bincode::serialize(&tx).map(|b| b.len()).unwrap_or(0);
    let dest_ata = env.user_ata(&route.output_mint);
    let dest_before = env.svm_token_balance(&dest_ata);
    let meta = env.sign_and_send(tx).expect("swap failed");
    let dest_after = env.svm_token_balance(&dest_ata);
    let delta = dest_after - dest_before;

    // Router-instruction-only CU (excludes the ATA-create/wrap overhead).
    let router_cu = meta
        .logs
        .iter()
        .find_map(|log| {
            let rest = log.strip_prefix(&format!(
                "Program {} consumed ",
                helpers::litesvm_env::ROUTER_PROGRAM_ID
            ))?;
            rest.split(' ').next()?.parse::<u64>().ok()
        })
        .unwrap_or(0);

    println!(
        "{} single-hop: in={ONE_SOL} quoted={} min_out={} out={} compute_units_consumed={} router_cu={} tx_size={}",
        route.hops[0].dex_name, route.output_amount, min_out, delta,
        meta.compute_units_consumed, router_cu, tx_size,
    );

    assert!(
        delta >= min_out,
        "output {delta} below min_out {min_out} (quoted {})",
        route.output_amount
    );
}

// ---------------------------------------------------------------------------
// Adapters fixed in M2
// ---------------------------------------------------------------------------

#[test]
fn damm_v1_single_hop_sol_to_usdc() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Meteora DAMM V1") else {
        eprintln!("skipping: no Meteora DAMM V1 pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

#[test]
fn clmm_single_hop_sol_to_usdc() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Raydium CLMM") else {
        eprintln!("skipping: no Raydium CLMM pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

#[test]
fn raydium_v4_single_hop_sol_to_usdc() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Raydium AMM V4") else {
        eprintln!("skipping: no Raydium AMM V4 pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

#[test]
fn pumpfun_single_hop_sol_to_token() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Pumpfun AMM") else {
        eprintln!("skipping: no Pumpfun AMM pool in fixtures/manifest.json");
        return;
    };
    single_hop_from_sol(&mut env, &pool);
}

/// Round trip through the same pumpfun pool: SOL -> token (buy) then
/// token -> SOL (sell). Exercises both pumpfun adapter directions plus the
/// router's delta-chaining between them.
#[test]
fn pumpfun_round_trip_buy_then_sell() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Pumpfun AMM") else {
        eprintln!("skipping: no Pumpfun AMM pool in fixtures/manifest.json");
        return;
    };

    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[&pool, &pool], wsol, ONE_SOL);
    assert_eq!(route.output_mint, wsol, "round trip must end in WSOL");
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);

    let tx = build_swap_transaction(
        &route,
        &env.user.pubkey(),
        ONE_SOL,
        SLIPPAGE_BPS,
        &env.store,
        &env.registry,
        env.svm.latest_blockhash(),
    )
    .expect("build_swap_transaction");

    let lamports_before = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);
    let meta = env.sign_and_send(tx).expect("pumpfun round trip failed");
    let lamports_after = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);

    println!(
        "pumpfun round trip: in={ONE_SOL} quoted={} min_out={} lamports_delta={} compute_units_consumed={}",
        route.output_amount,
        min_out,
        lamports_after as i128 - lamports_before as i128,
        meta.compute_units_consumed,
    );

    // The user also pays rent for the one-time user_volume_accumulator the
    // pumpfun program creates on first buy.
    assert!(
        lamports_after + ONE_SOL + TX_FEE + PUMPFUN_UVA_RENT >= lamports_before + min_out,
        "output below min_out: before={lamports_before} after={lamports_after} min_out={min_out}"
    );
}

// ---------------------------------------------------------------------------
// M3: SwapIxBundle + ALT compile + wSOL output unwrap
// ---------------------------------------------------------------------------

/// Serialize an on-chain ALT account: 56-byte LookupTableMeta (initialized,
/// never deactivated, last extended at slot 1 so every address is active at
/// the pinned fixture slot) followed by the addresses.
fn alt_account_data(authority: &Pubkey, addresses: &[Pubkey]) -> Vec<u8> {
    let mut data = Vec::with_capacity(56 + addresses.len() * 32);
    data.extend_from_slice(&1u32.to_le_bytes()); // type = LookupTable
    data.extend_from_slice(&u64::MAX.to_le_bytes()); // deactivation_slot
    data.extend_from_slice(&1u64.to_le_bytes()); // last_extended_slot
    data.push(0); // last_extended_slot_start_index
    data.push(1); // authority = Some(..)
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(&[0u8; 2]); // padding
    for a in addresses {
        data.extend_from_slice(a.as_ref());
    }
    data
}

/// 2-hop route through the production bundle path, compiled against the
/// curated static ALT (hand-placed in the SVM) and executed. Verifies the
/// size win over the no-ALT compile and that the swap still clears min_out.
#[test]
fn two_hop_bundle_with_alt() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let (Some(damm_v2), Some(dlmm)) = (
        env.pool_by_dex("Meteora DAMM V2"),
        env.pool_by_dex("Meteora DLMM"),
    ) else {
        eprintln!("skipping: need both a DAMM V2 and a DLMM pool in fixtures");
        return;
    };

    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[&damm_v2, &dlmm], wsol, ONE_SOL);
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);
    let bundle = build_swap_instructions(
        &route,
        &env.user.pubkey(),
        ONE_SOL,
        min_out,
        &env.store,
        &env.registry,
        &SwapOptions::default(),
    )
    .expect("build_swap_instructions");

    // Baseline: compile without an ALT.
    let tx_plain =
        compile_to_transaction(&bundle, &env.user.pubkey(), &[], env.svm.latest_blockhash())
            .expect("compile without ALT");
    let plain_size = bincode::serialize(&tx_plain).unwrap().len();

    // Curated static ALT, hand-placed as an on-chain account.
    let addresses = thunder_engine::alt::static_alt_addresses();
    let alt_key = Pubkey::new_unique();
    let data = alt_account_data(&env.user.pubkey(), &addresses);
    // The engine-side parser must agree with the on-chain layout.
    assert_eq!(
        thunder_engine::alt::parse_alt_account(&data).expect("parse_alt_account"),
        addresses,
    );
    env.svm
        .set_account(
            alt_key,
            Account {
                lamports: 10_000_000,
                data,
                owner: thunder_engine::alt::ALT_PROGRAM_ID,
                executable: false,
                rent_epoch: u64::MAX,
            },
        )
        .expect("set ALT account");
    let alt = AddressLookupTableAccount { key: alt_key, addresses };

    let tx_alt = compile_to_transaction(
        &bundle,
        &env.user.pubkey(),
        std::slice::from_ref(&alt),
        env.svm.latest_blockhash(),
    )
    .expect("compile with ALT");
    let alt_size = bincode::serialize(&tx_alt).unwrap().len();

    println!("2-hop bundle: {plain_size} bytes without ALT, {alt_size} bytes with ALT");
    assert!(
        alt_size < plain_size,
        "ALT compile should shrink the tx ({alt_size} !< {plain_size})"
    );

    // Execute the ALT transaction (round trip ends in WSOL -> lamports).
    let lamports_before = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);
    let meta = env.sign_and_send(tx_alt).expect("2-hop ALT swap failed");
    let lamports_after = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);
    println!(
        "2-hop ALT: min_out={min_out} lamports_delta={} cu={}",
        lamports_after as i128 - lamports_before as i128,
        meta.compute_units_consumed,
    );
    // Margin: wrapped input + base fee + priority fee (limit x price / 1e6).
    let priority_fee = bundle.compute_unit_limit as u64
        * bundle.compute_unit_price_micro_lamports
        / 1_000_000;
    assert!(
        lamports_after + ONE_SOL + TX_FEE + priority_fee >= lamports_before + min_out,
        "output below min_out: before={lamports_before} after={lamports_after} min_out={min_out}"
    );
}

/// Token -> SOL: the output-side wSOL unwrap must close the WSOL ATA so the
/// proceeds land in the user's lamport balance (input is NOT SOL here, so
/// this exercises the M3 output-unwrap specifically).
#[test]
fn token_to_sol_output_unwrap() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let Some(pool) = env.pool_by_dex("Meteora DAMM V2") else {
        eprintln!("skipping: no Meteora DAMM V2 pool in fixtures/manifest.json");
        return;
    };

    let wsol = Pubkey::from_str_const(WSOL);
    let (token_mint, wsol_side_ok) = {
        let info = env.registry.get_pool(&pool).expect("pool in registry");
        if info.quote_mint == wsol {
            (info.base_mint, true)
        } else if info.base_mint == wsol {
            (info.quote_mint, true)
        } else {
            (info.quote_mint, false)
        }
    };
    if !wsol_side_ok {
        eprintln!("skipping: DAMM V2 fixture pool has no WSOL side");
        return;
    }

    // Fund the user's (pre-created) token ATA directly.
    let amount_in: u64 = 1_000_000_000;
    let token_ata = env.user_ata(&token_mint);
    let mut ata_acc = env.svm.get_account(&token_ata).expect("token ATA pre-created");
    ata_acc.data[64..72].copy_from_slice(&amount_in.to_le_bytes());
    env.svm.set_account(token_ata, ata_acc).expect("fund token ATA");

    let route = env.route(&[&pool], token_mint, amount_in);
    assert_eq!(route.output_mint, wsol, "expected a token -> WSOL route");
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);

    let bundle = build_swap_instructions(
        &route,
        &env.user.pubkey(),
        amount_in,
        min_out,
        &env.store,
        &env.registry,
        &SwapOptions::default(),
    )
    .expect("build_swap_instructions");
    // Input is not SOL: no wrap in setup, exactly one close in cleanup.
    assert_eq!(bundle.cleanup.len(), 1, "expected exactly one wSOL close");
    let tx = compile_to_transaction(&bundle, &env.user.pubkey(), &[], env.svm.latest_blockhash())
        .expect("compile");

    let wsol_ata = env.user_ata(&wsol);
    let lamports_before = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);
    let meta = env.sign_and_send(tx).expect("token -> SOL swap failed");
    let lamports_after = env.svm.get_balance(&env.user.pubkey()).unwrap_or(0);

    // The wSOL ATA must be closed (rent round-trips back to the user).
    let wsol_ata_after = env.svm.get_account(&wsol_ata);
    assert!(
        wsol_ata_after.is_none() || wsol_ata_after.unwrap().lamports == 0,
        "wSOL ATA was not closed by the output unwrap"
    );
    // Token balance spent.
    assert_eq!(env.svm_token_balance(&token_ata), 0, "input token not fully spent");

    let priority_fee = bundle.compute_unit_limit as u64
        * bundle.compute_unit_price_micro_lamports
        / 1_000_000;
    println!(
        "token -> SOL: in={amount_in} quoted={} min_out={} lamports_delta={} cu={}",
        route.output_amount,
        min_out,
        lamports_after as i128 - lamports_before as i128,
        meta.compute_units_consumed,
    );
    assert!(
        lamports_after + TX_FEE + priority_fee >= lamports_before + min_out,
        "output below min_out: before={lamports_before} after={lamports_after} min_out={min_out}"
    );
}

// ---------------------------------------------------------------------------
// Hardening: negative tests (router-program/src/checks.rs)
// ---------------------------------------------------------------------------

/// Hand-rolled borsh encoding of `ExecuteRouteArgs` for tests that need to
/// tamper with `num_accounts`.
fn route_args_bytes(amount_in: u64, min_amount_out: u64, legs: &[(u8, u8)]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&amount_in.to_le_bytes());
    v.extend_from_slice(&min_amount_out.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes()); // amounts.len
    v.extend_from_slice(&amount_in.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes()); // paths.len
    v.extend_from_slice(&(legs.len() as u32).to_le_bytes()); // hops.len
    for (dex_type, num_accounts) in legs {
        v.extend_from_slice(&1u32.to_le_bytes()); // legs.len
        v.push(*dex_type);
        v.push(*num_accounts);
        v.push(100); // weight
    }
    v
}

/// Wrap `amount` SOL into the user's WSOL ATA and send the given router
/// instruction. Mirrors the pre-instructions of `build_swap_transaction`.
fn send_router_ix(
    env: &mut FixtureEnv,
    router_ix: Instruction,
    wrap_amount: u64,
) -> Result<TransactionMetadata, String> {
    let user = env.user.pubkey();
    let wsol = Pubkey::from_str_const(WSOL);
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
    let wsol_ata = env.user_ata(&wsol);
    let ixs = vec![
        create_associated_token_account_idempotent(&user, &user, &wsol, &tp),
        system_ix::transfer(&user, &wsol_ata, wrap_amount),
        sync_native(&tp, &wsol_ata).expect("sync_native"),
        router_ix,
    ];
    let message = v0::Message::try_compile(&user, &ixs, &[], env.svm.latest_blockhash())
        .expect("compile message");
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(message),
    };
    env.sign_and_send(tx)
}

/// Baseline router instruction over a single DAMM V2 SOL -> token hop.
fn damm_v2_router_ix(env: &FixtureEnv) -> (Instruction, u64) {
    let pool = env
        .pool_by_dex("Meteora DAMM V2")
        .expect("no Meteora DAMM V2 pool in fixtures");
    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[&pool], wsol, ONE_SOL);
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);
    let ix = build_router_instruction(
        &route, &env.user.pubkey(), ONE_SOL, min_out, &env.store, &env.registry,
    )
    .expect("build_router_instruction");
    (ix, min_out)
}

fn assert_custom_error(result: Result<TransactionMetadata, String>, code: u32, what: &str) {
    let err = result.err().unwrap_or_else(|| panic!("{what}: transaction unexpectedly succeeded"));
    assert!(
        err.contains(&format!("Custom({code})")),
        "{what}: expected Custom({code}), got {err}"
    );
}

/// accounts[0] of a hop must match the hardcoded program id for its DexType.
#[test]
fn rejects_wrong_dex_program() {
    let Some(mut env) = FixtureEnv::load() else { return };
    if env.pool_by_dex("Meteora DAMM V2").is_none() {
        eprintln!("skipping: no Meteora DAMM V2 pool in fixtures/manifest.json");
        return;
    }
    let (mut ix, _) = damm_v2_router_ix(&env);
    // Swap in the DLMM program id for a DAMM V2 hop.
    let dlmm = Pubkey::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
    ix.accounts[0] = AccountMeta::new_readonly(dlmm, false);
    assert_custom_error(send_router_ix(&mut env, ix, ONE_SOL), 2, "wrong dex program");
}

/// Hop N's source token account must hold hop N-1's output mint.
#[test]
fn rejects_mismatched_hop_chaining() {
    let Some(mut env) = FixtureEnv::load() else { return };
    let (Some(damm_v2), Some(dlmm)) = (
        env.pool_by_dex("Meteora DAMM V2"),
        env.pool_by_dex("Meteora DLMM"),
    ) else {
        eprintln!("skipping: need both a DAMM V2 and a DLMM pool in fixtures");
        return;
    };

    let wsol = Pubkey::from_str_const(WSOL);
    let route = env.route(&[&damm_v2, &dlmm], wsol, ONE_SOL);
    let min_out = calculate_min_amount_out(route.output_amount, SLIPPAGE_BPS);
    let mut ix = build_router_instruction(
        &route, &env.user.pubkey(), ONE_SOL, min_out, &env.store, &env.registry,
    )
    .expect("build_router_instruction");

    // Hop 1 (DLMM) starts after the 13 DAMM V2 accounts; its source token
    // account sits at [offset + 2]. Point it at the user's WSOL ATA, which
    // holds the wrong mint (hop 0 outputs the intermediate token).
    let wsol_ata = env.user_ata(&wsol);
    ix.accounts[13 + 2] = AccountMeta::new(wsol_ata, false);
    assert_custom_error(send_router_ix(&mut env, ix, ONE_SOL), 3, "mismatched hop chaining");
}

/// The swap authority must be a transaction signer.
#[test]
fn rejects_non_signer_authority() {
    let Some(mut env) = FixtureEnv::load() else { return };
    if env.pool_by_dex("Meteora DAMM V2").is_none() {
        eprintln!("skipping: no Meteora DAMM V2 pool in fixtures/manifest.json");
        return;
    }
    let (mut ix, _) = damm_v2_router_ix(&env);
    // Authority replaced by a pubkey that never signs the transaction.
    ix.accounts[1] = AccountMeta::new(Pubkey::new_unique(), false);
    assert_custom_error(send_router_ix(&mut env, ix, ONE_SOL), 4, "non-signer authority");
}

/// No writable user-owned token account may ride along in a hop's account
/// slice besides the declared source/dest.
#[test]
fn rejects_smuggled_user_token_account() {
    let Some(mut env) = FixtureEnv::load() else { return };
    if env.pool_by_dex("Meteora DAMM V2").is_none() {
        eprintln!("skipping: no Meteora DAMM V2 pool in fixtures/manifest.json");
        return;
    }
    let (mut ix, min_out) = damm_v2_router_ix(&env);

    // Find a fixture mint that is neither the hop's input nor output and
    // smuggle the user's (pre-created) ATA for it into the hop slice.
    let hop_atas: Vec<Pubkey> = ix.accounts[2..4].iter().map(|m| m.pubkey).collect();
    let smuggled_mint = env
        .registry
        .iter_pools()
        .flat_map(|(_, info)| [info.quote_mint, info.base_mint])
        .find(|mint| {
            let ata = env.user_ata(mint);
            !hop_atas.contains(&ata) && env.svm.get_account(&ata).is_some()
        })
        .expect("no spare fixture mint to smuggle");
    let smuggled_ata = env.user_ata(&smuggled_mint);
    ix.accounts.push(AccountMeta::new(smuggled_ata, false));

    // Patch num_accounts 13 -> 14 (DexType::MeteoraDAMMV2 = 1).
    ix.data = route_args_bytes(ONE_SOL, min_out, &[(1, 14)]);
    assert_custom_error(
        send_router_ix(&mut env, ix, ONE_SOL),
        4,
        "smuggled user token account",
    );
}
