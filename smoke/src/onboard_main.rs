//! `gib-onboard` — produce the block an indexer pastes, but only once your own side is ready.
//!
//! Doc 04 calls the whitelist handshake the slowest part of standing up an independent gateway.
//! It is slow because the failures are asymmetric: the operator's aggregator works perfectly from
//! the operator's shell, and the indexer finds out it does not work from the internet some hours
//! later, as receipts that bounce with no diagnosis.
//!
//! So this checks the operator's side first and refuses to print the block if it would waste
//! somebody's afternoon.

use anyhow::{Context, Result};
use clap::Parser;
use gib_smoke::{
    config, domain_matches, fetch_domain_info,
    onboard::{
        check_address_drift, check_aggregator_url, may_onboard, Check, IndexerBlock, Outcome,
    },
};
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::str::FromStr;
use thegraph_core::alloy::primitives::Address;

#[derive(Parser, Debug)]
#[command(about = "gib onboard: the artefact an indexer needs, withheld until your side is ready")]
struct Args {
    /// Rendered gateway config to self-configure from.
    #[arg(
        long,
        env = "GIB_GATEWAY_CONFIG",
        default_value = "/config/gateway.json"
    )]
    config: String,
    /// The aggregator URL as an INDEXER would reach it — public DNS, ideally https.
    /// Not the in-stack Compose address `gib smoke` uses.
    #[arg(long, env = "GIB_PUBLIC_AGGREGATOR_URL")]
    aggregator_url: String,
    /// `config/addresses.env` as produced by scripts/fetch-addresses.sh.
    #[arg(
        long,
        env = "GIB_ADDRESSES_ENV",
        default_value = "/config/addresses.env"
    )]
    addresses: String,
}

/// Parses a shell-style `KEY=value` env file, ignoring comments, blanks and `export`.
fn parse_env_file(raw: &str) -> BTreeMap<String, String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("export ").or(Some(l)))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_string(),
                v.trim().trim_matches(['"', '\'']).to_string(),
            )
        })
        .collect()
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("gib-onboard: fatal: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn run() -> Result<bool> {
    let args = Args::parse();
    let zero = Address::from_str("0x0000000000000000000000000000000000000000").unwrap();
    let loaded = config::from_gateway_json(&args.config, args.aggregator_url.clone(), zero, zero)
        .with_context(|| format!("reading {}", args.config))?;
    let cfg = &loaded.config;

    let mut checks: Vec<Check> = check_aggregator_url(&args.aggregator_url);

    // Does the aggregator that an indexer would reach agree with the gateway that signs?
    // A reverse proxy pointed at the wrong port passes every other check and fails every receipt.
    checks.push(match fetch_domain_info(&args.aggregator_url).await {
        Ok(info) if domain_matches(&info, cfg) => Check {
            name: "aggregator agrees with the gateway's EIP-712 domain",
            outcome: Outcome::Pass,
            detail: String::new(),
        },
        Ok(_) => Check {
            name: "aggregator agrees with the gateway's EIP-712 domain",
            outcome: Outcome::Fail,
            detail: format!(
                "the aggregator at {} reports a different chain id or verifier than \
                 gateway.json (chain {}, verifier {}). Your proxy is probably pointed at the \
                 wrong service.",
                args.aggregator_url, cfg.chain_id, cfg.verifier
            ),
        },
        Err(e) => Check {
            name: "aggregator agrees with the gateway's EIP-712 domain",
            outcome: Outcome::Fail,
            detail: format!(
                "could not reach {} from here: {e:#}. If it is unreachable from this box it is \
                 certainly unreachable from an indexer.",
                args.aggregator_url
            ),
        },
    });

    match std::fs::read_to_string(&args.addresses) {
        Ok(raw) => {
            let fetched = parse_env_file(&raw);
            let rendered = BTreeMap::from([
                (
                    "GRAPH_TALLY_COLLECTOR".to_string(),
                    cfg.verifier.to_string(),
                ),
                ("SUBGRAPH_SERVICE".to_string(), cfg.data_service.to_string()),
            ]);
            checks.extend(check_address_drift(&rendered, &fetched));
        }
        Err(e) => checks.push(Check {
            name: "collector + subgraph service match the address book",
            outcome: Outcome::Warn,
            detail: format!(
                "could not read {}: {e}. Skipped the drift check.",
                args.addresses
            ),
        }),
    }

    println!("\n=== gib onboard ===\n");
    for c in &checks {
        let tag = match c.outcome {
            Outcome::Pass => "PASS",
            Outcome::Warn => "WARN",
            Outcome::Fail => "FAIL",
        };
        println!("[{tag}] {}", c.name);
        if !c.detail.is_empty() {
            println!("       {}", c.detail);
        }
    }

    if !may_onboard(&checks) {
        println!(
            "\nRESULT: NOT READY — no block printed.\n\
             Every failure above is yours to fix and is invisible to an indexer until their \
             receipts bounce. Fix them, then ask."
        );
        return Ok(false);
    }

    let block = IndexerBlock {
        sender: cfg.payer.to_string(),
        aggregator_url: args.aggregator_url,
        collector: cfg.verifier.to_string(),
        subgraph_service: cfg.data_service.to_string(),
        chain_id: cfg.chain_id,
    };
    println!(
        "\nRESULT: READY — send this to the indexer:\n\n{}",
        block.render()
    );
    println!(
        "Escrow still has to be funded for each indexer separately (docs/02); a whitelisted \
         sender with an empty escrow returns 402, not data."
    );
    Ok(true)
}
