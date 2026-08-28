//! `gib-qos-publisher` — consume the gateway's query stream, emit oracle payloads.
//!
//! Dry-run only for now: it prints what it *would* publish. Pinning and posting come next and both
//! need funded keys.

use anyhow::Context;
use clap::Parser;
use gib_qos_publisher::{
    aggregate::{bucket_bounds, Bucket},
    oracle::{TOPIC_ALLOCATION, TOPIC_QUERY},
    proto::ClientQueryProtobuf,
};
use prost::Message;
use rdkafka::{
    consumer::{Consumer, StreamConsumer},
    ClientConfig, Message as _,
};

#[derive(Parser, Debug)]
#[command(about = "Aggregate a gateway's Kafka query stream into Gateway QoS Oracle payloads")]
struct Args {
    /// Kafka/Redpanda bootstrap servers, e.g. `redpanda:9092`.
    #[arg(long, env = "KAFKA_BROKERS")]
    brokers: String,

    /// The gateway's query topic.
    #[arg(long, env = "KAFKA_QUERY_TOPIC", default_value = "gateway_queries")]
    topic: String,

    /// Your oracle identity, stable forever once chosen.
    ///
    /// NOT the `gateway_id` in the protobuf: the gateway fills that from `graph_env_id`, which gib
    /// templates as `gib-${CHAIN_ID}` — identical for every gib operator. Publishing under that
    /// would put every gib gateway on earth into one bucket.
    #[arg(long, env = "QOS_GATEWAY_ID")]
    gateway_id: String,

    /// Seconds to wait past a bucket's end before closing it, for late messages.
    #[arg(long, env = "QOS_CLOSE_DELAY_SECS", default_value_t = 60)]
    close_delay_secs: i64,

    /// Print payloads instead of pinning and posting. The only supported mode today.
    #[arg(long, default_value_t = true)]
    dry_run: bool,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs() as i64
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    if !args.dry_run {
        anyhow::bail!(
            "only --dry-run is implemented. Pinning and posting need funded keys, and an \
             unpinned payload is a permanent hole in every consumer's history rather than a \
             retryable failure."
        );
    }

    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &args.brokers)
        .set("group.id", "gib-qos-publisher")
        .set("enable.auto.commit", "true")
        .set("auto.offset.reset", "latest")
        .create()
        .context("kafka consumer")?;
    consumer
        .subscribe(&[&args.topic])
        .with_context(|| format!("subscribe to {}", args.topic))?;

    tracing::info!(
        brokers = %args.brokers, topic = %args.topic, gateway_id = %args.gateway_id,
        "consuming; buckets close {}s after their end", args.close_delay_secs
    );

    let mut current: Option<Bucket> = None;

    loop {
        let msg = consumer.recv().await.context("kafka recv")?;
        let Some(payload) = msg.payload() else {
            continue;
        };
        let decoded = match ClientQueryProtobuf::decode(payload) {
            Ok(d) => d,
            Err(err) => {
                // A decode failure means the gateway's schema moved. Say so loudly rather than
                // dropping traffic into a silently-thinning bucket.
                tracing::error!(%err, "could not decode a gateway message; schema drift?");
                continue;
            }
        };
        let received_ms = msg.timestamp().to_millis().unwrap_or(now_secs() * 1000);
        let (start, _) = bucket_bounds(received_ms / 1000);

        // Close the open bucket once we see traffic from a later one and the delay has elapsed.
        if let Some(b) = &current {
            if b.start != start && now_secs() >= b.end + args.close_delay_secs {
                let bucket = current.take().expect("checked above");
                emit(&bucket, args.dry_run);
            }
        }
        let bucket =
            current.get_or_insert_with(|| Bucket::new(&args.gateway_id, received_ms / 1000));
        if bucket.start == start {
            if let Err(err) = bucket.add(&decoded, received_ms) {
                tracing::warn!(%err, "skipped a malformed record");
            }
        }
    }
}

fn emit(bucket: &Bucket, dry_run: bool) {
    let (allocations, queries) = bucket.close();
    // Publishing nothing looks exactly like publishing perfection: a gateway with no traffic emits
    // no records and every consumer renders that as blank rather than absent. Log the emptiness.
    tracing::info!(
        bucket_end = bucket.end,
        client_queries = bucket.client_query_count(),
        allocation_records = allocations.len(),
        query_records = queries.len(),
        dry_run,
        "bucket closed"
    );
    if dry_run {
        println!(
            "{}",
            serde_json::json!({
                "topic": TOPIC_ALLOCATION, "timestamp": bucket.end, "records": allocations
            })
        );
        println!(
            "{}",
            serde_json::json!({
                "topic": TOPIC_QUERY, "timestamp": bucket.end, "records": queries
            })
        );
    }
}
