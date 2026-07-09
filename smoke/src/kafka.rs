//! Reads `gateway_queries` records over Redpanda's HTTP proxy (pandaproxy) and
//! decodes the `ClientQueryProtobuf` — the RUNTIME evidence that the running
//! gateway signed with the configured signer. We only decode the two fields we
//! need; prost ignores the rest (graph_env_id, query id, …), exactly as the
//! escrow-manager does.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use prost::Message as _;
use thegraph_core::alloy::primitives::Address;

/// Subset of the gateway's `ClientQueryProtobuf` on `gateway_queries`.
/// NB: this topic carries fee METADATA, not signed receipts — see docs.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientQuery {
    /// 20-byte signer address the gateway used for this query's receipts.
    #[prost(bytes = "vec", tag = "2")]
    pub receipt_signer: Vec<u8>,
    #[prost(message, repeated, tag = "10")]
    pub indexer_queries: Vec<IndexerQuery>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct IndexerQuery {
    #[prost(bytes = "vec", tag = "1")]
    pub indexer: Vec<u8>,
    #[prost(double, tag = "6")]
    pub fee_grt: f64,
}

impl ClientQuery {
    pub fn signer(&self) -> Option<Address> {
        (self.receipt_signer.len() == 20).then(|| Address::from_slice(&self.receipt_signer))
    }
}

/// Fetch records from partition 0 of `gateway_queries` starting at `offset`,
/// return decoded `ClientQuery`s paired with their kafka offsets.
pub async fn fetch_client_queries(
    pandaproxy_url: &str,
    offset: i64,
) -> Result<Vec<(i64, ClientQuery)>> {
    let url = format!(
        "{}/topics/gateway_queries/partitions/0/records?offset={}&timeout=5000&max_bytes=1048576",
        pandaproxy_url.trim_end_matches('/'),
        offset
    );
    let body: serde_json::Value = reqwest::Client::new()
        .get(&url)
        .header("Accept", "application/vnd.kafka.binary.v2+json")
        .send()
        .await
        .context("pandaproxy request failed")?
        .json()
        .await
        .context("pandaproxy response not JSON")?;
    let arr = body
        .as_array()
        .ok_or_else(|| anyhow!("pandaproxy did not return a record array: {body}"))?;
    let mut out = Vec::new();
    for rec in arr {
        let off = rec.get("offset").and_then(|o| o.as_i64()).unwrap_or(-1);
        if let Some(v) = rec.get("value").and_then(|v| v.as_str()) {
            let raw = STANDARD.decode(v).context("record value not base64")?;
            if let Ok(cq) = ClientQuery::decode(raw.as_slice()) {
                out.push((off, cq));
            }
        }
    }
    Ok(out)
}

/// Latest `ClientQuery` on the topic (highest offset), or None if empty.
pub async fn latest_client_query(pandaproxy_url: &str) -> Result<Option<(i64, ClientQuery)>> {
    let mut recs = fetch_client_queries(pandaproxy_url, 0).await?;
    recs.sort_by_key(|(o, _)| *o);
    Ok(recs.pop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    // Round-trip, and — critically — a record shaped like the REAL one (a tag-1
    // graph_env_id field before receipt_signer at tag 2) must still decode: prost
    // ignores the unknown field, exactly as the escrow-manager relies on.
    #[test]
    fn decodes_signer_ignoring_extra_fields() {
        let signer = Address::from_slice(&[0x29; 20]);
        let mut buf = Vec::new();
        // field 1 (graph_env_id), wire type 2: 0x0a, len 9, "gib-42161"
        buf.extend_from_slice(&[0x0a, 0x09]);
        buf.extend_from_slice(b"gib-42161");
        // field 2 (receipt_signer), wire type 2: 0x12, len 20, <addr>
        buf.push(0x12);
        buf.push(0x14);
        buf.extend_from_slice(signer.as_slice());
        let cq = ClientQuery::decode(buf.as_slice()).expect("decode");
        assert_eq!(cq.signer(), Some(signer));
    }

    #[test]
    fn roundtrip_with_indexer_queries() {
        let cq = ClientQuery {
            receipt_signer: vec![0x29; 20],
            indexer_queries: vec![IndexerQuery { indexer: vec![0x01; 20], fee_grt: 1.5e-6 }],
        };
        let back = ClientQuery::decode(cq.encode_to_vec().as_slice()).unwrap();
        assert_eq!(back.indexer_queries.len(), 1);
        assert!(back.signer().is_some());
    }
}
