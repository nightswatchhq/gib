# 01 — Keys

The gateway is a TAP **sender**: it signs receipts that indexers redeem for GRT out of
*your* escrow. Two keys, deliberately separated:

| Key | Holds funds? | Lives where | Job |
|-----|--------------|-------------|-----|
| **sender** | Yes (ETH + GRT) | secrets dir, used by escrow-manager only | authorizes signers, deposits escrow |
| **signer** | No | secrets dir + inside the gateway & aggregator containers | signs TAP v2 receipts and RAVs |

Splitting them limits blast radius: the signer key is exposed at runtime (it's in the
gateway and aggregator), the sender key — which controls your money — is only touched by
the escrow-manager for on-chain transactions.

## Generate

```sh
./scripts/gen-keys.sh                 # -> ./secrets/{sender,signer}.txt (mode 600)
# or point elsewhere:
SECRETS_DIR=/root/gib-secrets ./scripts/gen-keys.sh
```

Requires [`cast`](https://getfoundry.sh) (Foundry). Each file gets an `Address:` and a
`Private key:` line. `render.sh` reads the `0x…` key from these; it never stores keys in
`.env` or the compose env-file that lands in git.

## Handle with care

- **Back up both files off the box.** Losing the sender key means losing access to escrowed GRT.
- Never commit them — `.gitignore` already excludes `secrets/`, `.env`, `runtime/`.
- Put the **sender address** (not the key) into `.env` as `SENDER_ADDRESS`.
- Fund the sender: a little ETH for Arbitrum gas + the GRT you want to back escrow with
  (see [02 — On-chain escrow](02-onchain-escrow.md)).

## Bring your own keys

Already have wallets? Just create the two files by hand:

```
Address:     0x...
Private key: 0x...
```

`gen-keys.sh` preserves existing files — it won't overwrite them.
