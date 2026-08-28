//! `gib-smoke` — the self-test every operator runs before asking an indexer to
//! whitelist them. One command against a running gib deployment; nothing
//! on-chain; keys unfunded; loopback/in-network only.
//!
//! It composes the two halves that each cover the other's gap:
//!   • the RUNNING gateway signs correctly — proven here by observing a
//!     `gateway_queries` record whose signer is the configured signer, from a
//!     real dispatched query (checks b, c). This is the local stand-in for
//!     Phase 1's "real indexers recovered our signer" evidence.
//!   • those signatures aggregate into a verifiable RAV — proven by minting
//!     receipts through the gateway's identical signing path and aggregating
//!     them against the live tap-aggregator (checks d, e, f).
//! Neither half alone is convincing; together they are. The mint step is not
//! circular precisely because check (c) independently confirms the running
//! gateway uses the same signer.

use anyhow::Result;
use clap::Parser;
use gib_smoke::{
    aggregate, config, dispatch_query, domain_matches, fetch_domain_info, fetch_topology_counts,
    kafka, mint_receipt, verify_rav, Config,
};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;
use thegraph_core::alloy::{primitives::Address, signers::local::PrivateKeySigner};

#[derive(Parser, Debug)]
#[command(about = "gib smoke: end-to-end payment-path self-test for a running gib deployment")]
struct Args {
    /// Rendered gateway config to self-configure from.
    #[arg(
        long,
        env = "GIB_GATEWAY_CONFIG",
        default_value = "/config/gateway.json"
    )]
    config: String,
    #[arg(
        long,
        env = "GIB_GATEWAY_URL",
        default_value = "http://gib-gateway:7700"
    )]
    gateway_url: String,
    #[arg(
        long,
        env = "GIB_AGGREGATOR_URL",
        default_value = "http://gib-tap-aggregator:7610"
    )]
    aggregator_url: String,
    #[arg(
        long,
        env = "GIB_PANDAPROXY_URL",
        default_value = "http://gib-redpanda:8082"
    )]
    pandaproxy_url: String,
    /// A well-indexed subgraph id to probe (default: Uniswap V3 Arbitrum).
    #[arg(
        long,
        env = "GIB_TEST_SUBGRAPH",
        default_value = "FQ6JYszEKApsBpAmiHesRsd9Ygc6mzmpNRANeVQFYoVX"
    )]
    subgraph: String,
    #[arg(
        long,
        env = "GIB_ALLOCATION",
        default_value = "0xc87271758174c82e232f966bfe56c2e4615ebea7"
    )]
    allocation: String,
    #[arg(
        long,
        env = "GIB_INDEXER",
        default_value = "0xf92f430dd8567b0d466358c79594ab58d919a6d4"
    )]
    indexer: String,
}

struct Row {
    id: char,
    name: &'static str,
    pass: bool,
    detail: String,
}
impl Row {
    fn ok(id: char, name: &'static str, detail: impl Into<String>) -> Self {
        Row {
            id,
            name,
            pass: true,
            detail: detail.into(),
        }
    }
    fn fail(id: char, name: &'static str, detail: impl Into<String>) -> Self {
        Row {
            id,
            name,
            pass: false,
            detail: detail.into(),
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).chars().take(140).collect()
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("gib-smoke: fatal: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<bool> {
    let args = Args::parse();
    let loaded = config::from_gateway_json(
        &args.config,
        args.aggregator_url.clone(),
        Address::from_str(&args.allocation)?,
        Address::from_str(&args.indexer)?,
    )?;
    let cfg: Config = loaded.config.clone();
    let signer = PrivateKeySigner::from_str(loaded.signer_key.trim())?;
    let signer_addr = signer.address();
    println!(
        "gib-smoke: gateway={} aggregator={} signer={}",
        args.gateway_url, args.aggregator_url, signer_addr
    );

    let mut rows: Vec<Row> = Vec::new();

    // (a) topology sync -------------------------------------------------------
    match fetch_topology_counts(&loaded.topology_url, &loaded.topology_auth).await {
        Ok((indexers, subgraphs)) => {
            let sane = (10..=2000).contains(&indexers) && (500..=500_000).contains(&subgraphs);
            rows.push(Row {
                id: 'a',
                name: "topology sync (counts within sane bounds)",
                pass: sane,
                detail: format!("indexers={indexers} subgraphs={subgraphs}"),
            });
        }
        Err(e) => rows.push(Row::fail(
            'a',
            "topology sync (counts within sane bounds)",
            first_line(&e.to_string()),
        )),
    }

    // (b,c) dispatch a query, then read the gateway_queries record it produced -
    let baseline = kafka::latest_client_query(&args.pandaproxy_url)
        .await
        .ok()
        .flatten()
        .map(|(o, _)| o)
        .unwrap_or(-1);
    let dispatched = dispatch_query(&args.gateway_url, &loaded.api_key, &args.subgraph).await;
    if let Err(e) = &dispatched {
        rows.push(Row::fail(
            'b',
            "query dispatched (candidates + receipts)",
            first_line(&e.to_string()),
        ));
    }
    // Poll for the new record.
    let mut record = None;
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(800)).await;
        if let Ok(Some((off, cq))) = kafka::latest_client_query(&args.pandaproxy_url).await {
            if off > baseline {
                record = Some(cq);
                break;
            }
        }
    }
    match &record {
        Some(cq) => {
            let n = cq.indexer_queries.len();
            rows.push(Row {
                id: 'b',
                name: "query dispatched (candidates + receipts)",
                pass: n >= 1,
                detail: format!("{n} indexer candidate(s) with receipts (fees) recorded"),
            });
            match cq.signer() {
                Some(rs) => rows.push(Row {
                    id: 'c',
                    name: "runtime signer == configured signer",
                    pass: rs == signer_addr,
                    detail: format!("gateway_queries.receipt_signer={rs}"),
                }),
                None => rows.push(Row::fail(
                    'c',
                    "runtime signer == configured signer",
                    "no signer in record",
                )),
            }
        }
        None => {
            rows.push(Row::fail(
                'b',
                "query dispatched (candidates + receipts)",
                "no new gateway_queries record observed",
            ));
            rows.push(Row::fail(
                'c',
                "runtime signer == configured signer",
                "no record to check",
            ));
        }
    }

    // (d,e) mint -> aggregate -> verify RAV + field assertions ----------------
    let values: [u128; 3] = [100, 200, 300];
    let expected_sum: u128 = values.iter().sum();
    let mut receipts = Vec::new();
    for v in values {
        receipts.push(mint_receipt(&cfg, &signer, v)?);
    }
    let agg_domain_info = fetch_domain_info(&cfg.aggregator_url)
        .await
        .unwrap_or(serde_json::Value::Null);
    let domain_ok = domain_matches(&agg_domain_info, &cfg);
    match aggregate(&cfg.aggregator_url, &receipts).await {
        Ok(rav) => {
            let c = verify_rav(&cfg, &rav, signer_addr, expected_sum, domain_ok)?;
            rows.push(Row {
                id: 'd',
                name: "RAV signature recovers to signer",
                pass: c.signer_ok,
                detail: format!("recovered {}", c.recovered_signer),
            });
            rows.push(Row {
                id: 'd',
                name: "RAV EIP-712 domain matches",
                pass: c.domain_ok,
                detail: format!("chain={} verifier={}", cfg.chain_id, cfg.verifier),
            });
            rows.push(Row {
                id: 'd',
                name: "RAV aggregate == sum of receipts",
                pass: c.value_ok,
                detail: format!(
                    "valueAggregate={} expected={}",
                    c.value_aggregate, c.expected_sum
                ),
            });
            rows.push(Row {
                id: 'e',
                name: "RAV.payer == configured sender",
                pass: c.payer_ok,
                detail: format!("payer={} sender={}", rav.message.payer, cfg.payer),
            });
            rows.push(Row {
                id: 'e',
                name: "RAV.dataService == SubgraphService",
                pass: c.data_service_ok,
                detail: format!(
                    "dataService={} expected={}",
                    rav.message.dataService, cfg.data_service
                ),
            });
            println!(
                "\n--- RAV (evidence) ---\n{}",
                serde_json::to_string_pretty(&rav)?
            );
        }
        Err(e) => {
            for id in ['d', 'e'] {
                rows.push(Row::fail(
                    id,
                    "mint -> aggregate -> verify RAV",
                    first_line(&e.to_string()),
                ));
            }
        }
    }

    // (f) negative tests ------------------------------------------------------
    {
        let mut bad = mint_receipt(&cfg, &signer, 100)?;
        bad.message.value = 999_999;
        let rejected = aggregate(&cfg.aggregator_url, std::slice::from_ref(&bad)).await;
        rows.push(Row {
            id: 'f',
            name: "NEG: tampered value rejected",
            pass: rejected.is_err(),
            detail: match &rejected {
                Err(e) => format!("rejected: {}", first_line(&e.to_string())),
                Ok(_) => "ACCEPTED (bad!)".into(),
            },
        });
    }
    {
        let wrong = PrivateKeySigner::from_str(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        )?;
        let bad = mint_receipt(&cfg, &wrong, 100)?;
        let rejected = aggregate(&cfg.aggregator_url, std::slice::from_ref(&bad)).await;
        rows.push(Row {
            id: 'f',
            name: "NEG: wrong-key receipt rejected",
            pass: rejected.is_err(),
            detail: match &rejected {
                Err(e) => format!("rejected: {}", first_line(&e.to_string())),
                Ok(_) => "ACCEPTED (bad!)".into(),
            },
        });
    }

    // Table -------------------------------------------------------------------
    println!("\n=== gib smoke ===");
    let mut all = true;
    for r in &rows {
        println!(
            "({}) [{}] {:<44} {}",
            r.id,
            if r.pass { "PASS" } else { "FAIL" },
            r.name,
            r.detail
        );
        all &= r.pass;
    }
    println!("\nboundary: proves discovery, running-gateway signing, and receipt->RAV aggregation");
    println!("          up to a verified signed RAV. On-chain RAV redemption is the collector");
    println!("          contract's job, not gib's — deliberately untouched.");
    println!("\n{}", if all { "RESULT: PASS" } else { "RESULT: FAIL" });
    Ok(all)
}
