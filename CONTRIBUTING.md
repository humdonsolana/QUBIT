# Contributing

Keep it small and keep it verifiable.

- The mint proof in `src/`, `tests/` and `proof/` is a public record: never rewrite its history or the files the launch transaction points to.
- Account layouts, instruction encodings, hash inputs and PDA seeds are part of the vault protocol. Open an issue before changing any of them.
- The program and the shared crates are `no_std`, use no runtime dependencies beyond the Solana SDK, and contain no `unwrap`, `expect` or panicking indexing outside tests.
- Every change ships with a test at the right level, and the full flow must still pass on a local validator.

Before you push, from `vault/`:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/build-program.sh
cargo test --workspace
python3 -I tests/reference/wots_ref.py --check crates/wots/tests/vectors.json
scripts/e2e-localnet.sh
```

Commit messages follow Conventional Commits, one logical change per commit.
