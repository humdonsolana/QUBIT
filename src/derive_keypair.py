#!/usr/bin/env python3
"""Derive the coin's Solana mint keypair from the quantum measurement bits.

Applies the published rule in verify.py: searches counters n = 0, 1, 2, ...
for the first seed SHA-256(entropy || n) whose address ends in "qubit" in any
letter case, then writes that keypair for the launcher in two formats:

    mint_keypair.json   64-byte Solana keypair, id.json format
    mint_keypair.b58    the same 64 bytes as one base58 string

The search uses every CPU core and needs about 41 million tries on average
(58^5 / 16). Both files are created next to the results file with mode 0600 and
are never overwritten. Nothing is written unless two independent ed25519
implementations agree on the public key: solders (Rust) and the RFC 8032
reference code in verify.py.

Keep both files private until the create transaction is confirmed on-chain.

Usage:
    python3 src/derive_keypair.py [RESULTS]                # default: private/quantum_results.json
    python3 src/derive_keypair.py RESULTS --print-only     # repeat the search, write nothing
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import sys
import time

from solders.keypair import Keypair

import verify

DEFAULT_RESULTS = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                               "private", "quantum_results.json")
KEYPAIR_JSON = "mint_keypair.json"
KEYPAIR_B58 = "mint_keypair.b58"
CHUNK = 50_000


def scan(task: tuple[bytes, str, int, int]) -> int | None:
    """Return the first counter in [start, start + size) whose address ends in suffix, any case."""
    entropy, suffix, start, size = task
    for n in range(start, start + size):
        if str(Keypair.from_seed(verify.counter_seed(entropy, n)).pubkey())[-len(suffix):].lower() == suffix:
            return n
    return None


def find_counter(entropy: bytes, suffix: str, workers: int | None = None, chunk: int = CHUNK) -> int:
    """Return the smallest counter whose address ends in suffix, ignoring letter case.

    Each round scans consecutive chunks in parallel and completes before the
    next round starts, so the lowest hit of the first round that has any hit
    is the smallest counter overall.
    """
    workers = workers or os.cpu_count() or 1
    tty = sys.stderr.isatty()
    started = time.monotonic()
    start = 0
    with multiprocessing.Pool(workers) as pool:
        while True:
            tasks = [(entropy, suffix, start + i * chunk, chunk) for i in range(workers * 4)]
            hits = [n for n in pool.map(scan, tasks) if n is not None]
            if hits:
                if start and tty:
                    print(file=sys.stderr)
                return min(hits)
            start += len(tasks) * chunk
            rate = start / (time.monotonic() - started)
            print(f"Searched    : {start:,} counters ({rate:,.0f}/s)",
                  end="\r" if tty else "\n", file=sys.stderr, flush=True)


def derive(bitstrings: list[str], suffix: str) -> tuple[int, Keypair]:
    """Find the counter and build the mint keypair, aborting unless solders and RFC 8032 agree."""
    entropy = verify.bits_to_entropy(bitstrings)
    counter = find_counter(entropy, suffix)
    seed = verify.counter_seed(entropy, counter)
    keypair = Keypair.from_seed(seed)
    if bytes(keypair.pubkey()) != verify.secret_to_public(seed):
        raise RuntimeError("solders and RFC 8032 disagree on the public key")
    return counter, keypair


def write_private(path: str, content: str) -> None:
    """Create path with mode 0600. Identical existing content is accepted, anything else aborts."""
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        with open(path) as fh:
            if fh.read() == content:
                return
        sys.exit(f"{path} already exists with different content. Refusing to overwrite a mint key.")
    with os.fdopen(fd, "w") as fh:
        fh.write(content)
        fh.flush()
        os.fsync(fh.fileno())


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description="Derive the mint keypair from the quantum results.")
    parser.add_argument("results", nargs="?", default=DEFAULT_RESULTS,
                        help="results file (default: private/quantum_results.json)")
    parser.add_argument("--print-only", action="store_true",
                        help="repeat the search and print the result without writing keypair files")
    parser.add_argument("--allow-dry-run", action="store_true",
                        help="accept local simulator output (testing only)")
    args = parser.parse_args(argv)

    try:
        with open(args.results) as fh:
            payload = json.load(fh)
    except FileNotFoundError:
        sys.exit(f"{args.results} not found. Run src/run_quantum_job.py first.")
    if payload.get("dry_run") and not args.allow_dry_run:
        sys.exit(f"{args.results} is a local simulator dry run. Never launch with it.")

    counter, keypair = derive(payload["bitstrings"], verify.SUFFIX)
    print(f"Quantum job : {payload.get('job_id')} on {payload.get('backend')}")
    print(f"Counter     : {counter}")
    print(f"Mint address: {keypair.pubkey()}")
    if args.print_only:
        return

    out_dir = os.path.dirname(os.path.abspath(args.results))
    json_path = os.path.join(out_dir, KEYPAIR_JSON)
    b58_path = os.path.join(out_dir, KEYPAIR_B58)
    write_private(json_path, json.dumps(list(bytes(keypair)), separators=(",", ":")))
    write_private(b58_path, str(keypair) + "\n")

    # Read both files back so a launcher loading either one gets this exact key.
    with open(json_path) as fh:
        from_json = Keypair.from_bytes(bytes(json.load(fh)))
    with open(b58_path) as fh:
        from_b58 = Keypair.from_base58_string(fh.read().strip())
    if not from_json.pubkey() == from_b58.pubkey() == keypair.pubkey():
        sys.exit("Keypair files do not reproduce the derived public key.")

    print(f"Keypair     : {os.path.relpath(json_path)}")
    print(f"              {os.path.relpath(b58_path)}")
    print("PRIVATE     : create the token first, publish the bits only after the create tx confirms")


if __name__ == "__main__":
    main()
