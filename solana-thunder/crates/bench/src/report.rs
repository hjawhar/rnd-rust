//! Report rendering: terminal tables (plain aligned text, no deps) plus
//! summary.csv / summary.json artifacts written next to the run file.
//! Also the `diff` regression mode between two runs.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use thunder_core::GenericError;

use crate::metrics::{
    Agg, CaseRecord, MatrixCell, SideStatus, classify, delta_bps, jaccard, mean,
    percentile, size_bucket,
};

// ---------------------------------------------------------------------------
// Run-file IO
// ---------------------------------------------------------------------------

/// Load a JSONL run file, warning on (and skipping) malformed lines.
pub fn load_records(path: &Path) -> Result<Vec<CaseRecord>, GenericError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read run file '{}': {e}", path.display()))?;
    let mut records = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CaseRecord>(line) {
            Ok(rec) => records.push(rec),
            Err(e) => eprintln!("warning: {}:{}: skipping malformed record: {e}", path.display(), i + 1),
        }
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// Plain text tables
// ---------------------------------------------------------------------------

/// Render an aligned text table. First column left-aligned, the rest right-aligned.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            widths[i] = widths[i].max(cell.len());
        }
    }
    let mut out = String::new();
    let fmt_row = |cells: &[String]| -> String {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate().take(cols) {
            if i > 0 {
                line.push_str("  ");
            }
            if i == 0 {
                line.push_str(&format!("{:<width$}", cell, width = widths[i]));
            } else {
                line.push_str(&format!("{:>width$}", cell, width = widths[i]));
            }
        }
        line.trim_end().to_string()
    };
    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    out.push_str(&fmt_row(&header_cells));
    out.push('\n');
    let total: usize = widths.iter().sum::<usize>() + 2 * (cols - 1);
    out.push_str(&"-".repeat(total));
    out.push('\n');
    for row in rows {
        out.push_str(&fmt_row(row));
        out.push('\n');
    }
    out
}

fn fmt_opt(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(x) => format!("{x:.decimals$}"),
        None => "-".to_string(),
    }
}

fn size_label(usd: f64) -> String {
    format!("${}", crate::cases::format_usd(usd))
}

// ---------------------------------------------------------------------------
// Summary artifacts
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LatencyStats {
    n: usize,
    p50: Option<f64>,
    p95: Option<f64>,
    mean: Option<f64>,
}

fn latency_stats(values: &mut Vec<f64>) -> LatencyStats {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    LatencyStats {
        n: values.len(),
        p50: percentile(values, 50.0),
        p95: percentile(values, 95.0),
        mean: mean(values),
    }
}

#[derive(Serialize)]
struct ColumnSummary {
    comparable: usize,
    win: usize,
    tie: usize,
    loss: usize,
    win_rate: Option<f64>,
    mean_delta_bps: Option<f64>,
    median_delta_bps: Option<f64>,
    ours_no_route: usize,
    jup_no_route: usize,
    both_no_route: usize,
    errors: usize,
}

impl From<&Agg> for ColumnSummary {
    fn from(a: &Agg) -> Self {
        Self {
            comparable: a.comparable,
            win: a.win,
            tie: a.tie,
            loss: a.loss,
            win_rate: a.win_rate(),
            mean_delta_bps: a.mean_delta(),
            median_delta_bps: a.median_delta(),
            ours_no_route: a.ours_no_route,
            jup_no_route: a.jup_no_route,
            both_no_route: a.both_no_route,
            errors: a.errors,
        }
    }
}

#[derive(Serialize)]
struct MatrixEntry {
    liquidity_tier: String,
    usd_size: f64,
    win: usize,
    tie: usize,
    loss: usize,
    other: usize,
}

#[derive(Serialize)]
struct RouteAgreement {
    n: usize,
    mean_jaccard: Option<f64>,
    median_jaccard: Option<f64>,
    identical_pct: Option<f64>,
}

#[derive(Serialize)]
struct Summary {
    run_file: String,
    generated_ts: u64,
    total_cases: usize,
    tie_band_bps: f64,
    vs_jupiter_restricted: ColumnSummary,
    vs_jupiter_unrestricted: ColumnSummary,
    matrix_vs_restricted: Vec<MatrixEntry>,
    latency_ms: BTreeMap<String, LatencyStats>,
    route_agreement_vs_restricted: RouteAgreement,
}

/// Full `report` command: print tables, write summary.csv + summary.json
/// next to the run file.
pub fn run_report(run_path: &Path) -> Result<(), GenericError> {
    let records = load_records(run_path)?;
    if records.is_empty() {
        return Err(format!("no records in '{}'", run_path.display()).into());
    }

    // -- Aggregate ----------------------------------------------------------
    let mut agg_restricted = Agg::default();
    let mut agg_unrestricted = Agg::default();
    for rec in &records {
        agg_restricted.add(&rec.ours, &rec.jup_restricted);
        agg_unrestricted.add(&rec.ours, &rec.jup_unrestricted);
    }

    // Matrix: tier x usd size (vs restricted).
    let mut tiers: Vec<String> = Vec::new();
    let mut sizes: Vec<(u64, f64)> = Vec::new();
    let mut matrix: BTreeMap<(String, u64), MatrixCell> = BTreeMap::new();
    for rec in &records {
        let tier = rec.case.liquidity_tier.clone();
        let bucket = size_bucket(rec.case.usd_size);
        if !tiers.contains(&tier) {
            tiers.push(tier.clone());
        }
        if !sizes.iter().any(|(b, _)| *b == bucket) {
            sizes.push((bucket, rec.case.usd_size));
        }
        matrix
            .entry((tier, bucket))
            .or_default()
            .add(classify(&rec.ours, &rec.jup_restricted));
    }
    tiers.sort_by_key(|t| match t.as_str() {
        "major" => 0,
        "mid" => 1,
        "tail" => 2,
        _ => 3,
    });
    sizes.sort_by_key(|(b, _)| *b);

    // Latency columns (labeled honestly; cached Jupiter responses have no RTT).
    let mut ours_server: Vec<f64> = Vec::new();
    let mut ours_rtt: Vec<f64> = Vec::new();
    let mut jup_r_rtt: Vec<f64> = Vec::new();
    let mut jup_u_rtt: Vec<f64> = Vec::new();
    let mut jup_r_server: Vec<f64> = Vec::new();
    for rec in &records {
        if let Some(v) = rec.ours.server_ms {
            ours_server.push(v);
        }
        if let Some(v) = rec.ours.rtt_ms {
            ours_rtt.push(v);
        }
        if let Some(v) = rec.jup_restricted.rtt_ms {
            jup_r_rtt.push(v);
        }
        if let Some(v) = rec.jup_unrestricted.rtt_ms {
            jup_u_rtt.push(v);
        }
        if let Some(v) = rec.jup_restricted.server_ms {
            jup_r_server.push(v);
        }
    }

    // Route agreement (vs restricted, both sides OK).
    let mut jaccards: Vec<f64> = Vec::new();
    for rec in &records {
        if rec.ours.status == SideStatus::Ok
            && rec.jup_restricted.status == SideStatus::Ok
            && let Some(j) = jaccard(&rec.ours.pools, &rec.jup_restricted.pools)
        {
            jaccards.push(j);
        }
    }
    jaccards.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let identical = jaccards.iter().filter(|&&j| j >= 1.0).count();
    let agreement = RouteAgreement {
        n: jaccards.len(),
        mean_jaccard: mean(&jaccards),
        median_jaccard: percentile(&jaccards, 50.0),
        identical_pct: if jaccards.is_empty() {
            None
        } else {
            Some(identical as f64 / jaccards.len() as f64 * 100.0)
        },
    };

    // -- Terminal output ----------------------------------------------------
    println!("Benchmark report: {}", run_path.display());
    println!("{} cases\n", records.len());

    println!("== Ours vs Jupiter restricted (dexes = our five) ==");
    print_overall(&agg_restricted);
    println!("\n== Ours vs Jupiter unrestricted (absolute market) ==");
    print_overall(&agg_unrestricted);

    println!("\n== Win/tie/loss matrix vs restricted (rows: liquidity tier, cols: usd size) ==");
    let mut headers: Vec<&str> = vec!["tier"];
    let size_labels: Vec<String> = sizes.iter().map(|(_, usd)| size_label(*usd)).collect();
    headers.extend(size_labels.iter().map(|s| s.as_str()));
    let mut rows = Vec::new();
    for tier in &tiers {
        let mut row = vec![tier.clone()];
        for (bucket, _) in &sizes {
            let cell = matrix.get(&(tier.clone(), *bucket));
            row.push(match cell {
                Some(c) => format!("{}/{}/{}", c.win, c.tie, c.loss),
                None => "-".to_string(),
            });
        }
        rows.push(row);
    }
    println!("{}", render_table(&headers, &rows));
    println!("(cells are win/tie/loss; no-route and error cases excluded)");

    println!("\n== Latency (ms) ==");
    let mut lat_map: BTreeMap<String, LatencyStats> = BTreeMap::new();
    lat_map.insert("ours_server_ms (in-process quote)".into(), latency_stats(&mut ours_server));
    lat_map.insert("ours_rtt_ms (localhost HTTP)".into(), latency_stats(&mut ours_rtt));
    lat_map.insert("jup_restricted_rtt_ms (network HTTP)".into(), latency_stats(&mut jup_r_rtt));
    lat_map.insert("jup_unrestricted_rtt_ms (network HTTP)".into(), latency_stats(&mut jup_u_rtt));
    lat_map.insert("jup_restricted_server_ms (self-reported)".into(), latency_stats(&mut jup_r_server));
    let lat_rows: Vec<Vec<String>> = [
        ("ours server ms (in-process quote)", "ours_server_ms (in-process quote)"),
        ("ours RTT ms (localhost HTTP)", "ours_rtt_ms (localhost HTTP)"),
        ("jupiter restricted RTT ms (network)", "jup_restricted_rtt_ms (network HTTP)"),
        ("jupiter unrestricted RTT ms (network)", "jup_unrestricted_rtt_ms (network HTTP)"),
        ("jupiter server ms (self-reported)", "jup_restricted_server_ms (self-reported)"),
    ]
    .iter()
    .map(|(label, key)| {
        let s = &lat_map[*key];
        vec![
            label.to_string(),
            s.n.to_string(),
            fmt_opt(s.p50, 1),
            fmt_opt(s.p95, 1),
            fmt_opt(s.mean, 1),
        ]
    })
    .collect();
    println!("{}", render_table(&["column", "n", "p50", "p95", "mean"], &lat_rows));
    println!("(these columns measure different things — do not compare across rows blindly)");

    println!("\n== Route agreement vs restricted (Jaccard over pool sets) ==");
    println!(
        "  n={}  mean={}  median={}  identical={}",
        agreement.n,
        fmt_opt(agreement.mean_jaccard, 3),
        fmt_opt(agreement.median_jaccard, 3),
        agreement
            .identical_pct
            .map(|p| format!("{p:.1}%"))
            .unwrap_or_else(|| "-".into()),
    );

    // -- Artifacts ----------------------------------------------------------
    let dir = run_path.parent().unwrap_or_else(|| Path::new("."));
    let json_path = dir.join("summary.json");
    let csv_path = dir.join("summary.csv");

    let summary = Summary {
        run_file: run_path.display().to_string(),
        generated_ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        total_cases: records.len(),
        tie_band_bps: crate::metrics::TIE_BAND_BPS,
        vs_jupiter_restricted: (&agg_restricted).into(),
        vs_jupiter_unrestricted: (&agg_unrestricted).into(),
        matrix_vs_restricted: tiers
            .iter()
            .flat_map(|tier| {
                sizes.iter().filter_map(|(bucket, usd)| {
                    matrix.get(&(tier.clone(), *bucket)).map(|c| MatrixEntry {
                        liquidity_tier: tier.clone(),
                        usd_size: *usd,
                        win: c.win,
                        tie: c.tie,
                        loss: c.loss,
                        other: c.other,
                    })
                })
            })
            .collect(),
        latency_ms: lat_map,
        route_agreement_vs_restricted: agreement,
    };
    std::fs::write(&json_path, serde_json::to_string_pretty(&summary)? + "\n")?;
    write_csv(&csv_path, &records)?;
    println!("\nwrote {}", json_path.display());
    println!("wrote {}", csv_path.display());
    Ok(())
}

fn print_overall(agg: &Agg) {
    let pct = |n: usize| {
        if agg.comparable == 0 {
            "-".to_string()
        } else {
            format!("{:.1}%", n as f64 / agg.comparable as f64 * 100.0)
        }
    };
    println!(
        "  comparable: {}   win: {} ({})   tie: {} ({})   loss: {} ({})",
        agg.comparable,
        agg.win,
        pct(agg.win),
        agg.tie,
        pct(agg.tie),
        agg.loss,
        pct(agg.loss),
    );
    println!(
        "  mean delta: {} bps   median delta: {} bps",
        fmt_opt(agg.mean_delta(), 2),
        fmt_opt(agg.median_delta(), 2),
    );
    println!(
        "  ours_no_route: {}   jup_no_route: {}   both_no_route: {}   errors: {}",
        agg.ours_no_route, agg.jup_no_route, agg.both_no_route, agg.errors,
    );
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Per-case detail rows.
fn write_csv(path: &Path, records: &[CaseRecord]) -> Result<(), GenericError> {
    let mut out = String::new();
    out.push_str(
        "id,liquidity_tier,usd_size,input_mint,output_mint,amount,\
         ours_status,ours_out,jup_restricted_status,jup_restricted_out,\
         jup_unrestricted_status,jup_unrestricted_out,\
         delta_bps_vs_restricted,delta_bps_vs_unrestricted,outcome_vs_restricted,\
         ours_server_ms,ours_rtt_ms,jup_restricted_rtt_ms,jup_unrestricted_rtt_ms,\
         jaccard_vs_restricted\n",
    );
    for rec in records {
        let status_str = |s: SideStatus| match s {
            SideStatus::Ok => "ok",
            SideStatus::NoRoute => "no_route",
            SideStatus::Error => "error",
        };
        let d_r = match (rec.ours.out_amount, rec.jup_restricted.out_amount) {
            (Some(a), Some(b)) if rec.ours.status == SideStatus::Ok
                && rec.jup_restricted.status == SideStatus::Ok =>
            {
                Some(delta_bps(a, b))
            }
            _ => None,
        };
        let d_u = match (rec.ours.out_amount, rec.jup_unrestricted.out_amount) {
            (Some(a), Some(b)) if rec.ours.status == SideStatus::Ok
                && rec.jup_unrestricted.status == SideStatus::Ok =>
            {
                Some(delta_bps(a, b))
            }
            _ => None,
        };
        let j = if rec.ours.status == SideStatus::Ok && rec.jup_restricted.status == SideStatus::Ok
        {
            jaccard(&rec.ours.pools, &rec.jup_restricted.pools)
        } else {
            None
        };
        let opt_u64 = |v: Option<u64>| v.map(|x| x.to_string()).unwrap_or_default();
        let opt_f = |v: Option<f64>| v.map(|x| format!("{x:.3}")).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&rec.case.id),
            rec.case.liquidity_tier,
            rec.case.usd_size,
            rec.case.input_mint,
            rec.case.output_mint,
            rec.case.amount,
            status_str(rec.ours.status),
            opt_u64(rec.ours.out_amount),
            status_str(rec.jup_restricted.status),
            opt_u64(rec.jup_restricted.out_amount),
            status_str(rec.jup_unrestricted.status),
            opt_u64(rec.jup_unrestricted.out_amount),
            opt_f(d_r),
            opt_f(d_u),
            classify(&rec.ours, &rec.jup_restricted).label(),
            opt_f(rec.ours.server_ms),
            opt_f(rec.ours.rtt_ms),
            opt_f(rec.jup_restricted.rtt_ms),
            opt_f(rec.jup_unrestricted.rtt_ms),
            opt_f(j),
        ));
    }
    std::fs::write(path, out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// diff: regression mode between two runs of OURS
// ---------------------------------------------------------------------------

/// Compare our out_amounts between two runs, joined on case id.
/// Returns process exit code: 1 if any case regresses more than
/// `threshold_bps`, else 0.
pub fn run_diff(path_a: &Path, path_b: &Path, threshold_bps: f64) -> Result<i32, GenericError> {
    let a = load_records(path_a)?;
    let b = load_records(path_b)?;
    let map_a: BTreeMap<&str, &CaseRecord> = a.iter().map(|r| (r.case.id.as_str(), r)).collect();
    let map_b: BTreeMap<&str, &CaseRecord> = b.iter().map(|r| (r.case.id.as_str(), r)).collect();

    let mut regressions: Vec<(String, String, String, f64)> = Vec::new();
    let mut improvements = 0usize;
    let mut unchanged = 0usize;
    let mut deltas: Vec<f64> = Vec::new();
    let mut only_a = 0usize;
    let mut only_b = 0usize;

    for (id, ra) in &map_a {
        let Some(rb) = map_b.get(id) else {
            only_a += 1;
            continue;
        };
        let a_ok = ra.ours.status == SideStatus::Ok;
        let b_ok = rb.ours.status == SideStatus::Ok;
        match (a_ok, b_ok) {
            (true, true) => {
                let (Some(oa), Some(ob)) = (ra.ours.out_amount, rb.ours.out_amount) else {
                    continue;
                };
                let d = delta_bps(ob, oa); // positive = run B improved
                deltas.push(d);
                if d < -threshold_bps {
                    regressions.push((id.to_string(), oa.to_string(), ob.to_string(), d));
                } else if d > threshold_bps {
                    improvements += 1;
                } else {
                    unchanged += 1;
                }
            }
            (true, false) => {
                // Route existed in A, lost in B: always a regression.
                regressions.push((
                    id.to_string(),
                    ra.ours.out_amount.map(|v| v.to_string()).unwrap_or_default(),
                    format!("({})", rb.ours.status_label()),
                    f64::NEG_INFINITY,
                ));
            }
            (false, true) => improvements += 1,
            (false, false) => unchanged += 1,
        }
    }
    for id in map_b.keys() {
        if !map_a.contains_key(id) {
            only_b += 1;
        }
    }

    println!("diff (OUR out_amounts): A = {}, B = {}", path_a.display(), path_b.display());
    println!(
        "joined: {}   only-in-A: {}   only-in-B: {}   threshold: {} bps",
        map_a.len() - only_a,
        only_a,
        only_b,
        threshold_bps,
    );
    println!(
        "improved: {}   unchanged (within threshold): {}   regressed: {}",
        improvements,
        unchanged,
        regressions.len(),
    );
    if let Some(m) = mean(&deltas) {
        println!("mean delta (B vs A): {m:.2} bps");
    }

    if !regressions.is_empty() {
        regressions.sort_by(|x, y| x.3.partial_cmp(&y.3).unwrap_or(std::cmp::Ordering::Equal));
        let rows: Vec<Vec<String>> = regressions
            .iter()
            .map(|(id, oa, ob, d)| {
                vec![
                    id.clone(),
                    oa.clone(),
                    ob.clone(),
                    if d.is_finite() { format!("{d:.2}") } else { "route lost".into() },
                ]
            })
            .collect();
        println!("\nregressions:");
        println!("{}", render_table(&["case", "out_A", "out_B", "delta_bps"], &rows));
        return Ok(1);
    }
    println!("no regressions beyond {threshold_bps} bps");
    Ok(0)
}

impl crate::metrics::SideResult {
    fn status_label(&self) -> &'static str {
        match self.status {
            SideStatus::Ok => "ok",
            SideStatus::NoRoute => "no_route",
            SideStatus::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cases::BenchCase;
    use crate::metrics::SideResult;

    fn record(id: &str, out: Option<u64>) -> CaseRecord {
        let ours = match out {
            Some(v) => SideResult {
                status: SideStatus::Ok,
                out_amount: Some(v),
                server_ms: Some(1.0),
                rtt_ms: Some(2.0),
                pools: vec!["p".into()],
                slot_hint: None,
                labels: Vec::new(),
                error: None,
            },
            None => SideResult::no_route(None, Some(2.0)),
        };
        CaseRecord {
            case: BenchCase {
                id: id.into(),
                input_mint: "in".into(),
                output_mint: "out".into(),
                amount: 1,
                usd_size: 100.0,
                liquidity_tier: "major".into(),
                slippage_bps: 50,
            },
            ours: ours.clone(),
            jup_restricted: ours.clone(),
            jup_unrestricted: ours,
            ts: 0,
        }
    }

    fn write_run(path: &Path, records: &[CaseRecord]) {
        let mut s = String::new();
        for r in records {
            s.push_str(&serde_json::to_string(r).unwrap());
            s.push('\n');
        }
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn table_renders_aligned() {
        let t = render_table(
            &["tier", "$1", "$100"],
            &[
                vec!["major".into(), "1/2/3".into(), "10/0/1".into()],
                vec!["tail".into(), "-".into(), "0/0/12".into()],
            ],
        );
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 4);
        // All rows align to the same width.
        assert!(lines[0].starts_with("tier"));
        assert!(lines[2].starts_with("major"));
        assert!(lines[3].starts_with("tail"));
    }

    #[test]
    fn diff_detects_regression_and_exit_code() {
        let dir = std::env::temp_dir().join("thunder-bench-test-diff");
        std::fs::create_dir_all(&dir).unwrap();
        let pa = dir.join("a.jsonl");
        let pb = dir.join("b.jsonl");

        // B regresses case-2 by 100 bps and loses case-3's route.
        write_run(&pa, &[record("c1", Some(10_000)), record("c2", Some(10_000)), record("c3", Some(10_000))]);
        write_run(&pb, &[record("c1", Some(10_000)), record("c2", Some(9_900)), record("c3", None)]);

        assert_eq!(run_diff(&pa, &pb, 5.0).unwrap(), 1);
        // Same run against itself: zero regressions.
        assert_eq!(run_diff(&pa, &pa, 5.0).unwrap(), 0);
        // Huge threshold forgives the 100 bps drop, but a lost route is
        // always a regression.
        assert_eq!(run_diff(&pa, &pb, 10_000.0).unwrap(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_within_threshold_passes() {
        let dir = std::env::temp_dir().join("thunder-bench-test-diff2");
        std::fs::create_dir_all(&dir).unwrap();
        let pa = dir.join("a.jsonl");
        let pb = dir.join("b.jsonl");
        // 4 bps drop < default 5 bps threshold.
        write_run(&pa, &[record("c1", Some(1_000_000))]);
        write_run(&pb, &[record("c1", Some(999_600))]);
        assert_eq!(run_diff(&pa, &pb, 5.0).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_writes_artifacts() {
        let dir = std::env::temp_dir().join("thunder-bench-test-report");
        std::fs::create_dir_all(&dir).unwrap();
        let run = dir.join("run.jsonl");
        write_run(&run, &[record("c1", Some(10_000)), record("c2", None)]);
        run_report(&run).unwrap();
        assert!(dir.join("summary.json").exists());
        assert!(dir.join("summary.csv").exists());
        let csv = std::fs::read_to_string(dir.join("summary.csv")).unwrap();
        assert!(csv.lines().count() == 3); // header + 2 cases
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
        assert_eq!(json["total_cases"], 2);
        assert_eq!(json["vs_jupiter_restricted"]["tie"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
