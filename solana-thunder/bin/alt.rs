//! thunder-alt: manage the static swap Address Lookup Table.
//!
//! Subcommands:
//!   create --url <RPC_URL> [--allow-mainnet]
//!       Create a new ALT owned by the PRIVATE_KEY wallet and extend it with
//!       the curated swap address list (see crates/engine/src/alt.rs).
//!   extend --url <RPC_URL> --alt <ADDRESS> [--allow-mainnet] [PUBKEY...]
//!       Extend an existing ALT. With explicit PUBKEYs, adds those; without,
//!       adds any curated address missing from the on-chain table.
//!   show --url <RPC_URL> --alt <ADDRESS>
//!       Fetch and print the table contents.
//!
//! `--url` is REQUIRED (no default) so a mainnet endpoint can never be hit
//! by accident; URLs containing "mainnet" are refused without
//! `--allow-mainnet`. Signs with PRIVATE_KEY from .env.

use std::env;
use std::process::exit;
use std::str::FromStr;

use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::{Keypair, Signer};
use thunder_engine::alt;

fn usage() -> ! {
    eprintln!(
        "Usage:\n  \
         thunder-alt create --url <RPC_URL> [--allow-mainnet]\n  \
         thunder-alt extend --url <RPC_URL> --alt <ADDRESS> [--allow-mainnet] [PUBKEY...]\n  \
         thunder-alt show   --url <RPC_URL> --alt <ADDRESS>\n\n\
         --url is required (no default). URLs containing \"mainnet\" are refused\n\
         unless --allow-mainnet is passed. Signs with PRIVATE_KEY from .env."
    );
    exit(2);
}

struct Args {
    command: String,
    url: String,
    alt: Option<Pubkey>,
    allow_mainnet: bool,
    extra: Vec<String>,
}

fn parse_args() -> Args {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else { usage() };
    let mut url: Option<String> = None;
    let mut alt: Option<Pubkey> = None;
    let mut allow_mainnet = false;
    let mut extra: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--alt" => {
                let Some(s) = args.next() else { usage() };
                match Pubkey::from_str(&s) {
                    Ok(pk) => alt = Some(pk),
                    Err(e) => {
                        eprintln!("invalid --alt address {s}: {e}");
                        exit(2);
                    }
                }
            }
            "--allow-mainnet" => allow_mainnet = true,
            other if other.starts_with("--") => {
                eprintln!("unknown flag: {other}");
                usage();
            }
            other => extra.push(other.to_string()),
        }
    }

    let Some(url) = url else {
        eprintln!("--url is required (there is no default, to keep mainnet out of reach)");
        usage();
    };
    Args { command, url, alt, allow_mainnet, extra }
}

fn load_keypair() -> Keypair {
    let Ok(pk) = env::var("PRIVATE_KEY") else {
        eprintln!("PRIVATE_KEY not set (expected base58 keypair in .env)");
        exit(2);
    };
    Keypair::from_base58_string(&pk)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let args = parse_args();

    if args.url.to_ascii_lowercase().contains("mainnet") && !args.allow_mainnet {
        eprintln!(
            "refusing mainnet URL {} without --allow-mainnet\n\
             (this guard exists so ALT creation runs are always deliberate)",
            args.url
        );
        exit(2);
    }

    let rpc = RpcClient::new_with_commitment(args.url.clone(), CommitmentConfig::confirmed());

    match args.command.as_str() {
        "create" => {
            let payer = load_keypair();
            println!("Creating swap ALT");
            println!("  url:       {}", args.url);
            println!("  authority: {}", payer.pubkey());
            let curated = match alt::curated_alt_addresses(&rpc).await {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("failed to assemble curated address list: {e}");
                    exit(1);
                }
            };
            println!("  addresses: {} curated", curated.len());
            match alt::create_swap_alt(&rpc, &payer).await {
                Ok(table) => {
                    println!("\nALT created: {}", table.key);
                    println!("  {} addresses stored", table.addresses.len());
                    println!("\nExport for the engine:");
                    println!("  export SWAP_ALT_ADDRESS={}", table.key);
                    println!("\nMainnet equivalent (run yourself, deliberately):");
                    println!(
                        "  ROUTER_PROGRAM_ID=<mainnet router deploy> \\\n  \
                         cargo run --bin thunder-alt -- create \\\n    \
                         --url https://api.mainnet-beta.solana.com --allow-mainnet"
                    );
                }
                Err(e) => {
                    eprintln!("ALT creation failed: {e}");
                    exit(1);
                }
            }
        }
        "extend" => {
            let Some(alt_address) = args.alt else {
                eprintln!("extend requires --alt <ADDRESS>");
                usage();
            };
            let payer = load_keypair();
            let new_addresses: Vec<Pubkey> = if args.extra.is_empty() {
                // Diff the curated list against the on-chain table.
                let existing = match alt::load_alt(&rpc, &alt_address).await {
                    Ok(t) => t.addresses,
                    Err(e) => {
                        eprintln!("failed to load ALT {alt_address}: {e}");
                        exit(1);
                    }
                };
                let curated = match alt::curated_alt_addresses(&rpc).await {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("failed to assemble curated address list: {e}");
                        exit(1);
                    }
                };
                curated.into_iter().filter(|a| !existing.contains(a)).collect()
            } else {
                args.extra
                    .iter()
                    .map(|s| {
                        Pubkey::from_str(s).unwrap_or_else(|e| {
                            eprintln!("invalid pubkey {s}: {e}");
                            exit(2);
                        })
                    })
                    .collect()
            };
            if new_addresses.is_empty() {
                println!("nothing to extend: table already holds every curated address");
                return;
            }
            println!("Extending {alt_address} with {} addresses", new_addresses.len());
            match alt::extend_alt(&rpc, &payer, &alt_address, &new_addresses).await {
                Ok(()) => println!("done"),
                Err(e) => {
                    eprintln!("extend failed: {e}");
                    exit(1);
                }
            }
        }
        "show" => {
            let Some(alt_address) = args.alt else {
                eprintln!("show requires --alt <ADDRESS>");
                usage();
            };
            match alt::load_alt(&rpc, &alt_address).await {
                Ok(table) => {
                    println!("ALT {} ({} addresses):", table.key, table.addresses.len());
                    for (i, a) in table.addresses.iter().enumerate() {
                        println!("  [{i:>3}] {a}");
                    }
                }
                Err(e) => {
                    eprintln!("failed to load ALT {alt_address}: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
        }
    }
}
