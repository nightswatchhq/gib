#!/usr/bin/env bash
# Generate a sender (payer) and signer keypair for the gateway, writing each to
# a mode-600 file in SECRETS_DIR. The sender holds funds and deposits escrow;
# the signer signs TAP receipts/RAVs. Keeping them separate limits blast radius
# if the signer key (which lives in running containers) leaks.
#
#   ./scripts/gen-keys.sh                 # -> ./secrets/{sender,signer}.txt
#   SECRETS_DIR=/root/gib-secrets ./scripts/gen-keys.sh
#
# Uses `cast` (Foundry) if installed, otherwise the Foundry Docker image — gib
# already requires Docker, so no host Foundry install is needed.
set -euo pipefail
cd "$(dirname "$0")/.."

SECRETS_DIR="${SECRETS_DIR:-./secrets}"
FOUNDRY_IMAGE="${FOUNDRY_IMAGE:-ghcr.io/foundry-rs/foundry:latest}"

mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"

# Emit a fresh wallet as text (an "Address:" line and a "Private key:" line).
new_wallet() {
  if command -v cast >/dev/null; then
    cast wallet new
  elif command -v docker >/dev/null; then
    docker run --rm "$FOUNDRY_IMAGE" 'cast wallet new'
  else
    echo "ERROR: need either 'cast' (https://getfoundry.sh) or Docker to generate keys." >&2
    exit 1
  fi
}

gen() { # name -> writes $SECRETS_DIR/$name.txt if absent
  local name="$1" file="$SECRETS_DIR/$1.txt"
  if [ -f "$file" ]; then
    echo "  $name.txt exists — keeping it (delete to regenerate)."
    return
  fi
  local out; out="$(new_wallet)"
  # Extract by field name (robust across cast versions), not by line number.
  { echo "$out" | grep -iE '^Address'; echo "$out" | grep -iE 'Private key'; } > "$file"
  chmod 600 "$file"
  grep -qiE 'Private key.*0x[0-9a-fA-F]{64}' "$file" \
    || { echo "ERROR: failed to parse a private key into $file" >&2; exit 1; }
  echo "  wrote $file"
  grep -i 'Address' "$file"
}

echo "Generating keys in $SECRETS_DIR (existing files preserved)..."
gen sender
gen signer

echo
echo "NEXT:"
echo "  • Fund the SENDER address above with ETH (gas) + GRT (escrow backing) on Arbitrum One."
echo "  • Put the SENDER address into .env as SENDER_ADDRESS."
echo "  • Back up these files somewhere safe and OFF this box. Never commit them."
