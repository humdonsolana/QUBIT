#!/usr/bin/env python3
"""Public verifier: recompute the coin's mint address from the published quantum bits.

Zero dependencies: Python 3.8+ standard library only. Nothing to install and
nothing to trust except this file. The ed25519 code below is copied verbatim
from RFC 8032 section 6 (key-generation subset), so it can be diffed against
the standard line by line.

Derivation rule (fixed and published before the quantum job ran):
  1. bits    = the per-shot bitstrings concatenated in shot order (shot 0
               first), each exactly as returned by Qiskit's
               result[0].data.meas.get_bitstrings()
  2. entropy = the first 256 bits, packed MSB-first into 32 bytes
  3. seed    = SHA-256(entropy || n), with n an 8-byte big-endian counter
  4. key     = ed25519 keypair from the seed (RFC 8032, identical to
               Solana's Keypair::from_seed)
  5. CA      = base58(public key)
  n is the smallest counter, starting from 0, whose CA ends in "qubit" in
  any letter case. Base58 has no capital I, so the i is always lowercase.

Usage:
    python3 src/verify.py proof/quantum_results.json <MINT_ADDRESS> <COUNTER>

Prints MATCH (exit 0) or NO MATCH (exit 1). MATCH proves the address was
derived from the bits in this file and the given counter by the rule above.
Confirming that the counter is the smallest one takes a full search:
src/derive_keypair.py --print-only repeats it. Where the bits came from is
evidenced separately: the IBM job ID and the screen recording.
"""

from __future__ import annotations

import hashlib
import json
import sys

ENTROPY_BITS = 256
SUFFIX = "qubit"
B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

# ---------------------------------------------------------------------------
# RFC 8032 section 6 "Ed25519 Python Illustration" -- verbatim, keygen subset.
# https://www.rfc-editor.org/rfc/rfc8032#section-6
# ---------------------------------------------------------------------------

def sha512(s):
    return hashlib.sha512(s).digest()

# Base field Z_p
p = 2**255 - 19

def modp_inv(x):
    return pow(x, p-2, p)

# Curve constant
d = -121665 * modp_inv(121666) % p

# Points are represented as tuples (X, Y, Z, T) of extended
# coordinates, with x = X/Z, y = Y/Z, x*y = T/Z

def point_add(P, Q):
    A, B = (P[1]-P[0]) * (Q[1]-Q[0]) % p, (P[1]+P[0]) * (Q[1]+Q[0]) % p;
    C, D = 2 * P[3] * Q[3] * d % p, 2 * P[2] * Q[2] % p;
    E, F, G, H = B-A, D-C, D+C, B+A;
    return (E*F, G*H, F*G, E*H);

# Computes Q = s * Q
def point_mul(s, P):
    Q = (0, 1, 1, 0)  # Neutral element
    while s > 0:
        if s & 1:
            Q = point_add(Q, P)
        P = point_add(P, P)
        s >>= 1
    return Q

# Square root of -1
modp_sqrt_m1 = pow(2, (p-1) // 4, p)

# Compute corresponding x-coordinate, with low bit corresponding to
# sign, or return None on failure
def recover_x(y, sign):
    if y >= p:
        return None
    x2 = (y*y-1) * modp_inv(d*y*y+1)
    if x2 == 0:
        if sign:
            return None
        else:
            return 0

    # Compute square root of x2
    x = pow(x2, (p+3) // 8, p)
    if (x*x - x2) % p != 0:
        x = x * modp_sqrt_m1 % p
    if (x*x - x2) % p != 0:
        return None

    if (x & 1) != sign:
        x = p - x
    return x

# Base point
g_y = 4 * modp_inv(5) % p
g_x = recover_x(g_y, 0)
G = (g_x, g_y, 1, g_x * g_y % p)

def point_compress(P):
    zinv = modp_inv(P[2])
    x = P[0] * zinv % p
    y = P[1] * zinv % p
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")

def secret_expand(secret):
    if len(secret) != 32:
        raise Exception("Bad size of private key")
    h = sha512(secret)
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= (1 << 254)
    return (a, h[32:])

def secret_to_public(secret):
    (a, dummy) = secret_expand(secret)
    return point_compress(point_mul(a, G))

# ---------------------------------------------------------------------------
# End of RFC 8032 code.
# ---------------------------------------------------------------------------


def b58encode(data: bytes) -> str:
    """Base58 with the Bitcoin/Solana alphabet, no checksum."""
    n = int.from_bytes(data, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58_ALPHABET[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def bits_to_entropy(bitstrings: list[str]) -> bytes:
    """Concatenate shot bitstrings in order and pack the first 256 bits MSB-first into 32 bytes."""
    stream = "".join(bitstrings)
    if set(stream) - {"0", "1"}:
        raise ValueError("bitstrings may contain only '0' and '1'")
    if len(stream) < ENTROPY_BITS:
        raise ValueError(f"need {ENTROPY_BITS} bits, got {len(stream)}")
    return int(stream[:ENTROPY_BITS], 2).to_bytes(ENTROPY_BITS // 8, "big")


def counter_seed(entropy: bytes, counter: int) -> bytes:
    """The ed25519 seed for one counter value: SHA-256(entropy || counter as 8 bytes big-endian)."""
    return hashlib.sha256(entropy + counter.to_bytes(8, "big")).digest()


def mint_address(bitstrings: list[str], counter: int) -> str:
    """Apply the published rule for one counter value: bits -> entropy -> seed -> key -> address."""
    return b58encode(secret_to_public(counter_seed(bits_to_entropy(bitstrings), counter)))


def has_suffix(address: str) -> bool:
    """True if the address ends in SUFFIX, ignoring letter case."""
    return address[-len(SUFFIX):].lower() == SUFFIX


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print("Usage: python3 src/verify.py <quantum_results.json> <MINT_ADDRESS> <COUNTER>", file=sys.stderr)
        return 2

    results_file, expected, counter_arg = argv[1:]
    try:
        counter = int(counter_arg)
        counter.to_bytes(8, "big")
    except (ValueError, OverflowError):
        print("COUNTER must be an integer from 0 to 2^64 - 1", file=sys.stderr)
        return 2

    with open(results_file, "rb") as fh:
        raw = fh.read()
    payload = json.loads(raw)
    bitstrings = payload.get("bitstrings")
    if not isinstance(bitstrings, list) or not all(isinstance(b, str) for b in bitstrings):
        print(f"{results_file}: 'bitstrings' must be a list of strings", file=sys.stderr)
        return 2

    stream = "".join(bitstrings)
    derived = mint_address(bitstrings, counter)

    print(f"File sha256 : {hashlib.sha256(raw).hexdigest()}")
    print(f"IBM job ID  : {payload.get('job_id')}")
    print(f"Backend     : {payload.get('backend')}")
    print(f"Bits        : {len(bitstrings)} shots, {len(stream)} bits, {stream.count('1')} ones")
    print(f"Counter     : {counter}")
    print(f"Derived     : {derived}")
    print(f"Expected    : {expected}")
    if payload.get("dry_run"):
        print("WARNING     : DRY RUN file from a local simulator, NOT quantum hardware output")

    if not has_suffix(derived):
        print(f"RESULT      : NO MATCH (the address for this counter does not end in '{SUFFIX}')")
        return 1
    if derived == expected:
        print("RESULT      : MATCH (derived from the bits in this file and this counter)")
        return 0
    print("RESULT      : NO MATCH (these bits and this counter do not produce this address)")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
