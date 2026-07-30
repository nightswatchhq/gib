# 05 — Upgrades & fork maintenance

Two moving parts age independently: the **images** the box pulls, and the **gateway fork**
they're built from.

## Upgrading the box (operator)

Pin versions in `.env` and bump deliberately:

```sh
GATEWAY_IMAGE_TAG=v27.6.0    # ghcr.io/nightswatchhq/gateway
AGGREGATOR_IMAGE_TAG=v0.7.1  # ghcr.io/graphprotocol/graph_tally_aggregator
ESCROW_IMAGE_TAG=v2.0.0      # ghcr.io/graphprotocol/graph_tally_escrow_manager
```

```sh
docker compose --env-file runtime/.env pull
./scripts/render.sh
docker compose --env-file runtime/.env up -d
```

Prefer a pinned version tag over `latest` in production — `latest` tracks the fork's default
branch and can move under you.

## Maintaining the gateway fork (maintainer)

The gateway image is built from `nightswatchhq/gateway`, a fork of `edgeandnode/gateway`.
Keep the fork close to upstream by rebasing, carrying the box's patches as a small, isolated
commit series so merges stay cheap:

1. **`dynamic-kafka`** — the macOS/librdkafka linking work.
2. **`Fixed` API-key / Studio-stub defaults** — the box uses the built-in static API-key mode
   instead of Edge & Node's Studio key service.
3. **Selection weights** — the optional `selection` config block (depends on the
   `nightswatchhq/candidate-selection` fork; keep that fork's rev pinned in `Cargo.toml`).
4. **Address-book auto-fill** — lives in gib, not the fork, so it doesn't touch rebases.

Cadence: track upstream releases (fork was cut at `v27.6.0`; upstream moves through
`v27.7.x`). For each upstream tag: rebase the patch series, run `cargo test`, cut a
`nightswatchhq/gateway` tag → the GHCR workflow publishes the multi-arch image → bump
`GATEWAY_IMAGE_TAG` in the box.

## When to re-baseline (decision triggers)

- **graph-tally escrow config drifts** from the current shape → re-check
  `config/escrow-manager.json.tmpl` against the `graphprotocol/graph-tally` repo before the
  next release.
- **Upstream removes `Fixed` API-key mode** → the Studio-stub strategy needs revisiting.
- **Horizon contract addresses change** → the auto-fill script is the safeguard; just
  re-run `fetch-addresses.sh` and re-render. Never hand-copy.
