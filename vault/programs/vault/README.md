# qubit-vault

The on-chain program. Three instructions, no admin.

| Instruction | What it does |
|---|---|
| `CreateVault` | Creates the vault at an address derived from the first key, and funds the treasury account. |
| `Execute` | Checks the signature for the current key, installs the lock for the next key, then lets the treasury sign the instruction you asked for (a transfer, a token transfer, a stake authorization, anything). |
| `Recover` | Same check and rotation without executing anything. For the rare case where a committed instruction can never succeed. |

```bash
scripts/build-program.sh        # cargo build-sbf → target/deploy/qubit_vault.so
cargo test -p qubit-vault      # tests run against the compiled program
```

Program id: `3kQoxmBhrQztTBRVhqkGbMbUcpQpK64cbadrt2y9kbjX` (not deployed yet).
