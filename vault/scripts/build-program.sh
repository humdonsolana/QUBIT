#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build-sbf --manifest-path programs/vault/Cargo.toml
ls -l target/deploy/qubit_vault.so
