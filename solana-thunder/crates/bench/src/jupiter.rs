//! Jupiter swap-API client: paced quotes, response caching, 429 backoff,
//! and runtime DEX-label discovery.
//!
//! Endpoints (v1 quote API):
//!   keyless   https://lite-api.jup.ag/swap/v1/quote
//!   with key  https://api.jup.ag/swap/v1/quote   (header `x-api-key`)
//!   self-host `JUPITER_BASE_URL=http://host:port` (jupiter-swap-api)

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thunder_core::GenericError;

use crate::metrics::{SideResult, SideStatus};
use crate::thunder::{truncate, urlencode};

pub const LITE_BASE_URL: &str = "https://lite-api.jup.ag/swap/v1";
pub const PRO_BASE_URL: &str = "https://api.jup.ag/swap/v1";

/// Keyless lite-api pacing (documented ~0.5 RPS; stay under it).
pub const LITE_RPS: f64 = 0.45;
/// Free-key pacing (documented 1 RPS; stay under it).
pub const KEYED_RPS: f64 = 0.9;

const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct JupiterConfig {
    pub base: String,
    pub api_key: Option<String>,
    pub rps: f64,
}

impl JupiterConfig {
    /// Resolve from env: JUPITER_API_KEY, JUPITER_BASE_URL, BENCH_JUP_RPS.
    pub fn from_env() -> Self {
        let api_key = std::env::var("JUPITER_API_KEY").ok().filter(|s| !s.is_empty());
        let base = std::env::var("JUPITER_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if api_key.is_some() { PRO_BASE_URL } else { LITE_BASE_URL }.to_string()
            });
        let rps = std::env::var("BENCH_JUP_RPS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|r| *r > 0.0)
            .unwrap_or(if api_key.is_some() { KEYED_RPS } else { LITE_RPS });
        Self { base: base.trim_end_matches('/').to_string(), api_key, rps }
    }
}

// ---------------------------------------------------------------------------
// Token-bucket pacing (capacity 1)
// ---------------------------------------------------------------------------

struct Pacer {
    interval: Duration,
    next: tokio::sync::Mutex<tokio::time::Instant>,
}

impl Pacer {
    fn new(rps: f64) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / rps),
            next: tokio::sync::Mutex::new(tokio::time::Instant::now()),
        }
    }

    /// Wait until a request slot is available and claim it.
    async fn acquire(&self) {
        let mut next = self.next.lock().await;
        let now = tokio::time::Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = (*next).max(now) + self.interval;
    }
}

// ---------------------------------------------------------------------------
// Response cache (runs/jupiter_cache.jsonl)
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit — stable across builds (std's DefaultHasher is not).
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// Hex FNV-1a of `params` (the cache key).
    key: String,
    /// Canonical parameter string (for debuggability).
    params: String,
    ts: u64,
    status: u16,
    body: String,
}

/// One HTTP fetch result (possibly served from cache).
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub status: u16,
    pub body: String,
    /// None when served from the local response cache.
    pub rtt_ms: Option<f64>,
    /// Kept for debugging / future progress display.
    #[allow(dead_code)]
    pub from_cache: bool,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct JupiterClient {
    http: reqwest::Client,
    config: JupiterConfig,
    pacer: Pacer,
    cache: tokio::sync::Mutex<HashMap<String, CacheEntry>>,
    cache_path: PathBuf,
    /// Cache entries younger than this many seconds are reused (0 = never reuse).
    max_cache_age: u64,
    /// Set once the API rejects `restrictIntermediateTokens=false`
    /// (lite-api free tier does); subsequent requests omit the param.
    rit_false_unsupported: std::sync::atomic::AtomicBool,
}

impl JupiterClient {
    pub fn new(
        config: JupiterConfig,
        cache_path: PathBuf,
        max_cache_age: u64,
    ) -> Result<Self, GenericError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let cache = load_cache_file(&cache_path);
        if !cache.is_empty() {
            eprintln!(
                "jupiter cache: {} entries loaded from {}",
                cache.len(),
                cache_path.display()
            );
        }
        Ok(Self {
            http,
            pacer: Pacer::new(config.rps),
            config,
            cache: tokio::sync::Mutex::new(cache),
            cache_path,
            max_cache_age,
            rit_false_unsupported: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn base(&self) -> &str {
        &self.config.base
    }

    pub fn rps(&self) -> f64 {
        self.config.rps
    }

    /// GET /quote (ExactIn). `dexes = None` -> unrestricted.
    ///
    /// Sends `restrictIntermediateTokens=false` (widest route search) unless
    /// the API has already rejected it: the lite-api free tier answers
    /// HTTP 400 NOT_SUPPORTED, in which case we retry once without the param
    /// (server default is the restricted intermediate set) and omit it from
    /// then on.
    pub async fn quote_raw(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u64,
        dexes: Option<&str>,
    ) -> Result<FetchResult, GenericError> {
        use std::sync::atomic::Ordering;

        let build_query = |with_rit_false: bool| {
            let mut query = format!(
                "inputMint={}&outputMint={}&amount={}&slippageBps={}&swapMode=ExactIn",
                urlencode(input_mint),
                urlencode(output_mint),
                amount,
                slippage_bps,
            );
            if with_rit_false {
                query.push_str("&restrictIntermediateTokens=false");
            }
            if let Some(d) = dexes {
                query.push_str("&dexes=");
                query.push_str(&urlencode(d));
            }
            query
        };

        let mut with_rit_false = !self.rit_false_unsupported.load(Ordering::Relaxed);
        loop {
            let query = build_query(with_rit_false);
            let url = format!("{}/quote?{query}", self.config.base);
            // Cache key: base + full query (base included so lite/pro/
            // self-hosted responses never cross-contaminate).
            let params = format!("{}|{query}", self.config.base);
            let fetch = self.fetch_cached(&url, &params).await?;

            if with_rit_false
                && fetch.status == 400
                && fetch.body.contains("restrict_intermediate_tokens")
            {
                if !self.rit_false_unsupported.swap(true, Ordering::Relaxed) {
                    eprintln!(
                        "note: this Jupiter tier rejects restrictIntermediateTokens=false; \
                         retrying without it (server default applies)"
                    );
                }
                with_rit_false = false;
                continue;
            }
            return Ok(fetch);
        }
    }

    /// GET with the response cache, pacing, and 429 backoff.
    async fn fetch_cached(&self, url: &str, params: &str) -> Result<FetchResult, GenericError> {
        let key = format!("{:016x}", fnv1a64(params));

        if self.max_cache_age > 0 {
            let cache = self.cache.lock().await;
            if let Some(entry) = cache.get(&key)
                && now_secs().saturating_sub(entry.ts) <= self.max_cache_age
            {
                return Ok(FetchResult {
                    status: entry.status,
                    body: entry.body.clone(),
                    rtt_ms: None,
                    from_cache: true,
                });
            }
        }

        let mut backoff = BACKOFF_BASE;
        for attempt in 1..=MAX_ATTEMPTS {
            self.pacer.acquire().await;
            let start = Instant::now();
            let mut req = self.http.get(url);
            if let Some(key) = &self.config.api_key {
                req = req.header("x-api-key", key);
            }
            let resp = req.send().await.map_err(|e| format!("jupiter request failed: {e}"))?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = resp.text().await.map_err(|e| format!("jupiter body read failed: {e}"))?;
            let rtt_ms = start.elapsed().as_secs_f64() * 1_000.0;

            if status == 429 {
                if attempt == MAX_ATTEMPTS {
                    return Err(format!(
                        "jupiter rate limited (429) after {MAX_ATTEMPTS} attempts: {}",
                        truncate(&body, 120)
                    )
                    .into());
                }
                let wait = retry_after.unwrap_or(backoff).max(backoff);
                eprintln!(
                    "  jupiter 429, backing off {:.1}s (attempt {attempt}/{MAX_ATTEMPTS})",
                    wait.as_secs_f64()
                );
                tokio::time::sleep(wait).await;
                backoff *= 2;
                continue;
            }

            // Persist every final response (200s and no-route 4xxs alike).
            let entry = CacheEntry {
                key: key.clone(),
                params: params.to_string(),
                ts: now_secs(),
                status,
                body: body.clone(),
            };
            self.append_cache(entry).await;

            return Ok(FetchResult { status, body, rtt_ms: Some(rtt_ms), from_cache: false });
        }
        unreachable!("retry loop always returns")
    }

    async fn append_cache(&self, entry: CacheEntry) {
        if let Some(parent) = self.cache_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cache_path)
            && let Ok(line) = serde_json::to_string(&entry)
        {
            let _ = writeln!(f, "{line}");
        }
        self.cache.lock().await.insert(entry.key.clone(), entry);
    }

    /// Quote one case side and convert to a recorded outcome (never panics).
    pub async fn quote_side(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u64,
        dexes: Option<&str>,
    ) -> SideResult {
        match self.quote_raw(input_mint, output_mint, amount, slippage_bps, dexes).await {
            Ok(fetch) => outcome_from_response(fetch.status, &fetch.body, fetch.rtt_ms),
            Err(e) => SideResult::error(e.to_string(), None),
        }
    }

    /// Discover available DEX labels: one unrestricted SOL->USDC quote at
    /// 1000 SOL (labels of a heavily-split route), enriched by
    /// GET /program-id-to-label when available.
    pub async fn discover_labels(&self) -> Result<Vec<String>, GenericError> {
        use thunder_core::{USDC, WSOL};
        let mut labels: Vec<String> = Vec::new();

        let fetch = self
            .quote_raw(WSOL, USDC, 1_000_000_000_000, 50, None) // 1000 SOL
            .await?;
        if fetch.status == 200 {
            if let Ok(parsed) = serde_json::from_str::<JupQuoteResponse>(&fetch.body) {
                for plan in &parsed.route_plan {
                    if let Some(label) = &plan.swap_info.label
                        && !labels.iter().any(|l| l == label)
                    {
                        labels.push(label.clone());
                    }
                }
            }
        } else {
            eprintln!(
                "warning: label-discovery quote returned HTTP {}: {}",
                fetch.status,
                truncate(&fetch.body, 120)
            );
        }

        // Enrichment: the authoritative label list (single extra request).
        let url = format!("{}/program-id-to-label", self.config.base);
        let params = format!("{}|program-id-to-label", self.config.base);
        match self.fetch_cached(&url, &params).await {
            Ok(fetch) if fetch.status == 200 => {
                if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&fetch.body) {
                    // Sorted for deterministic resolution (HashMap order is random).
                    let mut extra: Vec<String> = map.into_values().collect();
                    extra.sort();
                    extra.dedup();
                    for label in extra {
                        if !labels.iter().any(|l| *l == label) {
                            labels.push(label);
                        }
                    }
                }
            }
            Ok(fetch) => eprintln!(
                "note: /program-id-to-label returned HTTP {} (continuing with routePlan labels)",
                fetch.status
            ),
            Err(e) => eprintln!("note: /program-id-to-label failed: {e} (continuing with routePlan labels)"),
        }

        if labels.is_empty() {
            return Err("label discovery found no DEX labels".into());
        }
        Ok(labels)
    }
}

fn load_cache_file(path: &Path) -> HashMap<String, CacheEntry> {
    let mut map = HashMap::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<CacheEntry>(line) {
            // Later lines win (newest response for a key).
            map.insert(entry.key.clone(), entry);
        }
    }
    map
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupQuoteResponse {
    pub out_amount: String,
    #[serde(default)]
    pub context_slot: Option<u64>,
    /// Server-reported quote time in SECONDS.
    #[serde(default)]
    pub time_taken: Option<f64>,
    #[serde(default)]
    pub route_plan: Vec<JupRoutePlan>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupRoutePlan {
    pub swap_info: JupSwapInfo,
    /// Split percentage — part of the API shape, unused for now.
    #[allow(dead_code)]
    #[serde(default)]
    pub percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupSwapInfo {
    pub amm_key: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JupErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default, rename = "errorCode")]
    error_code: Option<String>,
}

/// Convert an HTTP response into a recorded outcome. Jupiter reports
/// "no route" as a 4xx with an errorCode — an explicit outcome, not a crash.
pub fn outcome_from_response(status: u16, body: &str, rtt_ms: Option<f64>) -> SideResult {
    if status == 200 {
        let parsed: JupQuoteResponse = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(e) => {
                return SideResult::error(
                    format!("unexpected jupiter quote shape: {e}; body: {}", truncate(body, 200)),
                    rtt_ms,
                );
            }
        };
        let out_amount = match parsed.out_amount.parse::<u64>() {
            Ok(v) => v,
            Err(e) => {
                return SideResult::error(
                    format!("bad outAmount '{}': {e}", parsed.out_amount),
                    rtt_ms,
                );
            }
        };
        let mut labels = Vec::new();
        for plan in &parsed.route_plan {
            if let Some(label) = &plan.swap_info.label
                && !labels.iter().any(|l| l == label)
            {
                labels.push(label.clone());
            }
        }
        return SideResult {
            status: SideStatus::Ok,
            out_amount: Some(out_amount),
            server_ms: parsed.time_taken.map(|s| s * 1_000.0),
            rtt_ms,
            pools: parsed.route_plan.iter().map(|p| p.swap_info.amm_key.clone()).collect(),
            slot_hint: parsed.context_slot,
            labels,
            error: None,
        };
    }

    // 4xx: distinguish "no route" from genuine errors.
    let err: JupErrorBody = serde_json::from_str(body).unwrap_or(JupErrorBody {
        error: None,
        error_code: None,
    });
    let text = format!(
        "{} {}",
        err.error_code.clone().unwrap_or_default(),
        err.error.clone().unwrap_or_default()
    )
    .to_ascii_uppercase();
    let is_no_route = (400..500).contains(&status)
        && (text.contains("COULD_NOT_FIND_ANY_ROUTE")
            || text.contains("NO_ROUTES_FOUND")
            || text.contains("NO ROUTE")
            || text.contains("TOKEN_NOT_TRADABLE")
            || text.contains("NOT TRADABLE"));
    if is_no_route {
        let detail = err.error_code.or(err.error);
        return SideResult::no_route(detail, rtt_ms);
    }
    SideResult::error(format!("HTTP {status}: {}", truncate(body, 200)), rtt_ms)
}

// ---------------------------------------------------------------------------
// DEX label resolution
// ---------------------------------------------------------------------------

/// Our six adapters, in display order.
pub const DEX_TARGETS: [&str; 6] = [
    "Raydium V4",
    "Raydium CLMM",
    "Meteora DAMM v1",
    "Meteora DAMM v2",
    "Meteora DLMM",
    "Pump.fun AMM",
];

fn normalize(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn matches_target(target: &str, n: &str) -> bool {
    match target {
        "Raydium CLMM" => n.contains("raydium") && n.contains("clmm"),
        // Plain "Raydium" is the V4 AMM. Exclude CLMM/CP/CPMM/Launchlab variants.
        "Raydium V4" => {
            n == "raydium"
                || n == "raydiumamm"
                || n == "raydiumv4"
                || n == "raydiumammv4"
                || (n.contains("raydium")
                    && n.contains("v4")
                    && !n.contains("clmm")
                    && !n.contains("cp"))
        }
        // Must be Meteora's DLMM specifically — "Saros DLMM" also exists.
        "Meteora DLMM" => n.contains("meteora") && n.contains("dlmm"),
        "Meteora DAMM v2" => {
            n.contains("meteora") && (n.contains("dammv2") || n.contains("damm2"))
        }
        // Historical label for DAMM v1 pools is plain "Meteora" / "Meteora Pools".
        "Meteora DAMM v1" => {
            n == "meteora"
                || n == "meteorapools"
                || (n.contains("meteora")
                    && n.contains("damm")
                    && !n.contains("dammv2")
                    && !n.contains("damm2"))
        }
        // PumpSwap AMM ("Pump.fun Amm"), not the "Pump.fun" bonding-curve launchpad.
        "Pump.fun AMM" => n.contains("pump") && (n.contains("amm") || n.contains("swap")),
        _ => false,
    }
}

/// Fuzzy-match discovered labels against our adapters.
/// Returns `(target, Some(exact Jupiter label))` or `(target, None)` if unresolved.
pub fn resolve_dex_labels(available: &[String]) -> Vec<(&'static str, Option<String>)> {
    DEX_TARGETS
        .iter()
        .map(|&target| {
            let hit = available
                .iter()
                .find(|label| matches_target(target, &normalize(label)))
                .cloned();
            (target, hit)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_deterministic() {
        // Known FNV-1a 64 test vectors.
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64("foobar"), 0x8594_4171_f739_67e8);
        assert_ne!(fnv1a64("inputMint=A"), fnv1a64("inputMint=B"));
    }

    #[test]
    fn resolves_realistic_label_set() {
        let available: Vec<String> = [
            "Raydium",
            "Raydium CLMM",
            "Raydium CP",
            "Meteora",
            "Meteora DLMM",
            "Meteora DAMM v2",
            "Pump.fun",
            "Pump.fun Amm",
            "Whirlpool",
            "Obric V2",
            "SolFi",
            "Lifinity V2",
            "Saros DLMM", // must NOT be mistaken for Meteora DLMM
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let resolved = resolve_dex_labels(&available);
        let get = |t: &str| {
            resolved
                .iter()
                .find(|(target, _)| *target == t)
                .and_then(|(_, l)| l.clone())
        };
        assert_eq!(get("Raydium V4").as_deref(), Some("Raydium"));
        assert_eq!(get("Raydium CLMM").as_deref(), Some("Raydium CLMM"));
        assert_eq!(get("Meteora DAMM v1").as_deref(), Some("Meteora"));
        assert_eq!(get("Meteora DAMM v2").as_deref(), Some("Meteora DAMM v2"));
        assert_eq!(get("Meteora DLMM").as_deref(), Some("Meteora DLMM"));
        // Must pick the AMM (PumpSwap), not the bonding-curve launchpad.
        assert_eq!(get("Pump.fun AMM").as_deref(), Some("Pump.fun Amm"));
    }

    #[test]
    fn unresolved_labels_are_none_not_wrong() {
        let available: Vec<String> = ["Whirlpool", "Orca V2"].iter().map(|s| s.to_string()).collect();
        for (_, label) in resolve_dex_labels(&available) {
            assert!(label.is_none());
        }
    }

    #[test]
    fn ok_response_parses() {
        let body = r#"{
            "inputMint":"So11111111111111111111111111111111111111112",
            "inAmount":"1000000000",
            "outputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "outAmount":"157123456",
            "otherAmountThreshold":"156337839",
            "swapMode":"ExactIn",
            "slippageBps":50,
            "priceImpactPct":"0.0001",
            "routePlan":[
              {"swapInfo":{"ammKey":"Amm1","label":"Raydium","inputMint":"a","outputMint":"b","inAmount":"1","outAmount":"2","feeAmount":"0","feeMint":"a"},"percent":60},
              {"swapInfo":{"ammKey":"Amm2","label":"Meteora DLMM","inputMint":"a","outputMint":"b","inAmount":"1","outAmount":"2","feeAmount":"0","feeMint":"a"},"percent":40}
            ],
            "contextSlot":354000123,
            "timeTaken":0.0421
        }"#;
        let side = outcome_from_response(200, body, Some(210.0));
        assert_eq!(side.status, SideStatus::Ok);
        assert_eq!(side.out_amount, Some(157_123_456));
        assert_eq!(side.pools, vec!["Amm1".to_string(), "Amm2".to_string()]);
        assert_eq!(side.labels, vec!["Raydium".to_string(), "Meteora DLMM".to_string()]);
        assert_eq!(side.slot_hint, Some(354_000_123));
        assert!((side.server_ms.unwrap() - 42.1).abs() < 1e-9);
    }

    #[test]
    fn no_route_is_explicit_outcome() {
        let body = r#"{"error":"Could not find any route","errorCode":"COULD_NOT_FIND_ANY_ROUTE"}"#;
        let side = outcome_from_response(400, body, Some(120.0));
        assert_eq!(side.status, SideStatus::NoRoute);
        assert_eq!(side.error.as_deref(), Some("COULD_NOT_FIND_ANY_ROUTE"));

        let body = r#"{"error":"Token not tradable","errorCode":"TOKEN_NOT_TRADABLE"}"#;
        assert_eq!(outcome_from_response(400, body, None).status, SideStatus::NoRoute);
    }

    #[test]
    fn other_errors_stay_errors() {
        assert_eq!(
            outcome_from_response(400, r#"{"error":"invalid amount"}"#, None).status,
            SideStatus::Error
        );
        assert_eq!(outcome_from_response(500, "internal", None).status, SideStatus::Error);
        assert_eq!(outcome_from_response(200, "not json", None).status, SideStatus::Error);
    }
}
