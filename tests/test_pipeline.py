"""Tests for the derivation rule, the public verifier and the keypair writer.

Run from the repository root:
    python -m unittest discover -s tests -v

A real "qubit" search takes a minute or more, so most tests use a
one-character suffix or the precomputed vector below. Set QUBIT_SLOW_TESTS=1
to also run the real search through the command-line tools.
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SRC = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src")
sys.path.insert(0, SRC)

import verify  # noqa: E402

try:
    from solders.keypair import Keypair
    from solders.pubkey import Pubkey

    import derive_keypair
except ImportError:
    Keypair = Pubkey = derive_keypair = None

try:
    import qiskit  # noqa: F401
    HAVE_QISKIT = True
except ImportError:
    HAVE_QISKIT = False

SLOW = os.environ.get("QUBIT_SLOW_TESTS") == "1"

# RFC 8032 section 7.1, tests 1, 2, 3 and 1024: (secret key, public key).
RFC8032_VECTORS = [
    ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
     "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
    ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
     "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"),
    ("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
     "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025"),
    ("f5e5767cf153319517630f226876b86c8160cc583bc013744c6bf255f5cc0ee5",
     "278117fc144c72340f67d0f2316e8386ceffbf2b2428c9c51fef7c597f1d426e"),
]

# Known answer for the published rule. The bits are SHA-256(b"QUBIT test vector")
# split into 16 shots of 16 bits; the counter was found by derive_keypair.find_counter.
VECTOR_BITS = [
    "0001000100001000", "0000100001010101", "0011110001011100", "1101001001101101",
    "0000101001110100", "1100000001110101", "1010001110101000", "1100101111111111",
    "1111111100101000", "0001011001000101", "0111100000101101", "1101000100100001",
    "0110010111111000", "0110111100001000", "0000011000111001", "1000001011111010",
]
VECTOR_COUNTER = 4054187
VECTOR_ADDRESS = "6sZ2anWNYntHZ3WVWQrbQmNyLAMfF3hTEgKT7CtQubit"


def random_bitstrings(shots: int = 16, width: int = 16) -> list[str]:
    return ["".join(secrets.choice("01") for _ in range(width)) for _ in range(shots)]


def write_results(path: str, bitstrings: list[str], dry_run: bool = False) -> None:
    with open(path, "w") as fh:
        json.dump({"job_id": "test-job", "backend": "test-backend", "dry_run": dry_run,
                   "bitstrings": bitstrings}, fh)


def first_counter(entropy: bytes, suffix: str) -> int:
    """Sequential reference search, used to check the parallel one."""
    n = 0
    while str(Keypair.from_seed(verify.counter_seed(entropy, n)).pubkey())[-len(suffix):].lower() != suffix:
        n += 1
    return n


class Ed25519Test(unittest.TestCase):
    def test_rfc8032_vectors(self):
        for secret, public in RFC8032_VECTORS:
            with self.subTest(secret=secret):
                self.assertEqual(verify.secret_to_public(bytes.fromhex(secret)).hex(), public)

    @unittest.skipIf(Keypair is None, "solders not installed")
    def test_matches_solders_on_random_seeds(self):
        for _ in range(2000):
            seed = secrets.token_bytes(32)
            self.assertEqual(verify.secret_to_public(seed), bytes(Keypair.from_seed(seed).pubkey()))


class Base58Test(unittest.TestCase):
    def test_all_zero_key_is_system_program_id(self):
        self.assertEqual(verify.b58encode(bytes(32)), "1" * 32)

    @unittest.skipIf(Pubkey is None, "solders not installed")
    def test_matches_solders(self):
        samples = [secrets.token_bytes(32) for _ in range(500)]
        samples += [bytes(n) + secrets.token_bytes(32 - n) for n in range(1, 4)]
        for raw in samples:
            self.assertEqual(verify.b58encode(raw), str(Pubkey(raw)))


class EntropyTest(unittest.TestCase):
    def test_shot_order_and_msb_first_packing(self):
        shots = ["0" * 16] * 16
        shots[0] = "1" + "0" * 15   # first bit of the stream -> top bit of byte 0
        shots[15] = "0" * 15 + "1"  # bit 255 -> low bit of byte 31
        self.assertEqual(verify.bits_to_entropy(shots), b"\x80" + bytes(30) + b"\x01")

    def test_matches_bytewise_packing(self):
        for _ in range(200):
            stream = "".join(random_bitstrings())
            expected = bytes(int(stream[i:i + 8], 2) for i in range(0, 256, 8))
            self.assertEqual(verify.bits_to_entropy([stream]), expected)

    def test_only_first_256_bits_are_used(self):
        shots = random_bitstrings()
        self.assertEqual(verify.bits_to_entropy(shots + ["1" * 16]), verify.bits_to_entropy(shots))

    def test_rejects_fewer_than_256_bits(self):
        with self.assertRaises(ValueError):
            verify.bits_to_entropy(["0" * 16] * 15)

    def test_rejects_non_binary_characters(self):
        with self.assertRaises(ValueError):
            verify.bits_to_entropy(["01" * 8] * 15 + ["2" * 16])


class SuffixTest(unittest.TestCase):
    def test_any_letter_case_matches(self):
        for address in ("abcQubit", "abcQUBiT", "abcqubit", "abcqUbit"):
            with self.subTest(address=address):
                self.assertTrue(verify.has_suffix(address))

    def test_other_endings_do_not_match(self):
        for address in ("abcqbit", "abcqubi", "abcqubitx", "abcquibt"):
            with self.subTest(address=address):
                self.assertFalse(verify.has_suffix(address))


class KnownAnswerTest(unittest.TestCase):
    def test_vector_bits_are_the_label_hash(self):
        self.assertEqual(verify.bits_to_entropy(VECTOR_BITS), hashlib.sha256(b"QUBIT test vector").digest())

    def test_vector_address(self):
        self.assertEqual(verify.mint_address(VECTOR_BITS, VECTOR_COUNTER), VECTOR_ADDRESS)
        self.assertTrue(verify.has_suffix(VECTOR_ADDRESS))

    def test_counter_seed_layout(self):
        entropy = verify.bits_to_entropy(VECTOR_BITS)
        self.assertEqual(verify.counter_seed(entropy, 1),
                         hashlib.sha256(entropy + b"\x00\x00\x00\x00\x00\x00\x00\x01").digest())

    @unittest.skipIf(Keypair is None, "solders not installed")
    def test_solders_agrees(self):
        seed = verify.counter_seed(verify.bits_to_entropy(VECTOR_BITS), VECTOR_COUNTER)
        self.assertEqual(str(Keypair.from_seed(seed).pubkey()), VECTOR_ADDRESS)


@unittest.skipIf(derive_keypair is None, "solders not installed")
class CounterSearchTest(unittest.TestCase):
    def test_parallel_search_returns_the_smallest_counter(self):
        # Tiny chunks force many rounds and chunk boundaries.
        for suffix in ["q"] * 10 + ["qb"] * 2:
            entropy = secrets.token_bytes(32)
            with self.subTest(suffix=suffix), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(derive_keypair.find_counter(entropy, suffix, workers=2, chunk=7),
                                 first_counter(entropy, suffix))


class QiskitBitOrderTest(unittest.TestCase):
    @unittest.skipUnless(HAVE_QISKIT, "qiskit not installed")
    def test_qubit_zero_is_the_rightmost_character(self):
        from qiskit import QuantumCircuit
        from qiskit.primitives import StatevectorSampler

        circuit = QuantumCircuit(16)
        circuit.x(0)
        circuit.measure_all()
        result = StatevectorSampler().run([circuit], shots=4).result()
        self.assertEqual(set(result[0].data.meas.get_bitstrings()), {"0" * 15 + "1"})


class VerifyCliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.results = os.path.join(self.tmp, "quantum_results.json")
        write_results(self.results, VECTOR_BITS)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def run_verify(self, *args: str, flags: tuple[str, ...] = ()) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, *flags, os.path.join(SRC, "verify.py"), *args],
                              capture_output=True, text=True)

    def test_match(self):
        proc = self.run_verify(self.results, VECTOR_ADDRESS, str(VECTOR_COUNTER))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("RESULT      : MATCH", proc.stdout)

    def test_wrong_counter(self):
        proc = self.run_verify(self.results, VECTOR_ADDRESS, str(VECTOR_COUNTER + 1))
        self.assertEqual(proc.returncode, 1)
        self.assertIn("does not end in 'qubit'", proc.stdout)

    def test_wrong_address(self):
        proc = self.run_verify(self.results, "1" * 32, str(VECTOR_COUNTER))
        self.assertEqual(proc.returncode, 1)
        self.assertIn("NO MATCH", proc.stdout)

    def test_usage_error(self):
        self.assertEqual(self.run_verify(self.results, VECTOR_ADDRESS).returncode, 2)

    def test_rejects_invalid_counters(self):
        for bad in ("-1", "abc", str(2**64)):
            with self.subTest(counter=bad):
                self.assertEqual(self.run_verify(self.results, VECTOR_ADDRESS, bad).returncode, 2)

    def test_runs_without_site_packages(self):
        # -I -S: isolated mode, no site-packages. Proves the verifier is stdlib-only.
        proc = self.run_verify(self.results, VECTOR_ADDRESS, str(VECTOR_COUNTER), flags=("-I", "-S"))
        self.assertEqual(proc.returncode, 0, proc.stderr)

    def test_flags_dry_run_files(self):
        write_results(self.results, VECTOR_BITS, dry_run=True)
        self.assertIn("DRY RUN", self.run_verify(self.results, VECTOR_ADDRESS, str(VECTOR_COUNTER)).stdout)


@unittest.skipIf(derive_keypair is None, "solders not installed")
class DeriveKeypairTest(unittest.TestCase):
    """Runs derive_keypair with a one-character suffix so each search takes milliseconds."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.results = os.path.join(self.tmp, "quantum_results.json")
        self.bits = random_bitstrings()
        write_results(self.results, self.bits)
        counter = first_counter(verify.bits_to_entropy(self.bits), "q")
        self.address = verify.mint_address(self.bits, counter)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def derive(self, *args: str) -> str:
        out = io.StringIO()
        with mock.patch.object(verify, "SUFFIX", "q"), contextlib.redirect_stdout(out), \
                contextlib.redirect_stderr(io.StringIO()):
            derive_keypair.main([self.results, *args])
        return out.getvalue()

    def path(self, name: str) -> str:
        return os.path.join(self.tmp, name)

    def test_writes_both_formats_with_mode_600(self):
        self.assertIn(self.address, self.derive())
        for name in ("mint_keypair.json", "mint_keypair.b58"):
            self.assertEqual(stat.S_IMODE(os.stat(self.path(name)).st_mode), 0o600)
        with open(self.path("mint_keypair.json")) as fh:
            self.assertEqual(str(Keypair.from_bytes(bytes(json.load(fh))).pubkey()), self.address)
        with open(self.path("mint_keypair.b58")) as fh:
            self.assertEqual(str(Keypair.from_base58_string(fh.read().strip()).pubkey()), self.address)

    def test_print_only_writes_nothing(self):
        self.assertIn(self.address, self.derive("--print-only"))
        self.assertFalse(os.path.exists(self.path("mint_keypair.json")))
        self.assertFalse(os.path.exists(self.path("mint_keypair.b58")))

    def test_rerun_with_same_bits_is_a_no_op(self):
        self.derive()
        self.assertIn(self.address, self.derive())

    def test_refuses_to_overwrite_a_different_key(self):
        self.derive()
        write_results(self.results, random_bitstrings())
        with self.assertRaises(SystemExit) as ctx:
            self.derive()
        self.assertIn("Refusing to overwrite", str(ctx.exception.code))

    def test_refuses_dry_run_results_without_flag(self):
        write_results(self.results, self.bits, dry_run=True)
        with self.assertRaises(SystemExit):
            self.derive()
        self.assertIn(self.address, self.derive("--allow-dry-run"))

    @unittest.skipUnless(shutil.which("solana-keygen"), "solana-keygen not installed")
    def test_solana_cli_reads_the_keypair(self):
        self.derive()
        out = subprocess.run(["solana-keygen", "pubkey", self.path("mint_keypair.json")],
                             capture_output=True, text=True, check=True).stdout.strip()
        self.assertEqual(out, self.address)


@unittest.skipUnless(HAVE_QISKIT and derive_keypair is not None, "qiskit and solders required")
class DryRunPipelineTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.results = os.path.join(self.tmp, "quantum_results.json")

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_simulated_job_to_verifier_match(self):
        job = [sys.executable, os.path.join(SRC, "run_quantum_job.py"), "--dry-run", "--out", self.results]
        proc = subprocess.run(job, capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(stat.S_IMODE(os.stat(self.results).st_mode), 0o600)
        self.assertNotEqual(subprocess.run(job, capture_output=True).returncode, 0,
                            "results must never be overwritten")

        with open(self.results) as fh:
            payload = json.load(fh)
        self.assertTrue(payload["dry_run"])
        self.assertEqual(len(payload["bitstrings"]), 16)
        self.assertTrue(all(len(b) == 16 for b in payload["bitstrings"]))

        out = io.StringIO()
        with mock.patch.object(verify, "SUFFIX", "q"), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                derive_keypair.main([self.results])
            with contextlib.redirect_stdout(out):
                derive_keypair.main([self.results, "--allow-dry-run"])
            counter = re.search(r"Counter     : (\d+)", out.getvalue()).group(1)
            address = re.search(r"Mint address: (\S+)", out.getvalue()).group(1)
            report = io.StringIO()
            with contextlib.redirect_stdout(report):
                code = verify.main(["verify.py", self.results, address, counter])
        self.assertEqual(code, 0, report.getvalue())
        self.assertIn("DRY RUN", report.getvalue())


@unittest.skipUnless(SLOW and derive_keypair is not None, "set QUBIT_SLOW_TESTS=1 to run the full search")
class FullSearchTest(unittest.TestCase):
    """Real 'qubit' search through the command-line tools, about 70 seconds on 4 cores."""

    def test_cli_pipeline_on_known_vector(self):
        with tempfile.TemporaryDirectory() as tmp:
            results = os.path.join(tmp, "quantum_results.json")
            write_results(results, VECTOR_BITS)
            proc = subprocess.run([sys.executable, os.path.join(SRC, "derive_keypair.py"), results],
                                  capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn(f"Counter     : {VECTOR_COUNTER}", proc.stdout)
            self.assertIn(VECTOR_ADDRESS, proc.stdout)
            proc = subprocess.run([sys.executable, os.path.join(SRC, "verify.py"), results,
                                   VECTOR_ADDRESS, str(VECTOR_COUNTER)], capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stdout)


if __name__ == "__main__":
    unittest.main()
