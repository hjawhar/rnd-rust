//! thunder-bench: benchmark harness comparing the local Thunder engine
//! against Jupiter's swap API.
//!
//! Subcommands: gen-cases | run | report | diff
//! (manual std::env::args parsing — this workspace has zero CLI deps)
//!
//! Future (feature = "sim"): a `simulate` subcommand will provide ground
//! truth for divergent cases (|delta| > 20 bps): build both transactions
//! (ours via /swap-instructions once Workstream 1 M3 lands, Jupiter's via
//! their /swap-instructions), snapshot the union of touched accounts at a
//! single slot, execute both in LiteSVM against the identical snapshot, and
//! compare ACTUAL out amounts. Deliberately out of scope for now — this
//! stub comment is the only trace.

mod cases;
mod jupiter;
mod metrics;
mod report;
mod thunder;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thunder_core::GenericError;

use crate::metrics::{CaseRecord, SideStatus};

const USAGE: &str = "\
thunder-bench — benchmark harness vs Jupiter

USAGE:
  thunder-bench gen-cases [--sol-price <f64>] [--tokens-per-tier <n>] [--out <path>] [--slippage-bps <n>]
  thunder-bench run --cases <cases.json> --out <run.jsonl> [--resume] [--max-cache-age <secs>] [--max-hops <n>]
  thunder-bench report <run.jsonl>
  thunder-bench diff <runA.jsonl> <runB.jsonl> [--threshold-bps <f64>]

ENV:
  CACHE_PATH        pool cache for gen-cases (default: pools.cache)
  THUNDER_URL       local engine base URL (default: http://localhost:8080)
  JUPITER_BASE_URL  Jupiter quote API base (default: lite-api, or api.jup.ag with a key;
                    point at a self-hosted jupiter-swap-api for volume runs)
  JUPITER_API_KEY   portal.jup.ag key, sent as x-api-key (switches base to api.jup.ag)
  BENCH_JUP_DEXES   comma-separated Jupiter DEX labels; skips runtime label discovery
  BENCH_JUP_RPS     override Jupiter request pacing (default 0.45 keyless / 0.9 keyed)

NOTES:
  gen-cases loads the full pool cache (~1.6 GB) — build/run with --release.
  gen-cases never fetches from RPC; the cache file must already exist.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match dispatch(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    };
    std::process::exit(code);
}

fn dispatch(args: &[String]) -> Result<i32, GenericError> {
    let Some(cmd) = args.first() else {
        eprint!("{USAGE}");
        return Ok(2);
    };
    match cmd.as_str() {
        "gen-cases" => cmd_gen_cases(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "report" => cmd_report(&args[1..]),
        "diff" => cmd_diff(&args[1..]),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(0)
        }
        other => {
            eprintln!("unknown subcommand: {other}\n");
            eprint!("{USAGE}");
            Ok(2)
        }
    }
}

// ---------------------------------------------------------------------------
// Tiny arg parser
// ---------------------------------------------------------------------------

struct Args {
    positionals: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

/// Split args into positionals and `--flag [value]` pairs.
/// `value_flags` lists flags that consume the next token.
fn parse_args(args: &[String], value_flags: &[&str]) -> Result<Args, GenericError> {
    let mut positionals = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if value_flags.contains(&name) {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("--{name} requires a value"))?;
                flags.push((name.to_string(), Some(value.clone())));
                i += 2;
            } else {
                flags.push((name.to_string(), None));
                i += 1;
            }
        } else {
            positionals.push(a.clone());
            i += 1;
        }
    }
    Ok(Args { positionals, flags })
}

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }

    fn get_parsed<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, GenericError>
    where
        T::Err: std::fmt::Display,
    {
        match self.get(name) {
            Some(raw) => raw
                .parse::<T>()
                .map(Some)
                .map_err(|e| format!("invalid --{name} '{raw}': {e}").into()),
            None => Ok(None),
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, GenericError> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ---------------------------------------------------------------------------
// gen-cases
// ---------------------------------------------------------------------------

fn cmd_gen_cases(args: &[String]) -> Result<i32, GenericError> {
    let args = parse_args(args, &["sol-price", "tokens-per-tier", "out", "slippage-bps"])?;
    let out_path = PathBuf::from(args.get("out").unwrap_or("bench/cases.json"));
    let tokens_per_tier: usize = args.get_parsed("tokens-per-tier")?.unwrap_or(5);
    let slippage_bps: u64 = args.get_parsed("slippage-bps")?.unwrap_or(50);
    let cache_path =
        PathBuf::from(std::env::var("CACHE_PATH").unwrap_or_else(|_| "pools.cache".into()));

    // SOL/USD: prefer --sol-price (no engine needed); fall back to the
    // engine's /price endpoint if it happens to be running.
    let sol_price: f64 = match args.get_parsed::<f64>("sol-price")? {
        Some(p) => p,
        None => {
            eprintln!("--sol-price not given, trying the engine's /price endpoint...");
            let rt = runtime()?;
            rt.block_on(async {
                let client = thunder::ThunderClient::from_env()?;
                client.sol_usd_price().await
            })
            .map_err(|e| {
                format!(
                    "{e}. Pass --sol-price <usd> to generate cases without a running engine"
                )
            })?
        }
    };
    eprintln!("using SOL/USD = {sol_price}");

    let index = cases::load_index(&cache_path)?;
    let generated = cases::generate(&index, sol_price, tokens_per_tier, slippage_bps)?;
    cases::save_cases(&generated, &out_path)?;

    let mut per_tier: std::collections::BTreeMap<&str, usize> = Default::default();
    for c in &generated {
        *per_tier.entry(c.liquidity_tier.as_str()).or_default() += 1;
    }
    println!(
        "wrote {} cases to {} ({})",
        generated.len(),
        out_path.display(),
        per_tier
            .iter()
            .map(|(t, n)| format!("{t}: {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(0)
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn cmd_run(args: &[String]) -> Result<i32, GenericError> {
    let args = parse_args(args, &["cases", "out", "max-cache-age", "max-hops"])?;
    let cases_path = PathBuf::from(args.get("cases").unwrap_or("bench/cases.json"));
    let out_path = PathBuf::from(
        args.get("out")
            .ok_or("run requires --out <run.jsonl>")?,
    );
    let resume = args.has("resume");
    let max_cache_age: u64 = args.get_parsed("max-cache-age")?.unwrap_or(0);
    let max_hops: u32 = args.get_parsed("max-hops")?.unwrap_or(2);

    let all_cases = cases::load_cases(&cases_path)?;

    // --resume: skip case ids already present in the out file.
    let done_ids: std::collections::HashSet<String> = if resume && out_path.exists() {
        report::load_records(&out_path)?
            .into_iter()
            .map(|r| r.case.id)
            .collect()
    } else {
        Default::default()
    };
    let todo: Vec<&cases::BenchCase> =
        all_cases.iter().filter(|c| !done_ids.contains(&c.id)).collect();
    if todo.is_empty() {
        println!("nothing to do: all {} cases already in {}", all_cases.len(), out_path.display());
        return Ok(0);
    }
    if !done_ids.is_empty() {
        println!("resuming: {} of {} cases already done", done_ids.len(), all_cases.len());
    }

    let rt = runtime()?;
    rt.block_on(run_async(&todo, &out_path, max_cache_age, max_hops))
}

async fn run_async(
    todo: &[&cases::BenchCase],
    out_path: &Path,
    max_cache_age: u64,
    max_hops: u32,
) -> Result<i32, GenericError> {
    use std::io::Write;

    let thunder = thunder::ThunderClient::from_env()?;
    let jup_config = jupiter::JupiterConfig::from_env();
    let cache_path = out_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("jupiter_cache.jsonl");
    let jup = jupiter::JupiterClient::new(jup_config, cache_path, max_cache_age)?;
    println!("jupiter base: {} ({} rps)", jup.base(), jup.rps());

    // DEX label mapping: BENCH_JUP_DEXES override, else runtime discovery.
    let dexes = match std::env::var("BENCH_JUP_DEXES").ok().filter(|s| !s.is_empty()) {
        Some(raw) => {
            println!("using BENCH_JUP_DEXES override: {raw}");
            raw
        }
        None => {
            println!("discovering Jupiter DEX labels (one unrestricted 1000-SOL quote)...");
            let available = jup.discover_labels().await?;
            let mapping = jupiter::resolve_dex_labels(&available);
            println!("Jupiter DEX label mapping:");
            let mut resolved = Vec::new();
            for (target, label) in &mapping {
                match label {
                    Some(l) => {
                        println!("  {target:<16} -> {l}");
                        resolved.push(l.clone());
                    }
                    None => println!(
                        "  {target:<16} -> (unresolved; proceeding without — set BENCH_JUP_DEXES to override)"
                    ),
                }
            }
            if resolved.is_empty() {
                return Err(
                    "no Jupiter DEX labels resolved; set BENCH_JUP_DEXES=<label,label,...>".into(),
                );
            }
            resolved.join(",")
        }
    };
    println!("restricted quotes use dexes={dexes}\n");

    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out_path)?;

    let total = todo.len();
    for (i, case) in todo.iter().enumerate() {
        let ours = thunder.quote(case, max_hops).await;
        let jup_restricted = jup
            .quote_side(&case.input_mint, &case.output_mint, case.amount, case.slippage_bps, Some(&dexes))
            .await;
        let jup_unrestricted = jup
            .quote_side(&case.input_mint, &case.output_mint, case.amount, case.slippage_bps, None)
            .await;

        let record = CaseRecord {
            case: (*case).clone(),
            ours,
            jup_restricted,
            jup_unrestricted,
            ts: now_secs(),
        };
        writeln!(out, "{}", serde_json::to_string(&record)?)?;
        out.flush()?;

        let brief = |s: &metrics::SideResult| match s.status {
            SideStatus::Ok => format!("ok({})", s.out_amount.unwrap_or(0)),
            SideStatus::NoRoute => "no_route".to_string(),
            SideStatus::Error => "error".to_string(),
        };
        println!(
            "[{}/{}] {}  ours={}  jupR={}  jupU={}",
            i + 1,
            total,
            record.case.id,
            brief(&record.ours),
            brief(&record.jup_restricted),
            brief(&record.jup_unrestricted),
        );
    }
    println!("\nwrote {total} records to {}", out_path.display());
    Ok(0)
}

// ---------------------------------------------------------------------------
// report / diff
// ---------------------------------------------------------------------------

fn cmd_report(args: &[String]) -> Result<i32, GenericError> {
    let args = parse_args(args, &[])?;
    let [run] = args.positionals.as_slice() else {
        return Err("usage: thunder-bench report <run.jsonl>".into());
    };
    report::run_report(Path::new(run))?;
    Ok(0)
}

fn cmd_diff(args: &[String]) -> Result<i32, GenericError> {
    let args = parse_args(args, &["threshold-bps"])?;
    let [a, b] = args.positionals.as_slice() else {
        return Err("usage: thunder-bench diff <runA.jsonl> <runB.jsonl> [--threshold-bps <f64>]".into());
    };
    let threshold: f64 = args.get_parsed("threshold-bps")?.unwrap_or(5.0);
    report::run_diff(Path::new(a), Path::new(b), threshold)
}
