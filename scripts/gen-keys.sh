#!/usr/bin/env bash
# Generate a sender (payer) and signer keypair for the gateway, writing each to
# a mode-600 file in SECRETS_DIR. The sender holds funds and deposits escrow;
# the signer signs TAP receipts/RAVs. Keeping them separate limits blast radius
# if the signer key (which lives in running containers) leaks.
#
#   ./scripts/gen-keys.sh                 # -> ./secrets/{sender,signer}.txt
#   SECRETS_DIR=/root/gib-secrets ./scripts/gen-keys.sh
#
# Requires `cast` (Foundry). Install: https://getfoundry.sh
set -euo pipefail
cd "$(dirname "$0")/.."

SECRETS_DIR="${SECRETS_DIR:-./secrets}"
command -v cast >/dev/null || { echo "ERROR: 'cast' (Foundry) required. https://getfoundry.sh" >&2; exit 1; }

mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"

gen() { # name -> writes $SECRETS_DIR/$name.txt if absent
  local name="$1" file="$SECRETS_DIR/$1.txt"
  if [ -f "$file" ]; then
    echo "  $name.txt exists — keeping it (delete to regenerate)."
    return
  fi
  cast wallet new | sed -n '1,2p' > "$file"   # "Address:" + "Private key:" lines
  chmod 600 "$file"
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
