# qubit-wots

The signature scheme: WOTS+ one-time signatures over SHA-256 with the standard parameters (32-byte hashes, 67 chains, 2144-byte signatures).

- `no_std` and dependency-free. The hash function is plugged in through the `Sha256` trait, so the program uses the Solana syscall and host code uses `sha2` (enable the `host` feature).
- Every hash input is unique: it includes the vault's root, the key index, the chain and the position in the chain.
- `tests/vectors.json` comes from the independent Python reference in `tests/reference/`; the Rust code must reproduce it byte for byte.

A key signs exactly one message. The program enforces this by replacing the stored lock after every spend; the client enforces it by recording what it will sign before signing.
