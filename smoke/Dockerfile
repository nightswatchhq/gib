# gib-smoke — build the self-test binary, run it as a container on the gib
# network so it reaches gateway / tap-aggregator / redpanda by name.
FROM rust:1-slim-bookworm AS build
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release --bin gib-smoke

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/gib-smoke /usr/local/bin/gib-smoke
ENTRYPOINT ["gib-smoke"]
