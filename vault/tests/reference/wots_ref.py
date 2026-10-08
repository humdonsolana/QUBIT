#!/usr/bin/env python3
"""Reference WOTS+ (n = 32, w = 16) with QUBIT domain separation.

Independent of the Rust crate; generates and checks the shared test vectors.
"""
import argparse
import hashlib
import json
import sys

N = 32
W = 16
LEN1 = 64
LEN2 = 3
LEN = LEN1 + LEN2
PREFIX = b"QUBIT-v1"
DOMAIN_CHAIN = b"\x01"
DOMAIN_PUBLIC_KEY = b"\x02"
DOMAIN_SECRET_KEY = b"\x05"


def sha256(*parts):
    digest = hashlib.sha256()
    for part in parts:
        digest.update(part)
    return digest.digest()


def chain(vault, key_index, index, start, count, value):
    for step in range(start, start + count):
        value = sha256(PREFIX, DOMAIN_CHAIN, vault, key_index.to_bytes(8, "little"), bytes([index, step]), value)
    return value


def digits(message):
    out = []
    for byte in message:
        out.extend((byte >> 4, byte & 0x0F))
    checksum = sum(W - 1 - d for d in out)
    out.extend(((checksum >> 8) & 0x0F, (checksum >> 4) & 0x0F, checksum & 0x0F))
    return out


def secret_key(seed, vault, key_index):
    return [
        sha256(PREFIX, DOMAIN_SECRET_KEY, seed, vault, key_index.to_bytes(8, "little"), bytes([j]))
        for j in range(LEN)
    ]


def public_key(vault, key_index, secret):
    return [chain(vault, key_index, j, 0, W - 1, secret[j]) for j in range(LEN)]


def public_key_hash(vault, key_index, public):
    return sha256(PREFIX, DOMAIN_PUBLIC_KEY, vault, key_index.to_bytes(8, "little"), b"".join(public))


def sign(vault, key_index, secret, message):
    d = digits(message)
    return [chain(vault, key_index, j, 0, d[j], secret[j]) for j in range(LEN)]


def verify(vault, key_index, expected_hash, message, signature):
    d = digits(message)
    public = [chain(vault, key_index, j, d[j], W - 1 - d[j], signature[j]) for j in range(LEN)]
    return public_key_hash(vault, key_index, public) == expected_hash


def vector(label, seed, vault, key_index, message):
    secret = secret_key(seed, vault, key_index)
    public = public_key(vault, key_index, secret)
    signature = sign(vault, key_index, secret, message)
    assert verify(vault, key_index, public_key_hash(vault, key_index, public), message, signature)
    return {
        "label": label,
        "seed": seed.hex(),
        "vault": vault.hex(),
        "key_index": key_index,
        "message": message.hex(),
        "digits": digits(message),
        "secret_key": b"".join(secret).hex(),
        "public_key": b"".join(public).hex(),
        "public_key_hash": public_key_hash(vault, key_index, public).hex(),
        "signature": b"".join(signature).hex(),
    }


def generate():
    indexes = [0, 1, 2**32, 2**64 - 1]
    cases = [
        vector(f"random-{i}", sha256(b"seed", str(i).encode()), sha256(b"vault", str(i).encode()), indexes[i], sha256(b"message", str(i).encode()))
        for i in range(4)
    ]
    cases.append(vector("message-zero", sha256(b"seed-zero"), sha256(b"vault-zero"), 7, bytes(32)))
    cases.append(vector("message-ones", sha256(b"seed-ones"), sha256(b"vault-ones"), 8, bytes([0xFF]) * 32))
    return cases


def check(path):
    with open(path, encoding="utf-8") as handle:
        cases = json.load(handle)
    for case in cases:
        fresh = vector(case["label"], bytes.fromhex(case["seed"]), bytes.fromhex(case["vault"]), case["key_index"], bytes.fromhex(case["message"]))
        if fresh != case:
            print(f"mismatch: {case['label']}", file=sys.stderr)
            return 1
    print(f"{len(cases)} vectors ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--generate", metavar="PATH", help="write vectors to PATH")
    group.add_argument("--check", metavar="PATH", help="re-derive vectors from PATH and compare")
    args = parser.parse_args()
    if args.generate:
        with open(args.generate, "w", encoding="utf-8") as handle:
            json.dump(generate(), handle, indent=2)
            handle.write("\n")
        return 0
    return check(args.check)


if __name__ == "__main__":
    sys.exit(main())
