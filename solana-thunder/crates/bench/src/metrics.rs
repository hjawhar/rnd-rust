//! Per-case benchmark records and aggregation math.
//!
//! A run file is JSONL: one [`CaseRecord`] per line. Every side (ours,
//! Jupiter restricted, Jupiter unrestricted) is a [`SideResult`] whose
//! `status` makes "no route" and transport errors explicit, recorded
//! outcomes rather than crashes.

use serde::{Deserialize, Serialize};

use crate::cases::BenchCase;

/// |delta| <= 1 bp counts as a tie.
pub const TIE_BAND_BPS: f64 = 1.0;

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideStatus {
    Ok,
    NoRoute,
    Error,
}

/// One side's quote result for a single case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SideResult {
    pub status: SideStatus,
    /// Best-route output amount in raw units (parsed from the stringified u64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_amount: Option<u64>,
    /// Server-reported quote time in ms (Thunder `timeTakenMs`, Jupiter `timeTaken`*1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_ms: Option<f64>,
    /// Wall-clock round trip in ms as measured by this harness.
    /// `None` for Jupiter responses served from the local response cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    /// Pool addresses of the best route's hops (Thunder `poolAddress`,
    /// Jupiter `routePlan[].swapInfo.ammKey`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pools: Vec<String>,
    /// Best available slot indicator (Thunder /health `lastSlot`, Jupiter `contextSlot`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_hint: Option<u64>,
    /// Jupiter only: `routePlan[].swapInfo.label` values of the best route.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SideResult {
    pub fn error(msg: impl Into<String>, rtt_ms: Option<f64>) -> Self {
        Self {
            status: SideStatus::Error,
            out_amount: None,
            server_ms: None,
            rtt_ms,
            pools: Vec::new(),
            slot_hint: None,
            labels: Vec::new(),
            error: Some(msg.into()),
        }
    }

    pub fn no_route(detail: Option<String>, rtt_ms: Option<f64>) -> Self {
        Self {
            status: SideStatus::NoRoute,
            out_amount: None,
            server_ms: None,
            rtt_ms,
            pools: Vec::new(),
            slot_hint: None,
            labels: Vec::new(),
            error: detail,
        }
    }
}

/// One JSONL line of a run file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseRecord {
    pub case: BenchCase,
    pub ours: SideResult,
    pub jup_restricted: SideResult,
    pub jup_unrestricted: SideResult,
    /// Unix seconds at record time.
    pub ts: u64,
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Tie,
    Loss,
    OursNoRoute,
    JupNoRoute,
    BothNoRoute,
    Error,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Win => "win",
            Outcome::Tie => "tie",
            Outcome::Loss => "loss",
            Outcome::OursNoRoute => "ours_no_route",
            Outcome::JupNoRoute => "jup_no_route",
            Outcome::BothNoRoute => "both_no_route",
            Outcome::Error => "error",
        }
    }
}

/// delta_bps = (ours - jup) / jup * 1e4. Positive means we beat Jupiter.
pub fn delta_bps(ours: u64, jup: u64) -> f64 {
    if jup == 0 {
        return f64::INFINITY;
    }
    (ours as f64 - jup as f64) / jup as f64 * 10_000.0
}

/// Classify one case against one Jupiter column.
pub fn classify(ours: &SideResult, jup: &SideResult) -> Outcome {
    match (ours.status, jup.status) {
        (SideStatus::Ok, SideStatus::Ok) => match (ours.out_amount, jup.out_amount) {
            (Some(a), Some(b)) => {
                let d = delta_bps(a, b);
                if d.abs() <= TIE_BAND_BPS {
                    Outcome::Tie
                } else if d > 0.0 {
                    Outcome::Win
                } else {
                    Outcome::Loss
                }
            }
            _ => Outcome::Error,
        },
        (SideStatus::NoRoute, SideStatus::NoRoute) => Outcome::BothNoRoute,
        (SideStatus::NoRoute, SideStatus::Ok) => Outcome::OursNoRoute,
        (SideStatus::Ok, SideStatus::NoRoute) => Outcome::JupNoRoute,
        _ => Outcome::Error,
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize)]
pub struct Agg {
    pub comparable: usize,
    pub win: usize,
    pub tie: usize,
    pub loss: usize,
    pub ours_no_route: usize,
    pub jup_no_route: usize,
    pub both_no_route: usize,
    pub errors: usize,
    /// Deltas (bps) for the comparable cases.
    #[serde(skip)]
    pub deltas: Vec<f64>,
}

impl Agg {
    pub fn add(&mut self, ours: &SideResult, jup: &SideResult) {
        match classify(ours, jup) {
            Outcome::Win => {
                self.win += 1;
                self.comparable += 1;
            }
            Outcome::Tie => {
                self.tie += 1;
                self.comparable += 1;
            }
            Outcome::Loss => {
                self.loss += 1;
                self.comparable += 1;
            }
            Outcome::OursNoRoute => self.ours_no_route += 1,
            Outcome::JupNoRoute => self.jup_no_route += 1,
            Outcome::BothNoRoute => self.both_no_route += 1,
            Outcome::Error => self.errors += 1,
        }
        if let (SideStatus::Ok, SideStatus::Ok) = (ours.status, jup.status)
            && let (Some(a), Some(b)) = (ours.out_amount, jup.out_amount)
        {
            self.deltas.push(delta_bps(a, b));
        }
    }

    pub fn win_rate(&self) -> Option<f64> {
        if self.comparable == 0 {
            None
        } else {
            Some(self.win as f64 / self.comparable as f64)
        }
    }

    pub fn mean_delta(&self) -> Option<f64> {
        mean(&self.deltas)
    }

    pub fn median_delta(&self) -> Option<f64> {
        let mut v = self.deltas.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        percentile(&v, 50.0)
    }
}

/// Cell of the win-rate matrix (usd_size bucket x liquidity tier).
#[derive(Debug, Default, Clone, Serialize)]
pub struct MatrixCell {
    pub win: usize,
    pub tie: usize,
    pub loss: usize,
    /// no-route / error cases that could not be compared.
    pub other: usize,
}

impl MatrixCell {
    pub fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Win => self.win += 1,
            Outcome::Tie => self.tie += 1,
            Outcome::Loss => self.loss += 1,
            _ => self.other += 1,
        }
    }
}

/// Bucket key for a usd_size: cents, so f64 ladder values order and hash
/// deterministically.
pub fn size_bucket(usd: f64) -> u64 {
    (usd * 100.0).round() as u64
}

/// Mean of a slice (None when empty).
pub fn mean(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        None
    } else {
        Some(v.iter().sum::<f64>() / v.len() as f64)
    }
}

/// Nearest-rank percentile over an ASCENDING-sorted slice. `p` in [0, 100].
pub fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, n) - 1])
}

/// Jaccard similarity of two pool-address sets. None when both are empty.
pub fn jaccard(a: &[String], b: &[String]) -> Option<f64> {
    use std::collections::HashSet;
    let sa: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let sb: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let union = sa.union(&sb).count();
    if union == 0 {
        return None;
    }
    let inter = sa.intersection(&sb).count();
    Some(inter as f64 / union as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(out: u64) -> SideResult {
        SideResult {
            status: SideStatus::Ok,
            out_amount: Some(out),
            server_ms: Some(3.0),
            rtt_ms: Some(12.5),
            pools: vec!["poolA".into()],
            slot_hint: Some(31337),
            labels: Vec::new(),
            error: None,
        }
    }

    fn case() -> BenchCase {
        BenchCase {
            id: "sol-usdc-100usd-fwd".into(),
            input_mint: "So11111111111111111111111111111111111111112".into(),
            output_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            amount: 625_000_000,
            usd_size: 100.0,
            liquidity_tier: "major".into(),
            slippage_bps: 50,
        }
    }

    #[test]
    fn delta_bps_math() {
        assert_eq!(delta_bps(10_000, 10_000), 0.0);
        assert!((delta_bps(10_001, 10_000) - 1.0).abs() < 1e-9);
        assert!((delta_bps(9_999, 10_000) + 1.0).abs() < 1e-9);
        assert!((delta_bps(10_100, 10_000) - 100.0).abs() < 1e-9);
        assert!(delta_bps(1, 0).is_infinite());
    }

    #[test]
    fn win_tie_loss_boundaries() {
        // Exactly +1 bp and -1 bp are ties; beyond is win/loss.
        assert_eq!(classify(&ok(10_001), &ok(10_000)), Outcome::Tie);
        assert_eq!(classify(&ok(9_999), &ok(10_000)), Outcome::Tie);
        assert_eq!(classify(&ok(10_002), &ok(10_000)), Outcome::Win);
        assert_eq!(classify(&ok(9_997), &ok(10_000)), Outcome::Loss);
        assert_eq!(classify(&ok(10_000), &ok(10_000)), Outcome::Tie);
    }

    #[test]
    fn classify_no_route_and_error() {
        let nr = SideResult::no_route(None, Some(5.0));
        let err = SideResult::error("connection refused", None);
        assert_eq!(classify(&nr, &ok(1)), Outcome::OursNoRoute);
        assert_eq!(classify(&ok(1), &nr), Outcome::JupNoRoute);
        assert_eq!(classify(&nr, &nr), Outcome::BothNoRoute);
        assert_eq!(classify(&err, &ok(1)), Outcome::Error);
        assert_eq!(classify(&ok(1), &err), Outcome::Error);
    }

    #[test]
    fn agg_counts_and_stats() {
        let mut agg = Agg::default();
        agg.add(&ok(10_100), &ok(10_000)); // win, +100 bps
        agg.add(&ok(10_000), &ok(10_000)); // tie, 0 bps
        agg.add(&ok(9_900), &ok(10_000)); // loss, -100 bps
        agg.add(&SideResult::no_route(None, None), &ok(10_000));
        agg.add(&SideResult::error("x", None), &ok(10_000));
        assert_eq!(
            (agg.win, agg.tie, agg.loss, agg.ours_no_route, agg.errors),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(agg.comparable, 3);
        assert!((agg.mean_delta().unwrap() - 0.0).abs() < 1e-9);
        assert!((agg.median_delta().unwrap() - 0.0).abs() < 1e-9);
        assert!((agg.win_rate().unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn size_bucketing() {
        assert_eq!(size_bucket(0.1), 10);
        assert_eq!(size_bucket(1.0), 100);
        assert_eq!(size_bucket(100_000.0), 10_000_000);
        // Distinct ladder values map to distinct, ordered buckets.
        let ladder = crate::cases::USD_LADDER;
        let buckets: Vec<u64> = ladder.iter().map(|&u| size_bucket(u)).collect();
        let mut sorted = buckets.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(buckets, sorted);
    }

    #[test]
    fn matrix_cell_add() {
        let mut cell = MatrixCell::default();
        cell.add(Outcome::Win);
        cell.add(Outcome::Win);
        cell.add(Outcome::Loss);
        cell.add(Outcome::BothNoRoute);
        assert_eq!((cell.win, cell.tie, cell.loss, cell.other), (2, 0, 1, 1));
    }

    #[test]
    fn percentiles() {
        let v: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        assert_eq!(percentile(&v, 50.0), Some(50.0));
        assert_eq!(percentile(&v, 95.0), Some(95.0));
        assert_eq!(percentile(&v, 100.0), Some(100.0));
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&[], 50.0), None);
        assert_eq!(percentile(&[7.0], 95.0), Some(7.0));
    }

    #[test]
    fn jaccard_sets() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "c".to_string()];
        assert!((jaccard(&a, &b).unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(jaccard(&a, &a), Some(1.0));
        assert_eq!(jaccard(&[], &[]), None);
        assert_eq!(jaccard(&a, &[]), Some(0.0));
    }

    #[test]
    fn jsonl_round_trip() {
        let rec = CaseRecord {
            case: case(),
            ours: ok(123_456),
            jup_restricted: SideResult {
                status: SideStatus::Ok,
                out_amount: Some(123_400),
                server_ms: Some(41.2),
                rtt_ms: Some(250.0),
                pools: vec!["ammKey1".into(), "ammKey2".into()],
                slot_hint: Some(354_000_001),
                labels: vec!["Raydium".into(), "Meteora DLMM".into()],
                error: None,
            },
            jup_unrestricted: SideResult::no_route(Some("COULD_NOT_FIND_ANY_ROUTE".into()), Some(180.0)),
            ts: 1_760_000_000,
        };
        let err_rec = CaseRecord {
            case: case(),
            ours: SideResult::error("connection refused (is the engine running?)", None),
            jup_restricted: ok(u64::MAX), // u64 extremes must round-trip exactly
            jup_unrestricted: ok(1),
            ts: 1_760_000_001,
        };

        for original in [rec, err_rec] {
            let line = serde_json::to_string(&original).unwrap();
            assert!(!line.contains('\n'));
            let parsed: CaseRecord = serde_json::from_str(&line).unwrap();
            assert_eq!(parsed, original);
        }
    }
}
