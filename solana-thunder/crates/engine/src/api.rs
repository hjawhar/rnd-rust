use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use solana_sdk::message::AddressLookupTableAccount;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use thunder_aggregator::pool_index::PoolIndex;
use thunder_aggregator::price;
use thunder_aggregator::router::Router;
use thunder_aggregator::splitter::SplitConfig;
use thunder_aggregator::types::{Route, RouteHop, SplitQuote};
use thunder_core::{calculate_min_amount_out, WSOL};

use crate::account_store::AccountStore;
use crate::pool_registry::PoolRegistry;
use crate::swap::{
    build_swap_instructions, compile_to_transaction, route_compute_unit_limit, SwapIxBundle,
    SwapOptions, DEFAULT_COMPUTE_UNIT_PRICE,
};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub store: Arc<AccountStore>,
    pub pool_index: Arc<PoolIndex>,
    pub registry: Arc<RwLock<PoolRegistry>>,
    pub rpc: Arc<solana_rpc_client::nonblocking::rpc_client::RpcClient>,
    pub sol_usd_price: RwLock<Option<f64>>,
    pub recent_blockhash: RwLock<Option<(solana_sdk::hash::Hash, u64)>>,
    /// Cached swap ALT (from `SWAP_ALT_ADDRESS`, refreshed every 10 min by
    /// bin/engine.rs). `None` -> transactions compile without an ALT.
    pub alt: RwLock<Option<AddressLookupTableAccount>>,
    pub start_time: Instant,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn create_router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/quote", get(handle_quote))
        .route("/swap", post(handle_swap))
        .route("/swap-instructions", post(handle_swap_instructions))
        .route("/price", get(handle_price))
        .route("/health", get(handle_health))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /quote
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteParams {
    input_mint: String,
    output_mint: String,
    amount: u64,
    slippage_bps: Option<u64>,
    max_hops: Option<usize>,
    /// Split routing: absent/""/"0"/"1" -> single path (unchanged response),
    /// "2".."4" -> split across up to N pool-disjoint paths, "auto" -> split
    /// when the amount is large relative to the best direct pool's depth.
    splits: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuoteResponse {
    input_mint: String,
    output_mint: String,
    amount: String,
    slippage_bps: u64,
    routes: Vec<RouteJson>,
    time_taken_ms: u128,
    // --- Jupiter-mirror superset (best route; absent when no route found) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    in_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    out_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    other_amount_threshold: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price_impact_pct: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    route_plan: Option<Vec<RoutePlanStep>>,
    /// Output of the best unsplit route (only present when ?splits= was
    /// requested and a route exists; equals outAmount when no split fired).
    #[serde(skip_serializing_if = "Option::is_none")]
    single_path_out_amount: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteJson {
    hops: Vec<HopJson>,
    output_amount: String,
    price_impact_bps: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HopJson {
    pool_address: String,
    dex_name: String,
    input_mint: String,
    output_mint: String,
    input_amount: String,
    output_amount: String,
}

/// One Jupiter-style routePlan step.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoutePlanStep {
    swap_info: SwapInfo,
    percent: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwapInfo {
    /// Pool address.
    amm_key: String,
    /// DEX name (our internal labels, e.g. "Meteora DAMM V2").
    label: String,
    input_mint: String,
    output_mint: String,
    in_amount: String,
    out_amount: String,
    #[serde(default)]
    fee_amount: String,
    #[serde(default)]
    fee_mint: String,
}

fn route_to_json(route: &Route) -> RouteJson {
    RouteJson {
        output_amount: route.output_amount.to_string(),
        price_impact_bps: route.price_impact_bps,
        hops: route
            .hops
            .iter()
            .map(|hop| HopJson {
                pool_address: hop.pool_address.clone(),
                dex_name: hop.dex_name.clone(),
                input_mint: hop.input_mint.to_string(),
                output_mint: hop.output_mint.to_string(),
                input_amount: hop.input_amount.to_string(),
                output_amount: hop.output_amount.to_string(),
            })
            .collect(),
    }
}

fn route_to_route_plan(route: &Route) -> Vec<RoutePlanStep> {
    route
        .hops
        .iter()
        .map(|hop| RoutePlanStep {
            swap_info: SwapInfo {
                amm_key: hop.pool_address.clone(),
                label: hop.dex_name.clone(),
                input_mint: hop.input_mint.to_string(),
                output_mint: hop.output_mint.to_string(),
                in_amount: hop.input_amount.to_string(),
                out_amount: hop.output_amount.to_string(),
                fee_amount: "0".to_string(),
                fee_mint: hop.output_mint.to_string(),
            },
            percent: 100,
        })
        .collect()
}

/// Rebuild a `Route` from a Jupiter-style routePlan (ammKey -> pool address,
/// label -> DEX name). Amounts come from the plan verbatim; no re-quoting.
fn route_from_route_plan(plan: &[RoutePlanStep]) -> Result<Route, String> {
    if plan.is_empty() {
        return Err("quoteResponse.routePlan is empty".into());
    }
    let mut hops = Vec::with_capacity(plan.len());
    for step in plan {
        let si = &step.swap_info;
        hops.push(RouteHop {
            pool_address: si.amm_key.clone(),
            dex_name: si.label.clone(),
            input_mint: parse_mint(&si.input_mint)?,
            output_mint: parse_mint(&si.output_mint)?,
            input_amount: si
                .in_amount
                .parse()
                .map_err(|e| format!("routePlan inAmount: {e}"))?,
            output_amount: si
                .out_amount
                .parse()
                .map_err(|e| format!("routePlan outAmount: {e}"))?,
            price_impact_bps: 0,
        });
    }
    Ok(Route {
        input_mint: hops.first().unwrap().input_mint,
        output_mint: hops.last().unwrap().output_mint,
        input_amount: hops.first().unwrap().input_amount,
        output_amount: hops.last().unwrap().output_amount,
        hops,
        price_impact_bps: 0,
    })
}

/// Parse a mint string, accepting "SOL" as shorthand for WSOL.
fn parse_mint(s: &str) -> Result<Pubkey, String> {
    if s.eq_ignore_ascii_case("SOL") {
        Ok(Pubkey::from_str_const(WSOL))
    } else {
        Pubkey::from_str(s).map_err(|e| format!("invalid mint: {e}"))
    }
}

async fn handle_quote(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QuoteParams>,
) -> Result<Json<QuoteResponse>, (StatusCode, String)> {
    let input_mint = parse_mint(&params.input_mint).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let output_mint = parse_mint(&params.output_mint).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let slippage_bps = params.slippage_bps.unwrap_or(50);
    let max_hops = params.max_hops.unwrap_or(2).min(4);

    let start = Instant::now();

    let swappable = state.registry.read().await.swappable_set();
    let router = Router::new(&state.pool_index, max_hops)
        .with_swappable_set(swappable.clone())
        .with_live_data(state.store.as_ref());

    // --- split routing (opt-in via ?splits=; quote-side only) --------------
    if let Some(max_paths) = split_mode(&state, &params, input_mint, output_mint, max_hops) {
        let cfg = SplitConfig {
            max_paths,
            ..Default::default()
        };
        let split_quote = router
            .find_split_routes(input_mint, output_mint, params.amount, &cfg)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("routing error: {e}")))?;
        return Ok(Json(split_quote_response(
            &params,
            input_mint,
            output_mint,
            slippage_bps,
            &split_quote,
            start,
        )));
    }

    let mut quote = router
        .find_routes(input_mint, output_mint, params.amount, 5)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("routing error: {e}")))?;

    // Adaptive hop escalation: if no route exists within the requested hop
    // budget, retry once at 3 hops. The extra cost is paid only on cases that
    // would otherwise return no route, so fast paths stay fast.
    if quote.routes.is_empty() && max_hops < 3 {
        let deep = Router::new(&state.pool_index, 3)
            .with_swappable_set(swappable.clone())
            .with_live_data(state.store.as_ref());
        quote = deep
            .find_routes(input_mint, output_mint, params.amount, 5)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("routing error: {e}")))?;
    }

    let best = quote.routes.first();
    let (in_amount, out_amount, other_amount_threshold, swap_mode, price_impact_pct, route_plan) =
        match best {
            Some(route) => (
                Some(route.input_amount.to_string()),
                Some(route.output_amount.to_string()),
                Some(calculate_min_amount_out(route.output_amount, slippage_bps).to_string()),
                Some("ExactIn"),
                Some((route.price_impact_bps as f64 / 10_000.0).to_string()),
                Some(route_to_route_plan(route)),
            ),
            None => (None, None, None, None, None, None),
        };

    let routes: Vec<RouteJson> = quote.routes.iter().map(route_to_json).collect();
    let elapsed = start.elapsed();

    Ok(Json(QuoteResponse {
        input_mint: input_mint.to_string(),
        output_mint: output_mint.to_string(),
        amount: params.amount.to_string(),
        slippage_bps,
        routes,
        time_taken_ms: elapsed.as_millis(),
        in_amount,
        out_amount,
        other_amount_threshold,
        swap_mode,
        price_impact_pct,
        route_plan,
        single_path_out_amount: None,
    }))
}

/// Resolve the `splits` query param to a `max_paths`, or `None` for the
/// unchanged single-path code path.
///
/// "auto" splits when the amount exceeds 0.5% of the input-side vault
/// balance of the best direct pool (one live `token_balance` read via the
/// store, falling back to the cached balance while the store is cold). With
/// no direct pool the swap is multi-hop anyway, so auto behaves like
/// splits=3 when maxHops permits multi-hop routes.
fn split_mode(
    state: &AppState,
    params: &QuoteParams,
    input_mint: Pubkey,
    output_mint: Pubkey,
    max_hops: usize,
) -> Option<usize> {
    match params.splits.as_deref() {
        None | Some("" | "0" | "1") => None,
        Some("auto") => {
            let direct = state.pool_index.direct_pools(&input_mint, &output_mint);
            let mut best: Option<(&thunder_aggregator::types::PoolEntry, u64)> = None;
            for addr in &direct {
                let Some(entry) = state.pool_index.get_pool(addr) else { continue };
                let Ok(fin) = entry.market.financials() else { continue };
                let cached_in = if input_mint == entry.quote_mint {
                    fin.quote_balance
                } else {
                    fin.base_balance
                };
                if best.as_ref().is_none_or(|(_, b)| cached_in > *b) {
                    best = Some((entry, cached_in));
                }
            }
            match best {
                Some((entry, cached_in)) => {
                    let vault = if input_mint == entry.quote_mint {
                        entry.quote_vault
                    } else {
                        entry.base_vault
                    };
                    let live = state.store.read_token_balance(&vault);
                    let vault_balance = if live > 0 { live } else { cached_in };
                    // amount > 0.5% of the vault  <=>  amount * 200 > vault
                    (params.amount as u128 * 200 > vault_balance as u128).then_some(3)
                }
                None => (max_hops >= 2).then_some(3),
            }
        }
        Some(n) => n
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 2)
            .map(|n| n.min(4)),
    }
}

/// Build the /quote response for a split quote. Shapes:
/// - no route: identical to the unsplit no-route response;
/// - one path (split did not beat single): the single-path response shape
///   with `singlePathOutAmount == outAmount`;
/// - multiple paths: `outAmount` = total output, one `routePlan` entry per
///   hop per path with `percent` = the path's share, `routes[]` = one entry
///   per path ranked by allocated amount.
fn split_quote_response(
    params: &QuoteParams,
    input_mint: Pubkey,
    output_mint: Pubkey,
    slippage_bps: u64,
    sq: &SplitQuote,
    start: Instant,
) -> QuoteResponse {
    let (in_amount, out_amount, other_amount_threshold, swap_mode, price_impact_pct, route_plan, single_path_out_amount) =
        if sq.paths.is_empty() {
            (None, None, None, None, None, None, None)
        } else {
            // Amount-weighted aggregate price impact across paths.
            let weighted_impact_bps: u64 = (sq
                .paths
                .iter()
                .map(|p| p.amount_in as u128 * p.route.price_impact_bps as u128)
                .sum::<u128>()
                / sq.input_amount.max(1) as u128) as u64;
            let plan: Vec<RoutePlanStep> = sq
                .paths
                .iter()
                .flat_map(|p| {
                    let percent = ((p.percent_bps + 50) / 100).clamp(1, 100) as u8;
                    p.route.hops.iter().map(move |hop| RoutePlanStep {
                        swap_info: SwapInfo {
                            amm_key: hop.pool_address.clone(),
                            label: hop.dex_name.clone(),
                            input_mint: hop.input_mint.to_string(),
                            output_mint: hop.output_mint.to_string(),
                            in_amount: hop.input_amount.to_string(),
                            out_amount: hop.output_amount.to_string(),
                            fee_amount: "0".to_string(),
                            fee_mint: hop.output_mint.to_string(),
                        },
                        percent,
                    })
                })
                .collect();
            (
                Some(sq.input_amount.to_string()),
                Some(sq.total_output.to_string()),
                Some(calculate_min_amount_out(sq.total_output, slippage_bps).to_string()),
                Some("ExactIn"),
                Some((weighted_impact_bps as f64 / 10_000.0).to_string()),
                Some(plan),
                Some(sq.single_path_output.to_string()),
            )
        };

    QuoteResponse {
        input_mint: input_mint.to_string(),
        output_mint: output_mint.to_string(),
        amount: params.amount.to_string(),
        slippage_bps,
        routes: sq.paths.iter().map(|p| route_to_json(&p.route)).collect(),
        time_taken_ms: start.elapsed().as_millis(),
        in_amount,
        out_amount,
        other_amount_threshold,
        swap_mode,
        price_impact_pct,
        route_plan,
        single_path_out_amount,
    }
}

// ---------------------------------------------------------------------------
// POST /swap + POST /swap-instructions (shared core)
// ---------------------------------------------------------------------------

/// Accepts both the legacy convenience form (inputMint/outputMint/amount)
/// and the Jupiter-mirror form (quoteResponse).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwapRequest {
    user_public_key: String,
    // --- convenience form ---
    input_mint: Option<String>,
    output_mint: Option<String>,
    amount: Option<u64>,
    slippage_bps: Option<u64>,
    max_hops: Option<usize>,
    // --- Jupiter-mirror form ---
    quote_response: Option<QuoteResponseIn>,
    // --- shared knobs ---
    wrap_and_unwrap_sol: Option<bool>,
    /// Total priority fee in lamports (number) or "auto".
    prioritization_fee_lamports: Option<serde_json::Value>,
    /// Direct compute-unit price in µ-lamports (wins over the above).
    compute_unit_price_micro_lamports: Option<u64>,
    /// Re-tighten the CU limit to simulated consumption x 1.1.
    dynamic_compute_unit_limit: Option<bool>,
    /// Skip the pre-send simulation entirely.
    skip_preflight_simulation: Option<bool>,
}

/// The subset of our /quote response POST /swap consumes. Unknown fields
/// (swapMode, priceImpactPct, ...) are ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteResponseIn {
    other_amount_threshold: String,
    route_plan: Vec<RoutePlanStep>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SwapResponse {
    /// Base64-encoded unsigned serialized VersionedTransaction.
    transaction: String,
    /// Jupiter-compatible alias of `transaction`.
    swap_transaction: String,
    /// The route used.
    route: RouteJson,
    /// Blockhash used in the transaction.
    blockhash: String,
    /// Last block height at which the transaction's blockhash is valid.
    last_valid_block_height: u64,
    /// Compute-unit limit set in the transaction.
    compute_unit_limit: u32,
    /// Total priority fee paid at that limit (lamports).
    prioritization_fee_lamports: u64,
    /// Slot the pre-send simulation ran at (absent when skipped).
    #[serde(skip_serializing_if = "Option::is_none")]
    simulation_slot: Option<u64>,
    /// Compute units consumed in the pre-send simulation.
    #[serde(skip_serializing_if = "Option::is_none")]
    units_consumed: Option<u64>,
    time_taken_ms: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IxJson {
    program_id: String,
    accounts: Vec<IxAccountJson>,
    /// Base64-encoded instruction data.
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IxAccountJson {
    pubkey: String,
    is_signer: bool,
    is_writable: bool,
}

fn ix_to_json(ix: &solana_sdk::instruction::Instruction) -> IxJson {
    IxJson {
        program_id: ix.program_id.to_string(),
        accounts: ix
            .accounts
            .iter()
            .map(|m| IxAccountJson {
                pubkey: m.pubkey.to_string(),
                is_signer: m.is_signer,
                is_writable: m.is_writable,
            })
            .collect(),
        data: base64::engine::general_purpose::STANDARD.encode(&ix.data),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SwapInstructionsResponse {
    compute_budget_instructions: Vec<IxJson>,
    setup_instructions: Vec<IxJson>,
    swap_instruction: IxJson,
    cleanup_instructions: Vec<IxJson>,
    address_lookup_table_addresses: Vec<String>,
    route: RouteJson,
    blockhash: String,
    last_valid_block_height: u64,
    compute_unit_limit: u32,
    prioritization_fee_lamports: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    simulation_slot: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    units_consumed: Option<u64>,
    time_taken_ms: u128,
}

/// How the caller specified the priority fee.
enum FeeSpec {
    /// Default / "auto": a constant µ-lamport price.
    Auto,
    /// Total lamports to spend on priority fee; price derives from CU limit.
    TotalLamports(u64),
    /// Direct µ-lamport price.
    PriceMicroLamports(u64),
}

impl FeeSpec {
    fn from_request(req: &SwapRequest) -> Self {
        if let Some(price) = req.compute_unit_price_micro_lamports {
            return FeeSpec::PriceMicroLamports(price);
        }
        match &req.prioritization_fee_lamports {
            Some(serde_json::Value::Number(n)) => {
                FeeSpec::TotalLamports(n.as_u64().unwrap_or(0))
            }
            // "auto", objects, null -> auto.
            _ => FeeSpec::Auto,
        }
    }

    /// µ-lamports per CU at the given limit.
    fn price(&self, cu_limit: u32) -> u64 {
        match self {
            FeeSpec::Auto => DEFAULT_COMPUTE_UNIT_PRICE,
            FeeSpec::PriceMicroLamports(p) => *p,
            FeeSpec::TotalLamports(total) => {
                if cu_limit == 0 {
                    0
                } else {
                    (*total as u128 * 1_000_000 / cu_limit as u128) as u64
                }
            }
        }
    }
}

/// Total priority fee in lamports at a given limit/price.
fn total_priority_fee(cu_limit: u32, cu_price: u64) -> u64 {
    (cu_limit as u128 * cu_price as u128 / 1_000_000) as u64
}

/// Everything both POST endpoints need, computed once.
struct PreparedSwap {
    route: Route,
    bundle: SwapIxBundle,
    tx: solana_sdk::transaction::VersionedTransaction,
    blockhash: solana_sdk::hash::Hash,
    last_valid_block_height: u64,
    alt_addresses: Vec<String>,
    simulation_slot: Option<u64>,
    units_consumed: Option<u64>,
}

/// Ensure every account the tx builder reads for this route is in the store
/// (pool accounts, mints, and DEX-specific auxiliaries), one-shot fetching
/// anything missing via RPC.
async fn ensure_route_accounts(
    state: &AppState,
    route: &Route,
) -> Result<(), (StatusCode, String)> {
    let mut wanted: Vec<Pubkey> = Vec::new();
    for hop in &route.hops {
        let pool_pk = Pubkey::from_str(&hop.pool_address)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad pool addr: {e}")))?;
        if !state.store.contains(&pool_pk) {
            if let Ok(acc) = state.rpc.get_account(&pool_pk).await {
                state.store.upsert(pool_pk, acc.data, acc.owner, acc.lamports, 0);
            }
        }
        // Mints (token-program detection reads their owner).
        wanted.push(hop.input_mint);
        wanted.push(hop.output_mint);
        // DEX-specific auxiliary accounts the tx builder reads from the store.
        match hop.dex_name.as_str() {
            // Swap-ordered tick array selection needs the bitmap extension
            // (optional on-chain; skipped if it does not exist).
            "Raydium CLMM" => wanted.push(crate::swap::clmm_bitmap_extension_pda(&pool_pk)),
            // Fee recipients are read from the AMM's GlobalConfig.
            "Pumpfun AMM" => wanted.push(crate::swap::pumpfun_global_config_pda()),
            // token_vault/lp_mint are read from the two vault state accounts.
            "Meteora DAMM V1" => {
                if let Some(pool_data) = state.store.get_data(&pool_pk) {
                    if pool_data.len() >= 168 {
                        wanted.push(Pubkey::new_from_array(pool_data[104..136].try_into().unwrap()));
                        wanted.push(Pubkey::new_from_array(pool_data[136..168].try_into().unwrap()));
                    }
                }
            }
            _ => {}
        }
    }
    for pk in wanted {
        if !state.store.contains(&pk) {
            if let Ok(acc) = state.rpc.get_account(&pk).await {
                state.store.upsert(pk, acc.data, acc.owner, acc.lamports, 0);
            }
        }
    }
    Ok(())
}

/// Build (and by default pre-simulate) a swap transaction for the request.
///
/// Convenience form: quotes up to 5 routes and falls through to the next
/// ranked route when simulation fails (max 3 attempts). Jupiter-mirror form
/// (quoteResponse): single attempt on the given route, `otherAmountThreshold`
/// used verbatim as min_out.
async fn prepare_swap(
    state: &AppState,
    req: &SwapRequest,
) -> Result<PreparedSwap, (StatusCode, String)> {
    let user = Pubkey::from_str(&req.user_public_key)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid userPublicKey: {e}")))?;
    let skip_sim = req.skip_preflight_simulation.unwrap_or(false);
    let dynamic_cu = req.dynamic_compute_unit_limit.unwrap_or(false);
    let fee_spec = FeeSpec::from_request(req);
    let wrap_and_unwrap_sol = req.wrap_and_unwrap_sol.unwrap_or(true);

    // Cached blockhash.
    let (blockhash, last_valid_block_height) = state
        .recent_blockhash
        .read()
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "blockhash not yet available".to_string()))?;

    // Cached ALT (None -> compile without lookup tables).
    let alt = state.alt.read().await.clone();
    let alts: Vec<AddressLookupTableAccount> = alt.into_iter().collect();
    let alt_addresses: Vec<String> = alts.iter().map(|a| a.key.to_string()).collect();

    // Candidate routes + per-route min_out.
    let candidates: Vec<(Route, u64)> = if let Some(quote_response) = &req.quote_response {
        let route = route_from_route_plan(&quote_response.route_plan)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let min_out: u64 = quote_response
            .other_amount_threshold
            .parse()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("otherAmountThreshold: {e}")))?;
        vec![(route, min_out)]
    } else {
        let (Some(input), Some(output), Some(amount)) =
            (&req.input_mint, &req.output_mint, req.amount)
        else {
            return Err((
                StatusCode::BAD_REQUEST,
                "provide either quoteResponse or inputMint+outputMint+amount".to_string(),
            ));
        };
        let input_mint = parse_mint(input).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let output_mint = parse_mint(output).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let slippage_bps = req.slippage_bps.unwrap_or(50);
        let max_hops = req.max_hops.unwrap_or(2).min(4);

        let swappable = state.registry.read().await.swappable_set();
        let router = Router::new(&state.pool_index, max_hops)
            .with_swappable_set(swappable)
            .with_live_data(state.store.as_ref());
        let quote = router
            .find_routes(input_mint, output_mint, amount, 5)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("routing error: {e}")))?;
        if quote.routes.is_empty() {
            return Err((StatusCode::NOT_FOUND, "no route found".to_string()));
        }
        quote
            .routes
            .into_iter()
            .map(|r| {
                let min_out = calculate_min_amount_out(r.output_amount, slippage_bps);
                (r, min_out)
            })
            .collect()
    };

    // Max simulation attempts across ranked candidates (build/compile
    // failures are cheap local errors and do not consume an attempt).
    let max_sim_attempts = if req.quote_response.is_some() { 1 } else { 3 };
    let mut sim_attempts = 0usize;
    let mut last_error: String = "no viable route".to_string();

    for (route, min_out) in candidates {
        if sim_attempts >= max_sim_attempts {
            break;
        }
        ensure_route_accounts(state, &route).await?;
        let amount_in = route.input_amount;

        let cu_limit = route_compute_unit_limit(&route);
        let opts = SwapOptions {
            wrap_and_unwrap_sol,
            compute_unit_price_micro_lamports: fee_spec.price(cu_limit),
            compute_unit_limit: Some(cu_limit),
        };
        let built = {
            let registry = state.registry.read().await;
            build_swap_instructions(
                &route, &user, amount_in, min_out, &state.store, &registry, &opts,
            )
        };
        let bundle = match built {
            Ok(b) => b,
            Err(e) => {
                last_error = format!("tx build error: {e}");
                eprintln!("[swap] route via {} failed to build: {e}", route_label(&route));
                continue;
            }
        };
        let tx = match compile_to_transaction(&bundle, &user, &alts, blockhash) {
            Ok(tx) => tx,
            Err(e) => {
                last_error = format!("tx compile error: {e}");
                eprintln!("[swap] route via {} failed to compile: {e}", route_label(&route));
                continue;
            }
        };

        if skip_sim {
            return Ok(PreparedSwap {
                route,
                bundle,
                tx,
                blockhash,
                last_valid_block_height,
                alt_addresses,
                simulation_slot: None,
                units_consumed: None,
            });
        }

        // Pre-send simulation.
        sim_attempts += 1;
        let sim_config = solana_rpc_client_api::config::RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(solana_commitment_config::CommitmentConfig::confirmed()),
            ..Default::default()
        };
        let sim = match state.rpc.simulate_transaction_with_config(&tx, sim_config).await {
            Ok(sim) => sim,
            Err(e) => {
                last_error = format!("simulation RPC error: {e}");
                eprintln!("[swap] route via {}: {last_error}", route_label(&route));
                continue;
            }
        };
        if let Some(err) = &sim.value.err {
            let logs = sim.value.logs.clone().unwrap_or_default();
            last_error = format!("simulation failed: {err:?}; logs: {}", logs.join(" | "));
            eprintln!(
                "[swap] route via {} failed simulation: {err:?}",
                route_label(&route)
            );
            for log in &logs {
                eprintln!("[swap]   {log}");
            }
            continue;
        }

        let simulation_slot = Some(sim.context.slot);
        let units_consumed = sim.value.units_consumed;

        // Optionally re-tighten the CU limit to simulated consumption x 1.1.
        if dynamic_cu {
            if let Some(consumed) = units_consumed.filter(|c| *c > 0) {
                let tight_limit = ((consumed as f64 * 1.1) as u32)
                    .clamp(1_000, crate::swap::MAX_COMPUTE_UNIT_LIMIT);
                let tight_opts = SwapOptions {
                    wrap_and_unwrap_sol,
                    compute_unit_price_micro_lamports: fee_spec.price(tight_limit),
                    compute_unit_limit: Some(tight_limit),
                };
                let rebuilt = {
                    let registry = state.registry.read().await;
                    build_swap_instructions(
                        &route, &user, amount_in, min_out, &state.store, &registry, &tight_opts,
                    )
                };
                if let Ok(tight_bundle) = rebuilt {
                    if let Ok(tight_tx) =
                        compile_to_transaction(&tight_bundle, &user, &alts, blockhash)
                    {
                        return Ok(PreparedSwap {
                            route,
                            bundle: tight_bundle,
                            tx: tight_tx,
                            blockhash,
                            last_valid_block_height,
                            alt_addresses,
                            simulation_slot,
                            units_consumed,
                        });
                    }
                }
            }
        }

        return Ok(PreparedSwap {
            route,
            bundle,
            tx,
            blockhash,
            last_valid_block_height,
            alt_addresses,
            simulation_slot,
            units_consumed,
        });
    }

    Err((StatusCode::UNPROCESSABLE_ENTITY, last_error))
}

fn route_label(route: &Route) -> String {
    route
        .hops
        .iter()
        .map(|h| format!("{}({})", h.dex_name, &h.pool_address[..h.pool_address.len().min(6)]))
        .collect::<Vec<_>>()
        .join(" -> ")
}

async fn handle_swap(
    State(state): State<Arc<AppState>>,
    Json(params): Json<SwapRequest>,
) -> Result<Json<SwapResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let prepared = prepare_swap(&state, &params).await?;

    let tx_bytes = bincode::serialize(&prepared.tx)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    let tx_base64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    Ok(Json(SwapResponse {
        transaction: tx_base64.clone(),
        swap_transaction: tx_base64,
        route: route_to_json(&prepared.route),
        blockhash: prepared.blockhash.to_string(),
        last_valid_block_height: prepared.last_valid_block_height,
        compute_unit_limit: prepared.bundle.compute_unit_limit,
        prioritization_fee_lamports: total_priority_fee(
            prepared.bundle.compute_unit_limit,
            prepared.bundle.compute_unit_price_micro_lamports,
        ),
        simulation_slot: prepared.simulation_slot,
        units_consumed: prepared.units_consumed,
        time_taken_ms: start.elapsed().as_millis(),
    }))
}

async fn handle_swap_instructions(
    State(state): State<Arc<AppState>>,
    Json(params): Json<SwapRequest>,
) -> Result<Json<SwapInstructionsResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let prepared = prepare_swap(&state, &params).await?;
    let bundle = &prepared.bundle;

    Ok(Json(SwapInstructionsResponse {
        compute_budget_instructions: bundle.compute_budget.iter().map(ix_to_json).collect(),
        setup_instructions: bundle.setup.iter().map(ix_to_json).collect(),
        swap_instruction: ix_to_json(&bundle.swap),
        cleanup_instructions: bundle.cleanup.iter().map(ix_to_json).collect(),
        address_lookup_table_addresses: prepared.alt_addresses.clone(),
        route: route_to_json(&prepared.route),
        blockhash: prepared.blockhash.to_string(),
        last_valid_block_height: prepared.last_valid_block_height,
        compute_unit_limit: bundle.compute_unit_limit,
        prioritization_fee_lamports: total_priority_fee(
            bundle.compute_unit_limit,
            bundle.compute_unit_price_micro_lamports,
        ),
        simulation_slot: prepared.simulation_slot,
        units_consumed: prepared.units_consumed,
        time_taken_ms: start.elapsed().as_millis(),
    }))
}

// ---------------------------------------------------------------------------
// GET /price
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PriceParams {
    mint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PriceResponse {
    mint: String,
    price_sol: Option<f64>,
    price_usd: Option<f64>,
}

async fn handle_price(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PriceParams>,
) -> Result<Json<PriceResponse>, (StatusCode, String)> {
    let mint = parse_mint(&params.mint).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let sol_usd = *state.sol_usd_price.read().await;

    let token_price = price::get_token_price(&state.pool_index, &mint, sol_usd)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("price error: {e}")))?;

    Ok(Json(PriceResponse {
        mint: mint.to_string(),
        price_sol: token_price.price_sol,
        price_usd: token_price.price_usd,
    }))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    pools: usize,
    swappable_pools: usize,
    accounts_in_store: u64,
    last_slot: u64,
    uptime_seconds: u64,
}

async fn handle_health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let registry = state.registry.read().await;
    Json(HealthResponse {
        status: "ok",
        pools: registry.pool_count(),
        swappable_pools: registry.swappable_count(),
        accounts_in_store: state.store.len(),
        last_slot: state.store.last_slot(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
    })
}
