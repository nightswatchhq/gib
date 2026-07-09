#!/usr/bin/env bash
# Render runtime configs from .env + config/addresses.env + your key files.
# Then deploy:  docker compose --env-file runtime/.env up -d
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env ] || { echo "ERROR: copy .env.example to .env and fill it first." >&2; exit 1; }
[ -f config/addresses.env ] || { echo "ERROR: run ./scripts/fetch-addresses.sh first (no config/addresses.env)." >&2; exit 1; }
command -v envsubst >/dev/null || { echo "ERROR: envsubst missing (apt-get install -y gettext-base)." >&2; exit 1; }

# Load KEY=VALUE files literally (NOT shell-sourced), so values with spaces/commas
# (e.g. multiple API keys) don't get executed. Same literal semantics docker uses.
load_env() {
  local line key val
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    case "$line" in ''|\#*) continue ;; esac
    [ "${line#*=}" = "$line" ] && continue   # no '=' -> skip
    key="${line%%=*}"; val="${line#*=}"
    key="${key//[[:space:]]/}"
    case "$val" in \"*\") val="${val#\"}"; val="${val%\"}" ;; \'*\') val="${val#\'}"; val="${val%\'}" ;; esac
    export "$key=$val"
  done < "$1"
}
load_env ./.env
load_env ./config/addresses.env

# --- Keys: read from SECRETS_DIR, never from .env or git -------------------
SECRETS_DIR="${SECRETS_DIR:-./secrets}"
SIGNER_KEY=$(grep -i 'Private key' "$SECRETS_DIR/signer.txt" | grep -oiE '0x[0-9a-f]{64}' | head -1)
SENDER_KEY=$(grep -i 'Private key' "$SECRETS_DIR/sender.txt" | grep -oiE '0x[0-9a-f]{64}' | head -1)
[ -n "$SIGNER_KEY" ] && [ -n "$SENDER_KEY" ] || {
  echo "ERROR: could not read 0x-keys from $SECRETS_DIR/{signer,sender}.txt (run ./scripts/gen-keys.sh)." >&2; exit 1; }
export SIGNER_KEY SENDER_KEY

# --- Required-field sanity -------------------------------------------------
: "${SENDER_ADDRESS:?set SENDER_ADDRESS in .env}"
: "${NETWORK_SUBGRAPH_URL:?set NETWORK_SUBGRAPH_URL in .env}"
: "${GATEWAY_API_KEYS:?set GATEWAY_API_KEYS in .env}"
: "${GRAPH_TALLY_COLLECTOR:?addresses.env missing GRAPH_TALLY_COLLECTOR — re-run fetch-addresses.sh}"

# --- Build the api_keys JSON array from comma-separated GATEWAY_API_KEYS ----
GATEWAY_API_KEYS_JSON=""
IFS=',' read -ra _keys <<< "$GATEWAY_API_KEYS"
for k in "${_keys[@]}"; do
  k="$(echo "$k" | xargs)"   # trim
  [ -z "$k" ] && continue
  [ -n "$GATEWAY_API_KEYS_JSON" ] && GATEWAY_API_KEYS_JSON+=","
  GATEWAY_API_KEYS_JSON+="{\"key\":\"$k\",\"user_address\":\"$SENDER_ADDRESS\",\"query_status\":\"ACTIVE\"}"
done
export GATEWAY_API_KEYS_JSON

# --- Optional selection weights block (only if any exponent is set) ---------
# Emits: "selection": { "exponents": [sr, lat, sb, grt] },   (trailing comma)
sr="${SELECTION_WEIGHT_SUCCESS_RATE:-}"
lat="${SELECTION_WEIGHT_LATENCY:-}"
sb="${SELECTION_WEIGHT_SECONDS_BEHIND:-}"
grt="${SELECTION_WEIGHT_SLASHABLE_GRT:-}"
if [ -n "$sr$lat$sb$grt" ]; then
  SELECTION_JSON="\"selection\": { \"exponents\": [${sr:-1.0}, ${lat:-1.0}, ${sb:-1.0}, ${grt:-1.0}] },
  "
else
  SELECTION_JSON=""
fi
export SELECTION_JSON

# --- Render ----------------------------------------------------------------
mkdir -p runtime
envsubst < config/gateway.json.tmpl        > runtime/gateway.json
envsubst < config/escrow-manager.json.tmpl > runtime/escrow-manager.json

# Validate the JSON we produced (catches template/substitution mistakes early).
if command -v python3 >/dev/null; then
  python3 -c "import json,sys; json.load(open('runtime/gateway.json'))" \
    || { echo "ERROR: runtime/gateway.json is not valid JSON." >&2; exit 1; }
  python3 -c "import json,sys; json.load(open('runtime/escrow-manager.json'))" \
    || { echo "ERROR: runtime/escrow-manager.json is not valid JSON." >&2; exit 1; }
fi

# Compose env-file: .env values + addresses + injected keys (gitignored, mode 600).
{
  grep -vE '^\s*#|^\s*$' .env
  grep -vE '^\s*#|^\s*$' config/addresses.env
  echo "SIGNER_KEY=$SIGNER_KEY"
  echo "SENDER_KEY=$SENDER_KEY"
} > runtime/.env

chmod 600 runtime/.env runtime/gateway.json runtime/escrow-manager.json
echo "Rendered runtime/{gateway.json,escrow-manager.json,.env}"
echo "Deploy with:  docker compose --env-file runtime/.env up -d"
[ -n "$SELECTION_JSON" ] && echo "Custom selection weights: [sr=${sr:-1.0} lat=${lat:-1.0} sb=${sb:-1.0} grt=${grt:-1.0}]"
[ "${ESCROW_DRY_RUN:-true}" = "true" ] && echo "NOTE: ESCROW_DRY_RUN=true — escrow-manager will NOT send on-chain txs. Set false in .env to fund escrow."
true