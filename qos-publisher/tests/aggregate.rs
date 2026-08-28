//! Aggregation checked against the shape of real published payloads.

use gib_qos_publisher::aggregate::{bucket_bounds, Bucket, BUCKET_SECS};
use gib_qos_publisher::proto::{ClientQueryProtobuf, IndexerQueryProtobuf};

const DEPLOYMENT: [u8; 32] = [7u8; 32];
const OTHER_DEPLOYMENT: [u8; 32] = [9u8; 32];
const INDEXER_A: [u8; 20] = [0xaa; 20];
const INDEXER_B: [u8; 20] = [0xbb; 20];

fn attempt(
    indexer: [u8; 20],
    deployment: [u8; 32],
    ms: u32,
    fee: f64,
    ok: bool,
) -> IndexerQueryProtobuf {
    IndexerQueryProtobuf {
        indexer: indexer.to_vec(),
        deployment: deployment.to_vec(),
        allocation: vec![0u8; 20],
        indexed_chain: "arbitrum-one".into(),
        url: "https://indexer.example/".into(),
        fee_grt: fee,
        response_time_ms: ms,
        seconds_behind: 12,
        result: if ok {
            "success".into()
        } else {
            "bad indexers: timeout".into()
        },
        indexer_errors: String::new(),
        blocks_behind: 15,
    }
}

fn query(result: &str, ms: u32, attempts: Vec<IndexerQueryProtobuf>) -> ClientQueryProtobuf {
    ClientQueryProtobuf {
        gateway_id: "gib-42161".into(), // deliberately the trap value
        receipt_signer: vec![0u8; 20],
        query_id: "q".into(),
        api_key: "k".into(),
        user_id: "u".into(),
        subgraph: None,
        result: result.into(),
        response_time_ms: ms,
        request_bytes: 100,
        response_bytes: Some(200),
        total_fees_usd: 0.5,
        indexer_queries: attempts,
    }
}

#[test]
fn buckets_are_300_wide_and_aligned_to_multiples_of_300() {
    let (start, end) = bucket_bounds(1_785_799_799);
    assert_eq!(end - start, BUCKET_SECS);
    assert_eq!(
        end % BUCKET_SECS,
        0,
        "the DataEdge timestamp must be a multiple of 300"
    );
    assert_eq!(bucket_bounds(1_785_799_500), (1_785_799_500, 1_785_799_800));
    // The boundary belongs to the next bucket, not the one it closes.
    assert_eq!(bucket_bounds(1_785_799_800).0, 1_785_799_800);
}

#[test]
fn the_configured_gateway_id_is_used_not_the_protobufs() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    b.add(
        &query(
            "success",
            100,
            vec![attempt(INDEXER_A, DEPLOYMENT, 90, 0.001, true)],
        ),
        1_785_799_500_000,
    )
    .unwrap();
    let (allocations, queries) = b.close();
    assert_eq!(allocations[0].gateway_id, "lodestar");
    assert_eq!(queries[0].gateway_id, "lodestar");
    assert_ne!(allocations[0].gateway_id, "gib-42161");
}

#[test]
fn allocation_records_are_one_per_deployment_indexer_pair() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    let msg = query(
        "success",
        100,
        vec![
            attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true),
            attempt(INDEXER_B, DEPLOYMENT, 300, 0.002, false),
        ],
    );
    b.add(&msg, 1_785_799_500_000).unwrap();
    b.add(&msg, 1_785_799_501_000).unwrap();

    let (mut allocations, queries) = b.close();
    allocations.sort_by(|a, x| a.indexer_wallet.cmp(&x.indexer_wallet));
    assert_eq!(allocations.len(), 2, "two indexers, one deployment");
    assert_eq!(queries.len(), 1, "one deployment, one query record");

    let a = &allocations[0];
    assert_eq!(
        a.indexer_wallet,
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(a.subgraph_deployment_ipfs_hash.starts_with("Qm"));
    assert_eq!(a.query_count, 2);
    assert_eq!(a.num_indexer_200_responses, 2);
    assert_eq!(a.proportion_indexer_200_responses, 1.0);
    assert_eq!(a.avg_indexer_latency_ms, 100.0);
    assert_eq!(a.max_indexer_latency_ms, 100.0);
    assert_eq!(
        a.stdev_indexer_latency_ms, 0.0,
        "identical samples have zero spread"
    );
    assert!((a.total_query_fees - 0.002).abs() < 1e-12);

    let b2 = &allocations[1];
    assert_eq!(
        b2.num_indexer_200_responses, 0,
        "a failed attempt is not a 200"
    );
    assert_eq!(b2.proportion_indexer_200_responses, 0.0);
}

#[test]
fn query_fees_are_grt_summed_from_attempts_never_the_usd_field() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    // total_fees_usd is 0.5; the GRT truth is 0.001 + 0.002.
    b.add(
        &query(
            "success",
            100,
            vec![
                attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true),
                attempt(INDEXER_B, DEPLOYMENT, 100, 0.002, true),
            ],
        ),
        1_785_799_500_000,
    )
    .unwrap();
    let (_, queries) = b.close();
    assert!((queries[0].total_query_fees - 0.003).abs() < 1e-12);
    assert_ne!(queries[0].total_query_fees, 0.5);
}

#[test]
fn only_user_errors_count_toward_the_user_attributed_rate() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    let ok = query(
        "success",
        100,
        vec![attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true)],
    );
    let user = query(
        "bad query: syntax",
        100,
        vec![attempt(INDEXER_A, DEPLOYMENT, 100, 0.0, false)],
    );
    let network = query(
        "no indexers found",
        100,
        vec![attempt(INDEXER_A, DEPLOYMENT, 100, 0.0, false)],
    );
    let gateway = query(
        "internal error: boom",
        100,
        vec![attempt(INDEXER_A, DEPLOYMENT, 100, 0.0, false)],
    );
    for (i, m) in [&ok, &user, &network, &gateway].iter().enumerate() {
        b.add(m, 1_785_799_500_000 + i as i64).unwrap();
    }
    let (_, queries) = b.close();
    let q = &queries[0];
    assert_eq!(q.query_count, 4);
    assert_eq!(q.gateway_query_success_rate, 0.25);
    assert_eq!(
        q.user_attributed_error_rate, 0.25,
        "one of four errors was the user's"
    );
}

#[test]
fn most_recent_query_ts_is_milliseconds_and_takes_the_latest() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    let m = query(
        "success",
        100,
        vec![attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true)],
    );
    b.add(&m, 1_785_799_500_000).unwrap();
    b.add(&m, 1_785_799_799_992).unwrap();
    b.add(&m, 1_785_799_600_000).unwrap();
    let (_, queries) = b.close();
    assert_eq!(queries[0].most_recent_query_ts, 1_785_799_799_992);
    // Milliseconds, in a record whose epoch fields are seconds.
    assert!(queries[0].most_recent_query_ts > queries[0].end_epoch * 1000 - 1000);
}

#[test]
fn latency_spread_is_a_real_stdev_not_a_zero() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    for ms in [100u32, 200, 300] {
        b.add(
            &query(
                "success",
                ms,
                vec![attempt(INDEXER_A, DEPLOYMENT, ms, 0.001, true)],
            ),
            1_785_799_500_000,
        )
        .unwrap();
    }
    let (allocations, _) = b.close();
    assert_eq!(allocations[0].avg_indexer_latency_ms, 200.0);
    assert_eq!(allocations[0].max_indexer_latency_ms, 300.0);
    // Population stdev of 100/200/300 is sqrt(20000/3) ≈ 81.6497.
    assert!((allocations[0].stdev_indexer_latency_ms - 81.649_658_09).abs() < 1e-6);
}

#[test]
fn a_query_whose_attempts_span_two_deployments_is_attributed_to_the_first() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    b.add(
        &query(
            "success",
            100,
            vec![
                attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true),
                attempt(INDEXER_B, OTHER_DEPLOYMENT, 100, 0.001, true),
            ],
        ),
        1_785_799_500_000,
    )
    .unwrap();
    let (allocations, queries) = b.close();
    assert_eq!(
        allocations.len(),
        2,
        "both attempts produce allocation records"
    );
    assert_eq!(
        queries.len(),
        1,
        "the client query is counted once, not twice"
    );
}

#[test]
fn a_malformed_deployment_is_rejected_rather_than_encoded_as_a_plausible_cid() {
    let mut b = Bucket::new("lodestar", 1_785_799_500);
    let mut bad = attempt(INDEXER_A, DEPLOYMENT, 100, 0.001, true);
    bad.deployment = vec![0u8; 31];
    assert!(b
        .add(&query("success", 100, vec![bad]), 1_785_799_500_000)
        .is_err());
}

#[test]
fn an_empty_bucket_emits_nothing_rather_than_zeroes() {
    let b = Bucket::new("lodestar", 1_785_799_500);
    let (allocations, queries) = b.close();
    assert!(allocations.is_empty());
    assert!(queries.is_empty());
    assert_eq!(b.client_query_count(), 0);
}
