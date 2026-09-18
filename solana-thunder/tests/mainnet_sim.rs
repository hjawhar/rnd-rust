//! Simulate-only smoke test: one route per DEX through `POST /swap`'s
//! pre-send simulation. **Nothing is ever signed or sent** — the endpoint
//! builds an unsigned transaction and calls `simulateTransaction`
//! (sigVerify=false, replaceRecentBlockhash=true) on the engine's RPC.
//!
//! Prerequisites:
//!   - A running thunder-engine on `ENGINE_URL` (default http://127.0.0.1:8080)
//!     started with the pool cache (`CACHE_MAX_AGE=99999999`) and
//!     `ROUTER_PROGRAM_ID` set.
//!   - The engine's RPC must host the router program at that id. Until the
//!     user deploys the router to mainnet, that means running the engine
//!     against a surfpool mainnet fork with the router deployed
//!     (`RPC_URL=http://127.0.0.1:8899`); after the mainnet deploy, the
//!     engine's normal mainnet RPC works directly.
//!
//! Run:
//!   cargo test --release --test mainnet_sim -- --ignored --nocapture
//!
//! Interpretation: pre-A0 the quote math is inaccurate for some DEXs, so a
//! simulation that reaches the router and fails only with `Custom(1)`
//! (slippage / min_out) still proves the transaction plumbing; the test
//! retries those once with `otherAmountThreshold = 1` and reports them as
//! `PASS (plumbing; slippage at quoted min_out)`.

use std::env;
use std::str::FromStr;

use borsh::BorshDeserialize;
use serde_json::{json, Value};
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::{Keypair, Signer};
use thunder_core::WSOL;

const AMOUNT_IN: u64 = 100_000_000; // 0.1 SOL

/// Deserialize an Anchor account, skipping the 8-byte discriminator.
fn deser_anchor<T: BorshDeserialize>(data: &[u8]) -> Option<T> {
    let mut slice = data.get(8..)?;
    T::deserialize(&mut slice).ok()
}

/// The two mints of a pool, parsed with the DEX crates.
fn pool_mints(dex: &str, data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    match dex {
        "Meteora DAMM V2" => {
            let p: meteora_damm::MeteoraDAMMV2Pool = deser_anchor(data)?;
            Some((p.token_a_mint, p.token_b_mint))
        }
        "Meteora DLMM" => {
            let p: meteora_dlmm::MeteoraDLMMPool = deser_anchor(data)?;
            Some((p.token_x_mint, p.token_y_mint))
        }
        "Meteora DAMM V1" => {
            let p: meteora_damm::MeteoraDAMMPool = deser_anchor(data)?;
            Some((p.token_a_mint, p.token_b_mint))
        }
        "Raydium CLMM" => {
            let p: raydium_clmm::RaydiumCLMMPool = deser_anchor(data)?;
            Some((p.token_mint_0, p.token_mint_1))
        }
        "Raydium AMM V4" => {
            let p = raydium_amm_v4::RaydiumAMMV4::try_from_slice(data).ok()?;
            Some((p.base_mint, p.quote_mint))
        }
        "Pumpfun AMM" => {
            let p: pumpfun_amm::PumpfunAmmPool = deser_anchor(data)?;
            Some((p.base_mint, p.quote_mint))
        }
        _ => None,
    }
}

/// Build the Jupiter-form quoteResponse for a single hop.
fn quote_response_json(
    pool: &str,
    dex: &str,
    input_mint: &str,
    output_mint: &str,
    in_amount: u64,
    out_amount: &str,
    min_out: &str,
) -> Value {
    json!({
        "inputMint": input_mint,
        "outputMint": output_mint,
        "inAmount": in_amount.to_string(),
        "outAmount": out_amount,
        "otherAmountThreshold": min_out,
        "swapMode": "ExactIn",
        "priceImpactPct": "0",
        "routePlan": [{
            "swapInfo": {
                "ammKey": pool,
                "label": dex,
                "inputMint": input_mint,
                "outputMint": output_mint,
                "inAmount": in_amount.to_string(),
                "outAmount": out_amount,
                "feeAmount": "0",
                "feeMint": output_mint,
            },
            "percent": 100,
        }],
    })
}

async fn post_swap(
    client: &reqwest::Client,
    engine: &str,
    user: &str,
    quote_response: Value,
) -> Result<(u16, Value), String> {
    let body = json!({
        "userPublicKey": user,
        "quoteResponse": quote_response,
        "wrapAndUnwrapSol": true,
        "skipPreflightSimulation": false,
    });
    let resp = client
        .post(format!("{engine}/swap"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST /swap: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok((status, value))
}

#[tokio::test]
#[ignore = "requires a running engine whose RPC hosts the router program (surfpool fork pre-deploy); run manually"]
async fn simulate_one_route_per_dex() {
    dotenvy::dotenv().ok();
    let engine = env::var("ENGINE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    // Only the PUBKEY is used; nothing in this test signs anything.
    let user = env::var("MAINNET_SIM_USER").unwrap_or_else(|_| {
        Keypair::from_base58_string(
            &env::var("PRIVATE_KEY").expect("set PRIVATE_KEY (pubkey source) or MAINNET_SIM_USER"),
        )
        .pubkey()
        .to_string()
    });
    // Read-only RPC for pool-account mints (mainnet or the fork; reads only).
    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let rpc = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let client = reqwest::Client::new();

    // One pool per DEX from the fixture manifest.
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string("fixtures/manifest.json").expect("fixtures/manifest.json"),
    )
    .expect("parse manifest");
    let pools: Vec<(String, String)> = manifest["pools"]
        .as_array()
        .expect("manifest pools")
        .iter()
        .map(|p| {
            (
                p["dex_name"].as_str().unwrap().to_string(),
                p["address"].as_str().unwrap().to_string(),
            )
        })
        .collect();

    println!("Engine:  {engine}");
    println!("User:    {user} (pubkey only; simulate-only)\n");

    let wsol = Pubkey::from_str_const(WSOL);
    let mut results: Vec<(String, String)> = Vec::new();

    for (dex, pool) in &pools {
        println!("=== {dex} ({pool}) ===");

        // Resolve the pool's mints and pick SOL as the input side.
        let pool_pk = Pubkey::from_str(pool).expect("pool pubkey");
        let account = match rpc.get_account(&pool_pk).await {
            Ok(a) => a,
            Err(e) => {
                results.push((dex.clone(), format!("FAIL: fetch pool account: {e}")));
                continue;
            }
        };
        let Some((m0, m1)) = pool_mints(dex, &account.data) else {
            results.push((dex.clone(), "FAIL: could not parse pool mints".into()));
            continue;
        };
        let (input_mint, output_mint) = if m0 == wsol {
            (m0, m1)
        } else if m1 == wsol {
            (m1, m0)
        } else {
            results.push((dex.clone(), "SKIP: pool has no WSOL side".into()));
            continue;
        };

        // Try to get a realistic quote for THIS pool from the engine.
        let quote_url = format!(
            "{engine}/quote?inputMint={input_mint}&outputMint={output_mint}&amount={AMOUNT_IN}&maxHops=1&slippageBps=500"
        );
        let (out_amount, min_out) = match client.get(&quote_url).send().await {
            Ok(resp) => {
                let v: Value = resp.json().await.unwrap_or(Value::Null);
                let matching = v["routes"].as_array().and_then(|routes| {
                    routes.iter().find(|r| {
                        r["hops"].as_array().is_some_and(|hops| {
                            hops.len() == 1 && hops[0]["poolAddress"] == *pool.as_str()
                        })
                    })
                });
                match matching {
                    Some(route) => {
                        let out: u64 = route["outputAmount"]
                            .as_str()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        (out.to_string(), (out * 95 / 100).max(1).to_string())
                    }
                    None => {
                        println!("  note: pool not in engine's top routes; using min_out=1 (plumbing-only)");
                        ("1".to_string(), "1".to_string())
                    }
                }
            }
            Err(e) => {
                results.push((dex.clone(), format!("FAIL: engine /quote unreachable: {e}")));
                continue;
            }
        };

        // POST /swap (quoteResponse form => single attempt, simulated).
        let qr = quote_response_json(
            pool,
            dex,
            &input_mint.to_string(),
            &output_mint.to_string(),
            AMOUNT_IN,
            &out_amount,
            &min_out,
        );
        let outcome = match post_swap(&client, &engine, &user, qr).await {
            Ok((200, body)) => format!(
                "PASS: simulated OK (slot={}, unitsConsumed={}, cuLimit={})",
                body["simulationSlot"], body["unitsConsumed"], body["computeUnitLimit"],
            ),
            Ok((422, body)) if body.to_string().contains("Custom(1)") => {
                // Quote-level slippage (expected pre-A0): prove plumbing with
                // min_out = 1.
                let qr_retry = quote_response_json(
                    pool,
                    dex,
                    &input_mint.to_string(),
                    &output_mint.to_string(),
                    AMOUNT_IN,
                    &out_amount,
                    "1",
                );
                match post_swap(&client, &engine, &user, qr_retry).await {
                    Ok((200, body)) => format!(
                        "PASS (plumbing; slippage at quoted min_out): slot={}, unitsConsumed={}",
                        body["simulationSlot"], body["unitsConsumed"],
                    ),
                    Ok((status, body)) => format!("FAIL after slippage retry: {status}: {body}"),
                    Err(e) => format!("FAIL after slippage retry: {e}"),
                }
            }
            Ok((status, body)) => format!("FAIL: {status}: {body}"),
            Err(e) => format!("FAIL: {e}"),
        };
        println!("  {outcome}\n");
        results.push((dex.clone(), outcome));
    }

    println!("=================== per-DEX results ===================");
    let mut failures = 0;
    for (dex, outcome) in &results {
        println!("{dex:<18} {outcome}");
        if outcome.starts_with("FAIL") {
            failures += 1;
        }
    }
    assert_eq!(failures, 0, "{failures} DEX simulation(s) failed");
}
