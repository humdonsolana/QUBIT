# QUBIT launchpad rule

Every coin launched on the QUBIT launchpad gets a mint address derived from
IBM quantum computer output, by the same method as the QUBIT coin.

1. **Job.** One IBM Quantum job measures 16 qubits, each after a Hadamard gate,
   over many shots on real hardware.
2. **Seal.** Before any coin uses the job, all of its slices are sealed under
   one SHA-256 Merkle root, published in `seals.json` and in a Solana memo
   transaction. From that moment the bits behind every future coin are fixed.
3. **Slice.** Shots `16k` to `16k + 15` form slice `k`: 16 bitstrings of 16
   bits, exactly as returned by `result[0].data.meas.get_bitstrings()`. Each
   coin uses one slice, and no slice is ever used twice.
4. **Entropy.** The slice's 256 bits, concatenated in shot order and packed
   most-significant-bit first into 32 bytes.
5. **Seed.** `SHA-256(entropy || n)`, where `n` is an 8-byte big-endian
   counter: the smallest one, starting from 0, whose address ends in `qbit`
   (lowercase).
6. **Mint.** The ed25519 keypair from the seed (RFC 8032, the same derivation
   as Solana's `Keypair::from_seed`); its base58 public key is the mint address.

## Sealed pool

Leaf `k` is `SHA-256(0x00 || entropy of slice k)`. The leaves are padded with
32 zero bytes up to the next power of two, and every node above them is
`SHA-256(0x01 || left || right)`. The top node is the pool root.

The launch transaction carries a memo with the job, the slice, the counter
and the pool root. After the coin exists, its token page publishes the slice's
bits and its Merkle path: the sibling hashes from the leaf up to the root,
where bit `i` of the slice number set means sibling `i` is on the left. The
sibling hashes reveal nothing about other slices. Unused slices stay private,
because each one is a future mint's private key.

Check a launched coin with nothing but Python. The verifier recomputes the
mint, walks the Merkle path up to the root and compares it with `seals.json`:

```bash
python3 launchpad/verify_token.py proof.json
```

## Rewards

The launchpad keeps no fees. Each launcher picks one of pump.fun's reward modes:

- **Creator rewards** (default): the fees go to the launcher's wallet.
- **Holder rewards**: the fees go to holders.
- **Creator rewards in a QUBIT Vault** (optional): the coin's pump.fun creator
  is the treasury of the launcher's QUBIT Vault, a program address derived from
  the seeds `"treasury"` and the vault address under the vault program
  `3kQoxmBhrQztTBRVhqkGbMbUcpQpK64cbadrt2y9kbjX`. It has no private key. Anyone
  can trigger fee collection, but the fees can only land in the vault, and
  taking them out takes the vault's one-time hash-based key (`qubit send`).

The launcher's wallet signs the launch transaction, so who launched each coin
is on chain. For a vault coin, the proof names the vault and its treasury, and
`verify_token.py` prints `fees locked in QUBIT Vault` only if the treasury
matches. On an explorer, the coin's pump.fun creator must be that treasury.
