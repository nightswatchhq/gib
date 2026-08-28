//! `gib onboard` — stop an operator sending a broken onboarding request.
//!
//! Onboarding an indexer is a human handshake (`docs/04-indexer-onboarding.md`): you hand over a
//! sender address and an aggregator URL, they paste one TOML block and restart. It is described as
//! the slowest part of standing up an independent gateway, and the reason is that every failure is
//! discovered by the *indexer*, hours later, as receipts that bounce.
//!
//! That is the wrong way round. Every one of the failures below is visible from your own side
//! before you ask anyone for anything. This module checks them and refuses to produce the block
//! until they pass, so the worst case is you fixing your own deployment instead of an indexer
//! losing an afternoon to it.

use std::collections::BTreeMap;

/// A single onboarding pre-flight check.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub outcome: Outcome,
    /// What is wrong and what to do about it. Empty when the check passed cleanly.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    /// Not fatal, but the indexer will notice. Still emits the block.
    Warn,
    /// Emitting the block would waste the indexer's time. Blocks output.
    Fail,
}

impl Check {
    fn pass(name: &'static str) -> Self {
        Self {
            name,
            outcome: Outcome::Pass,
            detail: String::new(),
        }
    }
    fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Warn,
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Fail,
            detail: detail.into(),
        }
    }
}

/// Is this URL something an indexer on the public internet could actually reach?
///
/// The single most common way to waste an indexer's time is handing them a loopback or
/// private-range address, because it works perfectly from the operator's own shell.
pub fn check_aggregator_url(url: &str) -> Vec<Check> {
    const NAME: &str = "aggregator URL is reachable by an indexer";
    const TLS: &str = "aggregator URL terminates TLS";

    let Some(rest) = url
        .strip_prefix("https://")
        .map(|r| (r, true))
        .or_else(|| url.strip_prefix("http://").map(|r| (r, false)))
    else {
        return vec![Check::fail(
            NAME,
            format!("`{url}` has no http:// or https:// scheme"),
        )];
    };
    let (rest, is_tls) = rest;
    let host = rest
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let unreachable = host.is_empty()
        || host == "localhost"
        || host.starts_with("127.")
        || host == "0.0.0.0"
        || host == "::1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || (host.starts_with("172.")
            && host
                .split('.')
                .nth(1)
                .and_then(|o| o.parse::<u8>().ok())
                .is_some_and(|o| (16..=31).contains(&o)))
        // Compose service names resolve inside the stack and nowhere else. `gib smoke` uses these
        // on purpose; an indexer cannot.
        || !host.contains('.');

    let mut checks = Vec::new();
    if unreachable {
        checks.push(Check::fail(
            NAME,
            format!(
                "`{host}` is loopback, private-range, or a Compose service name. An indexer on \
                 the public internet cannot reach it. Put the aggregator behind a reverse proxy \
                 with a stable DNS name (docs/04) and pass that URL."
            ),
        ));
    } else {
        checks.push(Check::pass(NAME));
    }

    if is_tls {
        checks.push(Check::pass(TLS));
    } else {
        checks.push(Check::warn(
            TLS,
            "plain http. You are asking an indexer to trust this endpoint with their receipt \
             aggregation; some will decline, and all of them should.",
        ));
    }
    checks
}

/// Compares what the gateway was *rendered* with against what `fetch-addresses.sh` *fetched*.
///
/// Both sides of the handshake derive from the same Horizon address book, so they agree until
/// someone hand-edits one. Doc 04 names a stale config as the first thing to check when receipts
/// bounce; this catches it before any bounce.
pub fn check_address_drift(
    rendered: &BTreeMap<String, String>,
    fetched: &BTreeMap<String, String>,
) -> Vec<Check> {
    const NAME: &str = "collector + subgraph service match the address book";
    let mut drift = Vec::new();
    for (key, want) in fetched {
        if let Some(have) = rendered.get(key) {
            if !have.eq_ignore_ascii_case(want) {
                drift.push(format!(
                    "{key}: gateway has {have}, address book says {want}"
                ));
            }
        }
    }
    if drift.is_empty() {
        vec![Check::pass(NAME)]
    } else {
        vec![Check::fail(
            NAME,
            format!(
                "{}. Re-run scripts/fetch-addresses.sh and scripts/render.sh; never hand-copy \
                 these.",
                drift.join("; ")
            ),
        )]
    }
}

/// Everything an indexer needs, in one paste.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexerBlock {
    pub sender: String,
    pub aggregator_url: String,
    pub collector: String,
    pub subgraph_service: String,
    pub chain_id: u64,
}

impl IndexerBlock {
    /// The message to send an indexer. Deliberately includes the two addresses they should verify
    /// against their own config, because "receipts bounce" is otherwise undiagnosable from either
    /// side alone.
    pub fn render(&self) -> String {
        format!(
            "Add to your TAP config and restart indexer-service + tap-agent:\n\
             \n\
             [tap.sender_aggregator_endpoints]\n\
             \"{sender}\" = \"{url}\"\n\
             \n\
             Before you do, check these match your own config, or my receipts will bounce:\n\
             \n\
               receipts_verifier_address_v2 = {collector}\n\
               subgraph_service_address     = {service}\n\
               chain id                     = {chain}\n",
            sender = self.sender,
            url = self.aggregator_url,
            collector = self.collector,
            service = self.subgraph_service,
            chain = self.chain_id,
        )
    }
}

/// Whether the checks permit sending the block at all.
pub fn may_onboard(checks: &[Check]) -> bool {
    !checks.iter().any(|c| c.outcome == Outcome::Fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(url: &str) -> Outcome {
        check_aggregator_url(url)[0].outcome
    }

    #[test]
    fn a_public_https_host_passes_both_checks() {
        let checks = check_aggregator_url("https://aggregator.example.com");
        assert!(checks.iter().all(|c| c.outcome == Outcome::Pass));
        assert!(may_onboard(&checks));
    }

    #[test]
    fn loopback_and_private_ranges_fail_because_an_indexer_cannot_reach_them() {
        for url in [
            "http://localhost:7610",
            "http://127.0.0.1:7610",
            "https://0.0.0.0",
            "http://10.1.2.3:7610",
            "http://192.168.1.10",
            "http://172.16.0.1",
            "http://172.31.255.255",
            "http://169.254.1.1",
        ] {
            assert_eq!(outcome(url), Outcome::Fail, "{url} should be unreachable");
            assert!(
                !may_onboard(&check_aggregator_url(url)),
                "{url} must block the block"
            );
        }
    }

    /// 172.15 and 172.32 are public; only 172.16-172.31 is private. Getting this wrong would
    /// refuse a perfectly good address.
    #[test]
    fn the_172_private_range_boundary_is_exact() {
        assert_eq!(outcome("http://172.15.0.1"), Outcome::Pass);
        assert_eq!(outcome("http://172.16.0.1"), Outcome::Fail);
        assert_eq!(outcome("http://172.31.0.1"), Outcome::Fail);
        assert_eq!(outcome("http://172.32.0.1"), Outcome::Pass);
    }

    /// `gib smoke` legitimately talks to these; an onboarding request must not.
    #[test]
    fn compose_service_names_fail() {
        assert_eq!(outcome("http://gib-tap-aggregator:7610"), Outcome::Fail);
    }

    #[test]
    fn plain_http_warns_but_does_not_block() {
        let checks = check_aggregator_url("http://aggregator.example.com");
        assert_eq!(checks[0].outcome, Outcome::Pass);
        assert_eq!(checks[1].outcome, Outcome::Warn);
        assert!(may_onboard(&checks), "http is rude, not fatal");
    }

    #[test]
    fn a_missing_scheme_fails_rather_than_being_guessed_at() {
        assert_eq!(outcome("aggregator.example.com"), Outcome::Fail);
    }

    #[test]
    fn address_drift_is_reported_per_key_and_blocks() {
        let fetched = BTreeMap::from([
            (
                "GRAPH_TALLY_COLLECTOR".into(),
                "0x8f69F5C07477Ac46FBc491B1E6D91E2bb0111A9e".into(),
            ),
            (
                "SUBGRAPH_SERVICE".into(),
                "0xb2Bb92d0DE618878E438b55D5846cfecD9301105".into(),
            ),
        ]);
        let mut rendered = fetched.clone();
        rendered.insert("GRAPH_TALLY_COLLECTOR".into(), "0xdeadbeef".into());
        let checks = check_address_drift(&rendered, &fetched);
        assert_eq!(checks[0].outcome, Outcome::Fail);
        assert!(checks[0].detail.contains("GRAPH_TALLY_COLLECTOR"));
        assert!(
            !checks[0].detail.contains("SUBGRAPH_SERVICE"),
            "only the drifted key"
        );
        assert!(!may_onboard(&checks));
    }

    /// Case differs between checksummed and lowercase forms of the same address; that is not drift.
    #[test]
    fn address_comparison_ignores_checksum_case() {
        let fetched = BTreeMap::from([(
            "GRAPH_TALLY_COLLECTOR".into(),
            "0x8f69F5C07477Ac46FBc491B1E6D91E2bb0111A9e".into(),
        )]);
        let rendered = BTreeMap::from([(
            "GRAPH_TALLY_COLLECTOR".into(),
            "0x8f69f5c07477ac46fbc491b1e6d91e2bb0111a9e".into(),
        )]);
        assert_eq!(
            check_address_drift(&rendered, &fetched)[0].outcome,
            Outcome::Pass
        );
    }

    #[test]
    fn the_rendered_block_carries_the_sender_the_url_and_both_verification_addresses() {
        let block = IndexerBlock {
            sender: "0x1111111111111111111111111111111111111111".into(),
            aggregator_url: "https://aggregator.example.com".into(),
            collector: "0x8f69F5C07477Ac46FBc491B1E6D91E2bb0111A9e".into(),
            subgraph_service: "0xb2Bb92d0DE618878E438b55D5846cfecD9301105".into(),
            chain_id: 42161,
        };
        let out = block.render();
        assert!(out.contains("[tap.sender_aggregator_endpoints]"));
        assert!(out.contains(
            "\"0x1111111111111111111111111111111111111111\" = \"https://aggregator.example.com\""
        ));
        assert!(out
            .contains("receipts_verifier_address_v2 = 0x8f69F5C07477Ac46FBc491B1E6D91E2bb0111A9e"));
        assert!(out
            .contains("subgraph_service_address     = 0xb2Bb92d0DE618878E438b55D5846cfecD9301105"));
        assert!(out.contains("42161"));
    }
}
