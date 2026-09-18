//! Client for the local Thunder engine (`GET /quote`, `GET /health`, `GET /price`).
//!
//! A refused connection (engine not running) is recorded as an explicit
//! per-case error outcome, never a crash — a run without the engine still
//! produces the two Jupiter columns.

use std::time::{Duration, Instant};

use serde::Deserialize;
use thunder_core::GenericError;

use crate::cases::BenchCase;
use crate::metrics::{SideResult, SideStatus};

pub const DEFAULT_THUNDER_URL: &str = "http://localhost:8080";

/// Percent-encode a query value. Keeps unreserved chars and ',' so
/// comma-separated lists stay readable; encodes space as %20 (never '+').
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Response shapes (mirror crates/engine/src/api.rs, camelCase)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThunderQuoteResponse {
    pub routes: Vec<ThunderRoute>,
    pub time_taken_ms: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThunderRoute {
    pub output_amount: String,
    /// Part of the /quote shape; currently hardcoded 0 server-side.
    #[allow(dead_code)]
    #[serde(default)]
    pub price_impact_bps: u64,
    pub hops: Vec<ThunderHop>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThunderHop {
    pub pool_address: String,
    /// Read by tests; part of the /quote shape.
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(default)]
    pub dex_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThunderHealth {
    last_slot: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThunderPrice {
    price_usd: Option<f64>,
}

/// Parse a /quote response body (pure, unit-testable).
pub fn parse_quote_response(body: &str) -> Result<ThunderQuoteResponse, GenericError> {
    Ok(serde_json::from_str(body)?)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct ThunderClient {
    base: String,
    http: reqwest::Client,
}

impl ThunderClient {
    pub fn new(base: String) -> Result<Self, GenericError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), http })
    }

    pub fn from_env() -> Result<Self, GenericError> {
        let base = std::env::var("THUNDER_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_THUNDER_URL.to_string());
        Self::new(base)
    }

    /// Quote one case. Never fails: transport problems become an error outcome.
    pub async fn quote(&self, case: &BenchCase, max_hops: u32) -> SideResult {
        let url = format!(
            "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}&maxHops={}",
            self.base,
            urlencode(&case.input_mint),
            urlencode(&case.output_mint),
            case.amount,
            case.slippage_bps,
            max_hops,
        );

        let start = Instant::now();
        let resp = match self.http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                let detail = if e.is_connect() {
                    format!("connection refused/unreachable at {} (is the engine running?)", self.base)
                } else {
                    format!("request failed: {e}")
                };
                return SideResult::error(detail, Some(ms_since(start)));
            }
        };
        let status = resp.status().as_u16();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => return SideResult::error(format!("body read failed: {e}"), Some(ms_since(start))),
        };
        let rtt_ms = ms_since(start);

        if status != 200 {
            return SideResult::error(
                format!("HTTP {status}: {}", truncate(&body, 200)),
                Some(rtt_ms),
            );
        }

        let parsed = match parse_quote_response(&body) {
            Ok(p) => p,
            Err(e) => {
                return SideResult::error(
                    format!("unexpected /quote shape: {e}; body: {}", truncate(&body, 200)),
                    Some(rtt_ms),
                );
            }
        };

        let Some(best) = parsed.routes.first() else {
            return SideResult::no_route(None, Some(rtt_ms));
        };
        let out_amount = match best.output_amount.parse::<u64>() {
            Ok(v) => v,
            Err(e) => {
                return SideResult::error(
                    format!("bad outputAmount '{}': {e}", best.output_amount),
                    Some(rtt_ms),
                );
            }
        };

        SideResult {
            status: SideStatus::Ok,
            out_amount: Some(out_amount),
            server_ms: Some(parsed.time_taken_ms),
            rtt_ms: Some(rtt_ms),
            pools: best.hops.iter().map(|h| h.pool_address.clone()).collect(),
            slot_hint: self.slot_hint().await,
            labels: Vec::new(),
            error: None,
        }
    }

    /// Best-effort slot indicator from /health (`lastSlot`).
    async fn slot_hint(&self) -> Option<u64> {
        let url = format!("{}/health", self.base);
        let resp = self.http.get(&url).send().await.ok()?;
        let health: ThunderHealth = resp.json().await.ok()?;
        (health.last_slot > 0).then_some(health.last_slot)
    }

    /// SOL/USD from the engine's /price endpoint (used by gen-cases when
    /// --sol-price is not given).
    pub async fn sol_usd_price(&self) -> Result<f64, GenericError> {
        let url = format!("{}/price?mint=SOL", self.base);
        let resp = self.http.get(&url).send().await.map_err(|e| {
            format!("could not reach engine at {} for SOL price: {e}", self.base)
        })?;
        let price: ThunderPrice = resp.json().await.map_err(|e| format!("bad /price response: {e}"))?;
        price
            .price_usd
            .filter(|p| p.is_finite() && *p > 0.0)
            .ok_or_else(|| "engine returned no SOL/USD price yet".into())
    }
}

fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

pub fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canned /quote response (shape from crates/engine/src/api.rs).
    const CANNED: &str = r#"{"inputMint":"So11111111111111111111111111111111111111112","outputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","amount":1000000000,"slippageBps":50,"timeTakenMs":3,"routes":[{"outputAmount":"123456","priceImpactBps":0,"hops":[{"poolAddress":"5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6","dexName":"Meteora DLMM","inputMint":"So11111111111111111111111111111111111111112","outputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","inputAmount":"1000000000","outputAmount":"123456"}]}]}"#;

    #[test]
    fn parses_canned_quote() {
        let parsed = parse_quote_response(CANNED).unwrap();
        assert_eq!(parsed.time_taken_ms, 3.0);
        assert_eq!(parsed.routes.len(), 1);
        let best = &parsed.routes[0];
        assert_eq!(best.output_amount, "123456");
        assert_eq!(best.output_amount.parse::<u64>().unwrap(), 123_456);
        assert_eq!(best.hops.len(), 1);
        assert_eq!(best.hops[0].pool_address, "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6");
        assert_eq!(best.hops[0].dex_name, "Meteora DLMM");
    }

    #[test]
    fn empty_routes_is_no_route_shape() {
        let body = r#"{"inputMint":"a","outputMint":"b","amount":"1","slippageBps":50,"timeTakenMs":1,"routes":[]}"#;
        let parsed = parse_quote_response(body).unwrap();
        assert!(parsed.routes.is_empty());
    }

    #[test]
    fn garbage_body_is_an_error() {
        assert!(parse_quote_response("<html>502 bad gateway</html>").is_err());
    }

    #[test]
    fn urlencode_keeps_base58_and_commas() {
        assert_eq!(urlencode("So11111111111111111111111111111111111111112"),
                   "So11111111111111111111111111111111111111112");
        assert_eq!(urlencode("Raydium CLMM,Meteora DLMM"), "Raydium%20CLMM,Meteora%20DLMM");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }

    #[tokio::test]
    async fn connection_refused_is_recorded_not_fatal() {
        // Port 1 is never listening.
        let client = ThunderClient::new("http://127.0.0.1:1".into()).unwrap();
        let case = BenchCase {
            id: "t".into(),
            input_mint: "So11111111111111111111111111111111111111112".into(),
            output_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            amount: 1_000_000_000,
            usd_size: 100.0,
            liquidity_tier: "major".into(),
            slippage_bps: 50,
        };
        let result = client.quote(&case, 2).await;
        assert_eq!(result.status, SideStatus::Error);
        let msg = result.error.unwrap();
        assert!(msg.contains("engine running"), "unexpected error message: {msg}");
    }
}
