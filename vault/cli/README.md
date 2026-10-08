# qubit

The command-line client.

```text
qubit keygen [--passphrase]      new 24-word seed, saved to ~/.config/qubit/seed.json (mode 0600)
qubit create                     create the vault on the selected cluster
qubit address                    print the qubit address
qubit balance                    SOL and token balances
qubit send <amount> --to <addr> [--mint <mint>] [--all]
qubit resume [--recover]         finish an interrupted send, or skip it
qubit sweep --from <keypair.json> [--yes]
qubit recover                    restore the seed from 24 words
qubit status                     cluster, vault state, pending operation
```

Global options: `--url mainnet|devnet|localnet|<rpc url>` (`QUBIT_URL`), `--config-dir`, `--fee-payer <keypair.json>` (default `~/.config/solana/id.json`), `--priority-fee <lamports>`.

Where things are: `keys.rs` derives keys from the seed, `tx.rs` builds and sends V1 transactions, `ix.rs` encodes instructions, `pending.rs` is the write-before-sign record, `rpc.rs` is a small JSON-RPC client, `commands/` has one file per command.
