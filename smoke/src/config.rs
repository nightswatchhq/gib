//! Self-configuration: `gib smoke` reads the deployment's own rendered
//! `runtime/gateway.json` so its checks are guaranteed to run against the exact
//! signer, sender, verifier, data service and API key the gateway is using —
//! there is nothing to keep in sync by hand, and config drift fails loudly.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::str::FromStr;
use thegraph_core::alloy::primitives::{Address, U256};

use crate::Config;

#[derive(Deserialize)]
struct Receipts {
    chain_id: serde_json::Value, // may be number or "0x.."/decimal string
    payer: String,
    signer: String, // private key (0x…) — the file is mode 600 on the box
    verifier: String,
}

#[derive(Deserialize)]
struct ApiKey {
    key: String,
}

#[derive(Deserialize)]
struct TrustedIndexer {
    url: String,
    #[serde(default)]
    auth: String,
}

#[derive(Deserialize)]
struct GatewayJson {
    receipts: Receipts,
    subgraph_service: String,
    api_keys: Vec<ApiKey>,
    trusted_indexers: Vec<TrustedIndexer>,
}

/// Everything smoke pulls out of gateway.json.
pub struct Loaded {
    pub config: Config,
    pub signer_key: String,
    pub api_key: String,
    /// Network-subgraph (topology) source URL + auth, as the gateway uses it.
    pub topology_url: String,
    pub topology_auth: String,
}

fn parse_chain_id(v: &serde_json::Value) -> Result<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| anyhow!("chain_id not u64")),
        serde_json::Value::String(s) => {
            let s = s.trim();
            if let Some(hex) = s.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).context("chain_id hex")
            } else {
                s.parse::<u64>().context("chain_id dec")
            }
        }
        _ => Err(anyhow!("chain_id wrong type")),
    }
}

/// Load from a rendered gateway.json. `allocation` and `indexer` are proof
/// stand-ins (any valid address; the aggregator doesn't check they're on-chain).
pub fn from_gateway_json(
    path: &str,
    aggregator_url: String,
    allocation: Address,
    service_provider: Address,
) -> Result<Loaded> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let g: GatewayJson = serde_json::from_str(&raw).context("parsing gateway.json")?;
    let chain_id = parse_chain_id(&g.receipts.chain_id)?;
    let cfg = Config {
        chain_id,
        verifier: Address::from_str(&g.receipts.verifier).context("verifier")?,
        payer: Address::from_str(&g.receipts.payer).context("payer")?,
        data_service: Address::from_str(&g.subgraph_service).context("subgraph_service")?,
        service_provider,
        allocation,
        aggregator_url,
    };
    let api_key = g
        .api_keys
        .first()
        .map(|k| k.key.clone())
        .ok_or_else(|| anyhow!("gateway.json has no api_keys"))?;
    let ti = g
        .trusted_indexers
        .first()
        .ok_or_else(|| anyhow!("gateway.json has no trusted_indexers"))?;
    Ok(Loaded {
        config: cfg,
        signer_key: g.receipts.signer.trim().to_string(),
        api_key,
        topology_url: ti.url.clone(),
        topology_auth: ti.auth.clone(),
    })
}

// Re-export for callers that only need the U256 form of chain id.
pub fn chain_u256(chain_id: u64) -> U256 {
    U256::from(chain_id)
}
