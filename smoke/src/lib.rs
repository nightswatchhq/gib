//! gib-smoke — closes the TAP receipt→RAV payment loop against the local
//! tap-aggregator, with zero indexers and nothing on-chain.
//!
//! What this proves (and what it deliberately does NOT):
//!   • receipt signing uses the gateway's exact EIP-712 path (tap_graph v2)
//!   • the aggregator accepts our signer, matches the EIP-712 domain, and
//!     returns a RAV whose signature recovers to our signer and whose
//!     aggregate value equals the sum of the input receipts
//!   • tampered / wrong-key receipts are REJECTED by the aggregator
//! It does NOT touch the chain: on-chain RAV redemption is the collector
//! contract's job, not gib's. This harness stops at a verified signed RAV.
//!
//! Note on where receipts come from: the gateway does NOT persist signed
//! receipts to Kafka — `gateway_queries` carries only per-indexer fee metadata
//! (ClientQueryProtobuf), used by the escrow-manager for debt tracking. The
//! signed receipts are ephemeral (sent to indexers in the `tap-receipt`
//! header). So this harness MINTS receipts with the gateway's identical
//! `tap_graph::v2` signing path + the deployment's signer key — byte-identical
//! to what the gateway emits — rather than scavenging a topic that has none.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub mod config;
pub mod kafka;
use thegraph_core::alloy::{
    dyn_abi::Eip712Domain,
    primitives::{Address, FixedBytes, U256},
    signers::local::PrivateKeySigner,
    sol,
};

pub use tap_eip712_message::Eip712SignedMessage;
pub use tap_graph::v2::{Receipt, SignedReceipt};

/// The Receipt Aggregate Voucher the aggregator returns, defined exactly as the
/// graph-tally aggregator source (camelCase sol fields). Defining it here keeps
/// verification independent of any single tap-crate's RAV re-exports.
sol! {
    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct ReceiptAggregateVoucher {
        bytes32 collectionId;
        address payer;
        address serviceProvider;
        address dataService;
        uint64 timestampNs;
        uint128 valueAggregate;
        bytes metadata;
    }
}

pub type SignedRav = Eip712SignedMessage<ReceiptAggregateVoucher>;

/// Everything needed to mint receipts the local aggregator will accept.
#[derive(Clone, Debug)]
pub struct Config {
    pub chain_id: u64,
    /// GraphTallyCollector address — the EIP-712 verifying contract.
    pub verifier: Address,
    /// TAP sender (payer) address.
    pub payer: Address,
    /// SubgraphService address (the data service).
    pub data_service: Address,
    /// Indexer (service provider) the receipts are addressed to.
    pub service_provider: Address,
    /// 20-byte allocation, zero-padded into the 32-byte collection id.
    pub allocation: Address,
    /// http(s) URL of the tap-aggregator JSON-RPC endpoint.
    pub aggregator_url: String,
}

/// Build the exact EIP-712 domain the gateway and aggregator use:
/// name="GraphTallyCollector", version="1", chainId, verifyingContract.
pub fn domain(chain_id: u64, verifier: Address) -> Eip712Domain {
    Eip712Domain {
        name: Some("GraphTallyCollector".into()),
        version: Some("1".into()),
        chain_id: Some(U256::from(chain_id)),
        verifying_contract: Some(verifier),
        salt: None,
    }
}

/// collection id = 20-byte allocation, left-zero-padded to 32 bytes (Subgraph
/// Service convention — see gateway receipts.rs).
pub fn collection_id(allocation: Address) -> FixedBytes<32> {
    FixedBytes::<32>::left_padding_from(allocation.as_slice())
}

/// Mint a signed TAP v2 receipt with the given signer — the gateway's exact path.
pub fn mint_receipt(cfg: &Config, signer: &PrivateKeySigner, value: u128) -> Result<SignedReceipt> {
    let receipt = Receipt::new(
        collection_id(cfg.allocation),
        cfg.payer,
        cfg.data_service,
        cfg.service_provider,
        value,
    )
    .map_err(|e| anyhow!("receipt build: {e:?}"))?;
    Eip712SignedMessage::new(&domain(cfg.chain_id, cfg.verifier), receipt, signer)
        .map_err(|e| anyhow!("receipt sign: {e:?}"))
}

/// Submit receipts to the aggregator's `aggregate_receipts` JSON-RPC method,
/// exactly as an indexer's tap-agent would: params = [api_version, receipts,
/// previous_rav]. Returns Ok(rav) on success, or Err with the aggregator's
/// error message on rejection.
pub async fn aggregate(url: &str, receipts: &[SignedReceipt]) -> Result<SignedRav> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "aggregate_receipts",
        "params": ["0.0", receipts, serde_json::Value::Null],
    });
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(url)
        .json(&body)
        .send()
        .await
        .context("aggregator request failed")?
        .json()
        .await
        .context("aggregator response not JSON")?;

    if let Some(err) = resp.get("error") {
        bail!("aggregator rejected: {err}");
    }
    let data = resp
        .get("result")
        .and_then(|r| r.get("data"))
        .ok_or_else(|| anyhow!("no result.data in aggregator response: {resp}"))?;
    serde_json::from_value(data.clone()).context("could not decode RAV")
}

/// Fetch the aggregator's advertised EIP-712 domain (eip712domain_info) as raw
/// JSON — alloy's Eip712Domain isn't Deserialize, so we compare fields directly.
pub async fn fetch_domain_info(url: &str) -> Result<serde_json::Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "eip712domain_info", "params": [],
    });
    let resp: serde_json::Value = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    Ok(resp
        .get("result")
        .and_then(|r| r.get("data"))
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

/// True if the aggregator's advertised domain matches our chain + verifier.
/// `chainId` may serialize as a number or a (hex/dec) string; be tolerant.
pub fn domain_matches(info: &serde_json::Value, cfg: &Config) -> bool {
    let verifier_ok = info
        .get("verifyingContract")
        .and_then(|v| v.as_str())
        .and_then(|s| Address::from_str(s).ok())
        .map(|a| a == cfg.verifier)
        .unwrap_or(false);
    let chain_ok = match info.get("chainId") {
        Some(serde_json::Value::Number(n)) => n.as_u64() == Some(cfg.chain_id),
        Some(serde_json::Value::String(s)) => {
            let s = s.trim_start_matches("0x");
            u64::from_str_radix(s, 16).ok() == Some(cfg.chain_id)
                || s.parse::<u64>().ok() == Some(cfg.chain_id)
        }
        _ => false,
    };
    verifier_ok && chain_ok
}

/// Result of verifying a returned RAV against expectations.
#[derive(Debug)]
pub struct RavChecks {
    pub recovered_signer: Address,
    pub signer_ok: bool,
    pub domain_ok: bool,
    pub value_aggregate: u128,
    pub expected_sum: u128,
    pub value_ok: bool,
    /// RAV.payer == configured sender (config-drift guard).
    pub payer_ok: bool,
    /// RAV.dataService == configured SubgraphService (config-drift guard).
    pub data_service_ok: bool,
}

impl RavChecks {
    pub fn all_ok(&self) -> bool {
        self.signer_ok && self.domain_ok && self.value_ok && self.payer_ok && self.data_service_ok
    }
}

/// Query the network-subgraph topology source (the gateway's trusted-indexer
/// URL) for indexer/subgraph counts. The source returns the indexer-service
/// envelope `{graphQLResponse, attestation}`; we unwrap `graphQLResponse` (a
/// JSON string) and read `graphNetwork`.
pub async fn fetch_topology_counts(url: &str, auth: &str) -> Result<(u64, u64)> {
    let query = serde_json::json!({
        "query": "{ graphNetwork(id:\"1\"){ indexerCount subgraphCount } }"
    });
    let mut req = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/125.0 Safari/537.36",
        )
        .body(query.to_string());
    if !auth.is_empty() {
        req = req.bearer_auth(auth);
    }
    let outer: serde_json::Value = req.send().await?.json().await.context("topology response")?;
    // Unwrap the indexer-service envelope if present; else accept raw {data}.
    let inner_str = outer.get("graphQLResponse").and_then(|v| v.as_str());
    let inner: serde_json::Value = match inner_str {
        Some(s) => serde_json::from_str(s).context("graphQLResponse decode")?,
        None => outer,
    };
    let gn = &inner["data"]["graphNetwork"];
    let indexers = gn["indexerCount"].as_u64().or_else(|| gn["indexerCount"].as_str().and_then(|s| s.parse().ok()));
    let subgraphs = gn["subgraphCount"].as_u64().or_else(|| gn["subgraphCount"].as_str().and_then(|s| s.parse().ok()));
    match (indexers, subgraphs) {
        (Some(i), Some(s)) => Ok((i, s)),
        _ => bail!("could not read graphNetwork counts from topology source: {inner}"),
    }
}

/// Dispatch a real query through the running gateway (like a client would).
/// Returns the response body. Auth passing here is what causes the gateway to
/// emit a `gateway_queries` record; indexer 402s (unfunded) are expected and do
/// not prevent the record.
pub async fn dispatch_query(gateway_url: &str, api_key: &str, subgraph_id: &str) -> Result<String> {
    let url = format!(
        "{}/api/subgraphs/id/{}",
        gateway_url.trim_end_matches('/'),
        subgraph_id
    );
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .body(r#"{"query":"{ _meta { block { number } } }"}"#)
        .send()
        .await
        .context("gateway query failed")?;
    Ok(resp.text().await.unwrap_or_default())
}

/// Verify a RAV: signature recovers to `expected_signer`, its aggregate equals
/// `expected_sum`. `domain_ok` is the caller's domain-match result (see
/// [`domain_matches`]). Recovery itself uses our domain, so a passing signer
/// check already implies domain agreement.
pub fn verify_rav(
    cfg: &Config,
    rav: &SignedRav,
    expected_signer: Address,
    expected_sum: u128,
    domain_ok: bool,
) -> Result<RavChecks> {
    let our_domain = domain(cfg.chain_id, cfg.verifier);
    let recovered = rav
        .recover_signer(&our_domain)
        .map_err(|e| anyhow!("RAV signer recovery failed: {e:?}"))?;
    let value_aggregate = rav.message.valueAggregate;
    Ok(RavChecks {
        recovered_signer: recovered,
        signer_ok: recovered == expected_signer,
        domain_ok,
        value_aggregate,
        expected_sum,
        value_ok: value_aggregate == expected_sum,
        payer_ok: rav.message.payer == cfg.payer,
        data_service_ok: rav.message.dataService == cfg.data_service,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use thegraph_core::alloy::signers::local::PrivateKeySigner;

    fn test_cfg() -> Config {
        Config {
            chain_id: 42161,
            verifier: Address::from_str("0x8f69F5C07477Ac46FBc491B1E6D91E2bb0111A9e").unwrap(),
            payer: Address::from_str("0x4a3156cEFBa872eb9711C5f37e52B5118323865C").unwrap(),
            data_service: Address::from_str("0xb2Bb92d0DE618878E438b55D5846cfecD9301105").unwrap(),
            service_provider: Address::from_str("0xf92f430dd8567b0d466358c79594ab58d919a6d4").unwrap(),
            allocation: Address::from_str("0xc87271758174c82e232f966bfe56c2e4615ebea7").unwrap(),
            aggregator_url: "http://localhost:7610".into(),
        }
    }

    // A minted receipt must recover to its signer under our domain — the exact
    // property the aggregator (and every real indexer) checks.
    #[test]
    fn receipt_recovers_to_signer() {
        let signer = PrivateKeySigner::from_str(
            "0x2222222222222222222222222222222222222222222222222222222222222222",
        )
        .unwrap();
        let cfg = test_cfg();
        let r = mint_receipt(&cfg, &signer, 123).unwrap();
        assert_eq!(r.message.value, 123);
        let rec = r.recover_signer(&domain(cfg.chain_id, cfg.verifier)).unwrap();
        assert_eq!(rec, signer.address());
    }

    // Tampering the value after signing must change the recovered signer.
    #[test]
    fn tampered_receipt_recovers_differently() {
        let signer = PrivateKeySigner::from_str(
            "0x2222222222222222222222222222222222222222222222222222222222222222",
        )
        .unwrap();
        let cfg = test_cfg();
        let mut r = mint_receipt(&cfg, &signer, 100).unwrap();
        r.message.value = 999_999;
        let rec = r.recover_signer(&domain(cfg.chain_id, cfg.verifier)).unwrap();
        assert_ne!(rec, signer.address());
    }

    #[test]
    fn collection_id_pads_allocation() {
        let a = Address::from_str("0xc87271758174c82e232f966bfe56c2e4615ebea7").unwrap();
        let cid = collection_id(a);
        assert_eq!(&cid.as_slice()[12..], a.as_slice());
        assert_eq!(&cid.as_slice()[..12], &[0u8; 12]);
    }

    // Tolerance is for ENCODING variations of the SAME domain only. A wrong
    // chain or a wrong verifier must NOT match — otherwise check (d) could pass
    // against a mismatched aggregator domain.
    #[test]
    fn domain_matches_same_domain_only() {
        let cfg = test_cfg();
        let good = cfg.verifier.to_string();
        let gv = good.as_str();
        let wrong_v = "0x000000000000000000000000000000000000dEaD";
        // Same domain, different chainId encodings -> all match:
        assert!(domain_matches(&serde_json::json!({"chainId": 42161, "verifyingContract": gv}), &cfg));
        assert!(domain_matches(&serde_json::json!({"chainId": "0xa4b1", "verifyingContract": gv}), &cfg));
        assert!(domain_matches(&serde_json::json!({"chainId": "42161", "verifyingContract": gv}), &cfg));
        // Wrong chain (right verifier) -> must fail:
        assert!(!domain_matches(&serde_json::json!({"chainId": 1, "verifyingContract": gv}), &cfg));
        assert!(!domain_matches(&serde_json::json!({"chainId": "0x1", "verifyingContract": gv}), &cfg));
        // Wrong verifier (right chain) -> must fail:
        assert!(!domain_matches(&serde_json::json!({"chainId": 42161, "verifyingContract": wrong_v}), &cfg));
    }
}
