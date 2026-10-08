#!/usr/bin/env bash
# Full M1 flow against a local validator with the V1 transaction feature active.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PROGRAM_KEYPAIR=${QUBIT_PROGRAM_KEYPAIR:-$HOME/.config/qubit/deploy/qubit_vault-keypair.json}
PROGRAM_ID=$(solana address -k "$PROGRAM_KEYPAIR")
WORK=$(mktemp -d)
LEDGER=$WORK/ledger
CONFIG_A=$WORK/config-a
CONFIG_B=$WORK/config-b
PAYER=$WORK/payer.json
HOT=$WORK/hot.json
URL=http://127.0.0.1:8899
QUBIT="$ROOT/target/debug/qubit --url localnet --fee-payer $PAYER"

cleanup() {
  if [ -n "${VALIDATOR:-}" ]; then kill "$VALIDATOR" 2>/dev/null || true; wait "$VALIDATOR" 2>/dev/null || true; fi
  rm -rf "$WORK"
}
# Runs a command, echoes its full output, and fails unless the output contains the needle.
# Capturing first keeps Rust CLIs from ever writing into a closed pipe.
expect() {
  local needle=$1 out
  shift
  out=$("$@")
  printf '%s\n' "$out"
  grep -qF -- "$needle" <<<"$out"
}
trap cleanup EXIT

"$ROOT/scripts/build-program.sh"
cargo build -p qubit --manifest-path "$ROOT/Cargo.toml"

solana-test-validator --reset --quiet --ledger "$LEDGER" \
  --bpf-program "$PROGRAM_ID" "$ROOT/target/deploy/qubit_vault.so" &
VALIDATOR=$!
for _ in $(seq 1 90); do
  solana cluster-version -u $URL >/dev/null 2>&1 && break
  kill -0 "$VALIDATOR" 2>/dev/null || { echo "validator exited" >&2; exit 1; }
  sleep 1
done
expect "active" solana feature status txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL -u $URL

solana-keygen new --no-bip39-passphrase --silent -o "$PAYER"
solana-keygen new --no-bip39-passphrase --silent -o "$HOT"
solana airdrop 10 "$(solana address -k "$PAYER")" -u $URL
solana airdrop 10 "$(solana address -k "$HOT")" -u $URL

$QUBIT --config-dir "$CONFIG_A" keygen | tee "$WORK/keygen.txt"
MNEMONIC=$(grep -E '^([a-z]+ ){23}[a-z]+$' "$WORK/keygen.txt")
$QUBIT --config-dir "$CONFIG_A" create
QUBIT_ADDR=$($QUBIT --config-dir "$CONFIG_A" address | awk '/^qubit:/ {print $2}')

solana transfer -u $URL --keypair "$PAYER" --allow-unfunded-recipient "$QUBIT_ADDR" 2
expect "SOL: 2" $QUBIT --config-dir "$CONFIG_A" balance
$QUBIT --config-dir "$CONFIG_A" send 0.5 --to "$(solana address -k "$HOT")"
expect "key index 1" $QUBIT --config-dir "$CONFIG_A" status

MINT=$(spl-token create-token -u $URL --fee-payer "$PAYER" --mint-authority "$PAYER" --decimals 6 --output json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["commandOutput"]["address"])')
spl-token create-account "$MINT" -u $URL --fee-payer "$PAYER" --owner "$(solana address -k "$HOT")"
spl-token mint "$MINT" 1000 -u $URL --fee-payer "$PAYER" --mint-authority "$PAYER" --recipient-owner "$(solana address -k "$HOT")"

$QUBIT --config-dir "$CONFIG_A" sweep --from "$HOT" --yes
expect "$MINT: 1000" $QUBIT --config-dir "$CONFIG_A" balance
$QUBIT --config-dir "$CONFIG_A" send 100 --to "$(solana address -k "$PAYER")" --mint "$MINT"
expect "key index 2" $QUBIT --config-dir "$CONFIG_A" status

expect "key index 2" $QUBIT --config-dir "$CONFIG_B" recover <<<"$MNEMONIC"
$QUBIT --config-dir "$CONFIG_B" send 0.1 --to "$(solana address -k "$PAYER")"
expect "key index 3" $QUBIT --config-dir "$CONFIG_B" status
echo "e2e ok"
