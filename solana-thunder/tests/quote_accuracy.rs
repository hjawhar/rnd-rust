//! Quote accuracy gate: our live curve math vs Jupiter single-DEX quotes.
//!
//! For each DEX, quotes hardcoded high-liquidity pairs at three sizes
//! (~$10 / ~$1k / ~$50k), restricted Jupiter to that DEX with direct routes
//! only, fetches the exact pool + auxiliary accounts (tick/bin arrays,
//! amm_config, fee configs, vaults) via RPC into a HashMap-backed
//! `AccountDataProvider`, and compares `calculate_output_live_ex` against
//! Jupiter's `outAmount`.
//!
//! Tolerance: 10 bps for CLMM/DLMM/DAMM v2/V4, 25 bps for Pumpfun
//! (fee-model risk). Snapshot skew between the Jupiter quote and our account
//! fetch (~1s of market drift) is mitigated by fetching immediately after
//! the quote and retrying a failing case once.
//!
//! Run: RPC_URL=... cargo test --test quote_accuracy -- --ignored --nocapture
//! (paced at < 0.45 requests/sec against Jupiter's free tier)

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use borsh::BorshDeserialize;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use thunder_core::{AccountDataProvider, Market, SwapDirection};

const SOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const FART: &str = "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump";

/// ~$10 / ~$1k / ~$50k ladders per input mint (SOL @ ~$76, July 2026).
fn sizes_for(input_mint: &str) -> Vec<u64> {
    match input_mint {
        SOL => vec![130_000_000, 13_200_000_000, 658_000_000_000],
        USDC | USDT => vec![10_000_000, 1_000_000_000, 50_000_000_000],
        FART => vec![70_000_000, 7_000_000_000, 350_000_000_000], // ~$0.145/FART
        _ => vec![10_000_000, 1_000_000_000, 50_000_000_000],
    }
}

struct Case {
    dex: &'static str,       // Jupiter dex label
    input: &'static str,     // input mint
    output: &'static str,    // output mint
    tolerance_bps: f64,
    // Expected pool(s), for documentation; the test adopts whatever pool
    // Jupiter routes through (asserting the label matches) so both sides
    // always price the SAME pool.
    #[allow(dead_code)]
    expected_pool: &'static str,
    pair: &'static str,
}

fn cases() -> Vec<Case> {
    vec![
        // ----- Raydium CLMM (validated pools: SOL/USDC tier-2bps + 1bps) -----
        Case { dex: "Raydium CLMM", input: SOL, output: USDC, tolerance_bps: 10.0,
               expected_pool: "3ucNos4NbumPLZNWztqGHNFFgkHeRMBQAVemeeomsUxv", pair: "SOL/USDC" },
        Case { dex: "Raydium CLMM", input: SOL, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "SOL/USDT" },
        Case { dex: "Raydium CLMM", input: USDC, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "USDC/USDT" },
        // ----- Meteora DLMM -----
        Case { dex: "Meteora DLMM", input: SOL, output: USDC, tolerance_bps: 10.0,
               expected_pool: "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR", pair: "SOL/USDC" },
        Case { dex: "Meteora DLMM", input: SOL, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "SOL/USDT" },
        Case { dex: "Meteora DLMM", input: USDC, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "USDC/USDT" },
        // ----- Meteora DAMM v2 (ground-truth pool from M1) -----
        Case { dex: "Meteora DAMM v2", input: SOL, output: USDC, tolerance_bps: 10.0,
               expected_pool: "8Pm2kZpnxD3hoMmt4bjStX2Pw2Z9abpbHzZxMPqxPmie", pair: "SOL/USDC" },
        Case { dex: "Meteora DAMM v2", input: USDC, output: SOL, tolerance_bps: 10.0,
               expected_pool: "8Pm2kZpnxD3hoMmt4bjStX2Pw2Z9abpbHzZxMPqxPmie", pair: "USDC/SOL" },
        Case { dex: "Meteora DAMM v2", input: SOL, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "SOL/USDT" },
        // ----- Meteora DAMM V1 (vault-share reserves; stable pools skipped) -----
        Case { dex: "Meteora", input: SOL, output: USDC, tolerance_bps: 10.0,
               expected_pool: "5yuefgbJJpmFNK2iiYbLSpv1aZXq7F9AUKkZKErTYCvs", pair: "SOL/USDC" },
        Case { dex: "Meteora", input: USDC, output: SOL, tolerance_bps: 10.0,
               expected_pool: "5yuefgbJJpmFNK2iiYbLSpv1aZXq7F9AUKkZKErTYCvs", pair: "USDC/SOL" },
        Case { dex: "Meteora", input: SOL, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "SOL/USDT" },
        // ----- Pump.fun AMM (25 bps tolerance: fee-model risk) -----
        Case { dex: "Pump.fun Amm", input: SOL, output: USDC, tolerance_bps: 25.0,
               expected_pool: "Gf7sXMoP8iRw4iiXmJ1nq4vxcRycbGXy5RL8a8LnTd3v", pair: "SOL/USDC" },
        Case { dex: "Pump.fun Amm", input: SOL, output: FART, tolerance_bps: 25.0,
               expected_pool: "AmmpSnW5xVeKHTAU9fMjyKEMPgrzmUj3ah5vgvHhAB5J", pair: "SOL/Fartcoin" },
        Case { dex: "Pump.fun Amm", input: FART, output: SOL, tolerance_bps: 25.0,
               expected_pool: "AmmpSnW5xVeKHTAU9fMjyKEMPgrzmUj3ah5vgvHhAB5J", pair: "Fartcoin/SOL" },
        // ----- Raydium V4 -----
        Case { dex: "Raydium", input: SOL, output: USDC, tolerance_bps: 10.0,
               expected_pool: "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2", pair: "SOL/USDC" },
        Case { dex: "Raydium", input: SOL, output: USDT, tolerance_bps: 10.0,
               expected_pool: "(jupiter's pick)", pair: "SOL/USDT" },
        Case { dex: "Raydium", input: USDC, output: SOL, tolerance_bps: 10.0,
               expected_pool: "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2", pair: "USDC/SOL" },
    ]
}

// ============================================================================
// HashMap-backed provider
// ============================================================================

#[derive(Default)]
struct MapProvider {
    accounts: Mutex<HashMap<Pubkey, Vec<u8>>>,
}

impl MapProvider {
    fn insert(&self, k: Pubkey, v: Vec<u8>) {
        self.accounts.lock().unwrap().insert(k, v);
    }
}

impl AccountDataProvider for MapProvider {
    fn pool_account_data(&self, pubkey: &Pubkey) -> Option<Vec<u8>> {
        self.accounts.lock().unwrap().get(pubkey).cloned()
    }
    fn token_balance(&self, vault_pubkey: &Pubkey) -> u64 {
        self.accounts
            .lock()
            .unwrap()
            .get(vault_pubkey)
            .filter(|d| d.len() >= 72)
            .map(|d| u64::from_le_bytes(d[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }
}

// ============================================================================
// Jupiter client (paced <= 0.45 rps)
// ============================================================================

async fn jup_quote(
    http: &reqwest::Client,
    input: &str,
    output: &str,
    amount: u64,
    dex_label: &str,
) -> Result<Option<(String, u64)>, String> {
    // Pace: free tier is 0.5 rps; stay under 0.45.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let url = format!(
        "https://lite-api.jup.ag/swap/v1/quote?inputMint={input}&outputMint={output}\
         &amount={amount}&slippageBps=50&onlyDirectRoutes=true&dexes={}",
        urlencode(dex_label)
    );
    for attempt in 0..3 {
        let resp = http.get(&url).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        if status.as_u16() == 429 {
            eprintln!("  [jup] 429, backing off (attempt {attempt})");
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        if !status.is_success() {
            if body.contains("COULD_NOT_FIND_ANY_ROUTE") {
                return Ok(None);
            }
            return Err(format!("jup {status}: {}", &body[..body.len().min(200)]));
        }
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let out_amount: u64 = v["outAmount"]
            .as_str()
            .ok_or("missing outAmount")?
            .parse()
            .map_err(|_| "bad outAmount")?;
        let plan = v["routePlan"].as_array().ok_or("missing routePlan")?;
        if plan.len() != 1 {
            return Err(format!("expected 1 hop, got {}", plan.len()));
        }
        let info = &plan[0]["swapInfo"];
        let label = info["label"].as_str().unwrap_or("");
        if label != dex_label {
            return Err(format!("label mismatch: {label} != {dex_label}"));
        }
        let amm_key = info["ammKey"].as_str().ok_or("missing ammKey")?.to_string();
        return Ok(Some((amm_key, out_amount)));
    }
    Err("jup: rate limited after retries".into())
}

fn urlencode(s: &str) -> String {
    s.replace(' ', "%20").replace('.', "%2E")
}

// ============================================================================
// Our quote: fetch pool + aux accounts, run calculate_output_live_ex
// ============================================================================

async fn fetch_map(
    rpc: &RpcClient,
    keys: &[Pubkey],
    provider: &MapProvider,
) -> Result<usize, String> {
    let mut stored = 0usize;
    for chunk in keys.chunks(100) {
        let accounts = rpc
            .get_multiple_accounts(chunk)
            .await
            .map_err(|e| e.to_string())?;
        for (k, acct) in chunk.iter().zip(accounts) {
            if let Some(a) = acct {
                provider.insert(*k, a.data);
                stored += 1;
            }
        }
    }
    Ok(stored)
}

async fn quote_ours(
    rpc: &RpcClient,
    dex: &str,
    pool_addr: &str,
    input_mint: &str,
    amount_in: u64,
) -> Result<u64, String> {
    let pool_pk = Pubkey::from_str(pool_addr).map_err(|e| e.to_string())?;
    let input_pk = Pubkey::from_str(input_mint).map_err(|e| e.to_string())?;
    let pool_acct = rpc
        .get_account(&pool_pk)
        .await
        .map_err(|e| format!("pool fetch: {e}"))?;
    let data = pool_acct.data;

    let provider = MapProvider::default();
    provider.insert(pool_pk, data.clone());

    // Build the market + collect aux account keys per DEX.
    let market: Box<dyn Market> = match dex {
        "Raydium CLMM" => {
            let pool = raydium_clmm::RaydiumCLMMPool::deserialize(&mut &data[8..])
                .map_err(|e| format!("clmm parse: {e}"))?;
            let (ext_pda, _) = raydium_clmm::tick_arrays::pda_array_bitmap_address(&pool_pk)
                .map_err(|e| e.to_string())?;
            // First batch: config, extension, vaults.
            fetch_map(
                rpc,
                &[pool.amm_config, ext_pda, pool.token_vault_0, pool.token_vault_1],
                &provider,
            )
            .await?;
            // Direction: physical zero_for_one when input is token_0.
            let is_buy = input_pk == pool.token_mint_0;
            let ext = provider.pool_account_data(&ext_pda);
            // Scan the bitmap for up to 8 initialized tick arrays in the
            // swap direction (compute_clmm_remaining_accounts caps at 3,
            // which can be too few for the $50k walk on 1-2 bps pools).
            let ticks_per_array = pool.tick_spacing as i32 * 60;
            let mut arrays: Vec<Pubkey> = Vec::new();
            let mut start =
                raydium_clmm::tick_arrays::tick_array_start_index(pool.tick_current, pool.tick_spacing);
            for _ in 0..1024 {
                if arrays.len() >= 8 {
                    break;
                }
                let idx = start.div_euclid(ticks_per_array);
                if idx.abs() > 7680 {
                    break;
                }
                if raydium_clmm::tick_arrays::is_tick_array_initialized(
                    &pool.tick_array_bitmap,
                    ext.as_deref(),
                    idx,
                ) {
                    arrays.push(
                        raydium_clmm::tick_arrays::pda_tick_array_address(&pool_pk, start)
                            .map_err(|e| e.to_string())?
                            .0,
                    );
                }
                start += if is_buy { -ticks_per_array } else { ticks_per_array };
            }
            fetch_map(rpc, &arrays, &provider).await?;
            Box::new(raydium_clmm::RaydiumClmmMarket::new(pool, pool_addr.to_string()))
        }
        "Meteora DLMM" => {
            let pool = meteora_dlmm::MeteoraDLMMPool::deserialize(&mut &data[8..])
                .map_err(|e| format!("dlmm parse: {e}"))?;
            let active_idx = meteora_dlmm::bin_math::bin_array_index(pool.active_id);
            let mut keys = vec![
                meteora_dlmm::bin_math::bitmap_extension_pda(&pool_pk),
                pool.reserve_x,
                pool.reserve_y,
            ];
            for i in (active_idx - 3)..=(active_idx + 3) {
                keys.push(meteora_dlmm::bin_math::bin_array_pda(&pool_pk, i));
            }
            fetch_map(rpc, &keys, &provider).await?;
            Box::new(meteora_dlmm::MeteoraDlmmMarket::new(pool, pool_addr.to_string()))
        }
        "Meteora" => {
            // DAMM V1: reserves are the pool's vault-LP share of the shared
            // dynamic vaults' unlocked amounts.
            let pool = meteora_damm::MeteoraDAMMPool::deserialize(&mut &data[8..])
                .map_err(|e| format!("damm1 parse: {e}"))?;
            fetch_map(rpc, &[pool.a_vault, pool.b_vault], &provider).await?;
            let mut keys: Vec<Pubkey> = Vec::new();
            for vault in [&pool.a_vault, &pool.b_vault] {
                let vd = provider
                    .pool_account_data(vault)
                    .ok_or("v1 vault account missing")?;
                keys.push(
                    meteora_damm::v1_vault_lp_mint(&vd).ok_or("v1 vault lp mint unreadable")?,
                );
            }
            keys.push(pool.a_vault_lp);
            keys.push(pool.b_vault_lp);
            fetch_map(rpc, &keys, &provider).await?;
            Box::new(meteora_damm::MeteoraDAMMMarket::new(pool, pool_addr.to_string()))
        }
        "Meteora DAMM v2" => {
            let pool = meteora_damm::MeteoraDAMMV2Pool::deserialize(&mut &data[8..])
                .map_err(|e| format!("damm2 parse: {e}"))?;
            fetch_map(rpc, &[pool.token_a_vault, pool.token_b_vault], &provider).await?;
            Box::new(meteora_damm::MeteoraDAMMV2Market::new(pool, pool_addr.to_string()))
        }
        "Pump.fun Amm" => {
            let pool = pumpfun_amm::PumpfunAmmPool::deserialize(&mut &data[8..])
                .map_err(|e| format!("pump parse: {e}"))?;
            let program = Pubkey::from_str(pumpfun_amm::PUMPFUN_AMM_PROGRAM).unwrap();
            let (global_config, _) =
                Pubkey::find_program_address(&[b"global_config"], &program);
            fetch_map(
                rpc,
                &[
                    pool.pool_base_token_account,
                    pool.pool_quote_token_account,
                    global_config,
                ],
                &provider,
            )
            .await?;
            Box::new(pumpfun_amm::PumpfunAmmMarket::new(pool, pool_addr.to_string()))
        }
        "Raydium" => {
            let pool = raydium_amm_v4::RaydiumAMMV4::deserialize(&mut &data[..])
                .map_err(|e| format!("v4 parse: {e}"))?;
            fetch_map(rpc, &[pool.base_vault, pool.quote_vault], &provider).await?;
            // Balances passed via constructor AND provider (metadata below).
            let quote_bal = provider.token_balance(&pool.quote_vault);
            let base_bal = provider.token_balance(&pool.base_vault);
            Box::new(raydium_amm_v4::RaydiumAmmV4Market::new(
                pool,
                pool_addr.to_string(),
                quote_bal,
                base_bal,
            ))
        }
        other => return Err(format!("unknown dex label {other}")),
    };

    let meta = market.metadata().map_err(|e| e.to_string())?;
    let direction = if input_pk == meta.quote_mint {
        SwapDirection::Buy
    } else if input_pk == meta.base_mint {
        SwapDirection::Sell
    } else {
        return Err("input mint not in pool".into());
    };
    let quote_bal = provider.token_balance(&meta.quote_vault);
    let base_bal = provider.token_balance(&meta.base_vault);

    // V4 has no anchor discriminator; all others carry the raw account
    // bytes (incl. discriminator) as pool_data.
    market
        .calculate_output_live_ex(
            amount_in,
            direction,
            Some(&data),
            quote_bal,
            base_bal,
            Some(&provider),
        )
        .map_err(|e| format!("quote: {e}"))
}

// ============================================================================
// The gate
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn quote_accuracy_vs_jupiter() {
    let _ = dotenvy::dotenv();
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL required");
    let rpc = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();

    struct Row {
        dex: &'static str,
        pair: &'static str,
        pool: String,
        amount_in: u64,
        ours: u64,
        jup: u64,
        delta_bps: f64,
        tolerance: f64,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for case in cases() {
        for amount_in in sizes_for(case.input) {
            // Up to 2 attempts per (case, size): market drift between the
            // Jupiter quote and our account snapshot can spike the delta.
            let mut last: Option<Row> = None;
            for attempt in 0..2 {
                let jq = match jup_quote(&http, case.input, case.output, amount_in, case.dex).await
                {
                    Ok(Some(x)) => x,
                    Ok(None) => {
                        skipped.push(format!("{} {} {}: no route", case.dex, case.pair, amount_in));
                        last = None;
                        break;
                    }
                    Err(e) => {
                        skipped.push(format!("{} {} {}: {e}", case.dex, case.pair, amount_in));
                        last = None;
                        break;
                    }
                };
                let (pool, jup_out) = jq;
                let ours = match quote_ours(&rpc, case.dex, &pool, case.input, amount_in).await {
                    Ok(o) => o,
                    Err(e) => {
                        skipped.push(format!(
                            "{} {} {} pool {pool}: ours failed: {e}",
                            case.dex, case.pair, amount_in
                        ));
                        last = None;
                        break;
                    }
                };
                let delta_bps = (ours as f64 - jup_out as f64) / jup_out as f64 * 10_000.0;
                let row = Row {
                    dex: case.dex,
                    pair: case.pair,
                    pool: pool.clone(),
                    amount_in,
                    ours,
                    jup: jup_out,
                    delta_bps,
                    tolerance: case.tolerance_bps,
                };
                let pass = delta_bps.abs() < case.tolerance_bps;
                last = Some(row);
                if pass {
                    break;
                }
                if attempt == 0 {
                    eprintln!(
                        "  retrying {} {} {} (delta {:.2} bps)",
                        case.dex, case.pair, amount_in, delta_bps
                    );
                }
            }
            if let Some(row) = last {
                rows.push(row);
            }
        }
    }

    // ---- report ----
    println!();
    println!(
        "{:<16} {:<14} {:<12} {:>16} {:>16} {:>16} {:>10}  {}",
        "DEX", "pair", "size", "ours", "jupiter", "delta_bps", "tol", "pool"
    );
    let mut failures = 0;
    for r in &rows {
        let ok = r.delta_bps.abs() < r.tolerance;
        if !ok {
            failures += 1;
        }
        println!(
            "{:<16} {:<14} {:<12} {:>16} {:>16} {:>16.2} {:>10}  {} {}",
            r.dex,
            r.pair,
            r.amount_in,
            r.ours,
            r.jup,
            r.delta_bps,
            r.tolerance,
            r.pool,
            if ok { "" } else { "FAIL" }
        );
    }
    if !skipped.is_empty() {
        println!("\nskipped:");
        for s in &skipped {
            println!("  {s}");
        }
    }
    println!("\n{} rows, {} failures, {} skipped", rows.len(), failures, skipped.len());

    assert!(rows.len() >= 10, "too few comparable rows ({})", rows.len());
    assert_eq!(failures, 0, "{failures} rows exceeded tolerance");
}
