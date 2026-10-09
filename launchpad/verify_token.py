#!/usr/bin/env python3
"""Verify a QUBIT launchpad token from its published proof, standard library only.

The proof (from the token page) holds the token's 16 bitstrings from the IBM
job, the counter, the mint and the slice's Merkle path. This recomputes the
mint with the rule in RULE.md, walks the path up to the pool root and compares
that root with the seal published in launchpad/seals.json before any coin
launched. The root must also equal the one in the launch transaction's memo.

If the proof says the creator rewards are locked in a QUBIT Vault, this also
recomputes the vault's treasury (the program address that must be the coin's
pump.fun creator).

Usage:
    python3 launchpad/verify_token.py proof.json
    python3 launchpad/verify_token.py https://<launchpad-api>/token/<MINT>
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "src"))

import verify  # noqa: E402

SUFFIX = "qbit"
SEALS_PATH = os.path.join(HERE, "seals.json")
VAULT_PROGRAM_ID = "3kQoxmBhrQztTBRVhqkGbMbUcpQpK64cbadrt2y9kbjX"


def leaf_hash(entropy: bytes) -> bytes:
    return hashlib.sha256(b"\x00" + entropy).digest()


def node_hash(left: bytes, right: bytes) -> bytes:
    return hashlib.sha256(b"\x01" + left + right).digest()


def merkle_root(leaves: list[bytes]) -> bytes:
    """Pool root: leaves padded with 32 zero bytes up to a power of two, then hashed in pairs to the top."""
    level = leaves + [bytes(32)] * ((1 << (len(leaves) - 1).bit_length()) - len(leaves))
    while len(level) > 1:
        level = [node_hash(level[i], level[i + 1]) for i in range(0, len(level), 2)]
    return level[0]


def root_from_path(leaf: bytes, index: int, path: list[bytes]) -> bytes:
    """Walk from a leaf to the root. Bit i of the slice number set means sibling i is on the left."""
    node = leaf
    for i, sibling in enumerate(path):
        node = node_hash(sibling, node) if index >> i & 1 else node_hash(node, sibling)
    return node


def b58decode(s: str) -> bytes:
    """Inverse of verify.b58encode: Bitcoin/Solana alphabet, no checksum. Each leading '1' is a leading zero byte."""
    n = 0
    for char in s:
        digit = verify.B58_ALPHABET.find(char)
        if digit < 0:
            raise ValueError(f"{char!r} is not a base58 character")
        n = n * 58 + digit
    return bytes(len(s) - len(s.lstrip("1"))) + n.to_bytes((n.bit_length() + 7) // 8, "big")


def is_on_curve(point: bytes) -> bool:
    """True if 32 bytes are the y coordinate of an ed25519 point, the test Solana applies to a program address.

    Solana runs dalek's decompress on the hash. It ignores the sign bit (that only picks which of the two x to use)
    and reduces y mod p, so this does the same around the RFC 8032 square root in src/verify.py.
    """
    if len(point) != 32:
        return False
    y = int.from_bytes(point, "little") & ((1 << 255) - 1)
    return verify.recover_x(y % verify.p, 0) is not None


def find_program_address(seeds: list[bytes], program_id: bytes) -> tuple[bytes, int]:
    """Solana's find_program_address: the first bump from 255 down for which
    SHA-256(seeds || bump || program id || "ProgramDerivedAddress") is not an ed25519 point."""
    if len(program_id) != 32 or len(seeds) > 15 or any(len(seed) > 32 for seed in seeds):
        raise ValueError("a program address takes a 32-byte program id and at most 15 seeds of at most 32 bytes")
    for bump in range(255, 0, -1):
        address = hashlib.sha256(b"".join(seeds) + bytes([bump]) + program_id + b"ProgramDerivedAddress").digest()
        if not is_on_curve(address):
            return address, bump
    raise ValueError("no bump seed gives an address off the curve")


def treasury_of(vault: str) -> str:
    """The vault's treasury: the program address of ["treasury", vault] under the vault program, with no private key."""
    key = b58decode(vault)
    if len(key) != 32:
        raise ValueError(f"{vault[:64]!r} is not a Solana address")
    address, _ = find_program_address([b"treasury", key], b58decode(VAULT_PROGRAM_ID))
    return verify.b58encode(address)


def load(source: str) -> dict:
    if source.startswith("https://"):
        with urllib.request.urlopen(source, timeout=30) as response:
            return json.load(response)
    with open(source) as fh:
        return json.load(fh)


def report_creator(creator: object) -> bool:
    """Print what the proof says about the creator rewards. False if it does not check out.

    A vault claim is checked here: its treasury must be the program address derived from its vault. That the coin's
    pump.fun creator is the treasury is an on-chain fact to read on an explorer.
    """
    if creator is None:
        return True
    if not isinstance(creator, dict):
        print("Rewards     : not verified, the creator entry is not an object")
        return False
    rewards = creator.get("rewards")
    print(f"Launcher    : {creator.get('wallet') or 'not recorded'}")
    if rewards is None:
        print("Rewards     : not recorded")
        return True
    if rewards == "creator":
        print("Rewards     : creator rewards, the fees go to the launcher's wallet")
        return True
    if rewards == "holders":
        print("Rewards     : holder rewards, the fees go to holders")
        return True
    if rewards != "vault":
        print(f"Rewards     : not verified, unknown rewards mode {rewards!r}")
        return False
    vault, treasury = creator.get("vault"), creator.get("treasury")
    try:
        expected = treasury_of(vault) if isinstance(vault, str) else None
    except ValueError:
        expected = None
    if expected is None:
        print(f"Rewards     : not verified, the proof has no valid vault address ({vault!r})")
        return False
    if treasury != expected:
        print(f"Rewards     : not verified, treasury {treasury!r} is not the program address of vault {vault}, "
              f"which is {expected}")
        return False
    print(f"Rewards     : fees locked in QUBIT Vault {vault}")
    print(f"Treasury    : {treasury} (the vault's program address, it has no private key)")
    print("Check       : on an explorer, the coin's pump.fun creator must be this treasury")
    return True


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("Usage: python3 launchpad/verify_token.py <proof.json | https://.../token/MINT>", file=sys.stderr)
        return 2
    proof = load(argv[1])
    entropy = verify.bits_to_entropy(proof["bitstrings"])
    derived = verify.b58encode(verify.secret_to_public(verify.counter_seed(entropy, int(proof["counter"]))))
    job, pool, index = proof["job"], proof["pool"], int(proof["slice"])
    slices, path = int(pool["slices"]), [bytes.fromhex(h) for h in pool["path"]]
    fits = 0 <= index < slices and len(path) == (slices - 1).bit_length()
    root = root_from_path(leaf_hash(entropy), index, path).hex() if fits else None
    seals = load(SEALS_PATH) if os.path.exists(SEALS_PATH) else {}
    seal = seals.get(job["id"])

    print(f"IBM job     : {job['id']} on {job['backend']}, slice {index} of {slices}")
    print(f"Counter     : {proof['counter']}")
    print(f"Derived     : {derived}")
    print(f"Mint        : {proof['mint']}")
    print(f"Pool root   : {root or 'none, the slice number or path length does not fit the pool'} "
          "(from the Merkle path, must equal the root in the launch memo)")
    if seal:
        print(f"Sealed root : {seal['root']} (launchpad/seals.json, seal transaction {seal.get('sealTx') or 'not recorded'})")
        print(f"Launch      : {proof.get('launchSignature')} (must come after the seal transaction)")
    else:
        print(f"Sealed root : none, job {job['id']} is not in launchpad/seals.json")
    creator_ok = report_creator(proof.get("creator"))
    if job.get("dryRun"):
        print("WARNING     : DRY RUN proof from a local simulator, not quantum hardware")

    failed = [name for name, ok in (
        ("mint", derived == proof["mint"] and derived.endswith(SUFFIX)),
        ("Merkle path", root is not None and root == pool["root"]),
        ("seal", seal is not None and seal["root"] == root and int(seal["slices"]) == slices),
        ("creator", creator_ok),
    ) if not ok]
    if failed:
        print(f"RESULT      : NO MATCH ({', '.join(failed)})")
        return 1
    print(f"RESULT      : MATCH (the mint comes from these bits, ends in {SUFFIX}, and the bits are slice {index} "
          "of the sealed pool)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
