#!/usr/bin/env python3
"""QUBIT launchpad entropy pool.

One IBM Quantum job measures 16 qubits for many shots. Every 16 consecutive
shots form one slice of 16 x 16 = 256 bits, the same layout as the QUBIT coin.
Each launched token consumes one slice, and the grinder finds every slice's
counter ahead of time so launches are instant. Before any coin uses a job, the
job is sealed: one SHA-256 Merkle root over all of its slices, published so
every coin can prove its bits were fixed in advance. The rule is in RULE.md.

Unused slices are future mint private keys: the database and raw results stay
in private/launchpad/ (mode 0700) and only launched slices are ever revealed.

Usage:
    python3 launchpad/pool.py run-job --shots 100000     real IBM hardware
    python3 launchpad/pool.py fetch-job JOB_ID           resume an interrupted run-job
    python3 launchpad/pool.py dry-job --shots 4096       local simulator, test pool only
    python3 launchpad/pool.py seal JOB_ID                Merkle root over the job's slices; the API serves sealed jobs only
    python3 launchpad/pool.py grind --stock 20           keep 20 ready mints, runs until stopped
    python3 launchpad/pool.py status
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import sqlite3
import sys
import time
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "src"))

import verify  # noqa: E402

N_QUBITS = 16
SHOTS_PER_SLICE = 16
SUFFIX = "qbit"
PRIVATE_DIR = os.path.join(HERE, "..", "private", "launchpad")
DB_PATH = os.path.join(PRIVATE_DIR, "pool.db")
CHUNK = 50_000

SCHEMA = """
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY,
    backend TEXT NOT NULL,
    n_qubits INTEGER NOT NULL,
    shots INTEGER NOT NULL,
    dry_run INTEGER NOT NULL,
    submitted_utc TEXT NOT NULL,
    finished_utc TEXT,
    status TEXT NOT NULL,
    merkle_root TEXT,
    sealed_utc TEXT
);
CREATE TABLE IF NOT EXISTS slots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(job_id),
    slice_index INTEGER NOT NULL,
    bitstrings TEXT NOT NULL,
    counter INTEGER,
    mint TEXT,
    status TEXT NOT NULL DEFAULT 'new',
    reserved_at TEXT,
    reserved_for TEXT,
    launch_sig TEXT,
    launched_at TEXT,
    name TEXT,
    symbol TEXT,
    rewards TEXT,
    vault TEXT,
    uri TEXT,
    image TEXT,
    UNIQUE (job_id, slice_index)
);
CREATE INDEX IF NOT EXISTS slots_status ON slots (status, id);
"""

# Columns added after the first pool.db was created; connect() adds any that are missing.
LATE_SLOT_COLUMNS = ("rewards", "vault", "uri", "image")


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def connect() -> sqlite3.Connection:
    os.makedirs(PRIVATE_DIR, mode=0o700, exist_ok=True)
    fresh = not os.path.exists(DB_PATH)
    if fresh:
        os.close(os.open(DB_PATH, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600))
    db = sqlite3.connect(DB_PATH, timeout=30)
    db.execute("PRAGMA journal_mode=WAL")
    db.executescript(SCHEMA)
    add_missing_slot_columns(db)
    return db


def add_missing_slot_columns(db: sqlite3.Connection) -> None:
    """ALTER TABLE ADD COLUMN for databases made before the creator-rewards columns; a no-op once they exist."""
    have = {row[1] for row in db.execute("PRAGMA table_info(slots)")}
    for column in LATE_SLOT_COLUMNS:
        if column in have:
            continue
        try:
            db.execute(f"ALTER TABLE slots ADD COLUMN {column} TEXT")
        except sqlite3.OperationalError as error:
            if "duplicate column" not in str(error):  # another process added it first
                raise
    db.commit()


def save_results(job_id: str, bitstrings: list[str]) -> None:
    """Keep IBM's full per-shot output privately; it is the source of every slice."""
    path = os.path.join(PRIVATE_DIR, f"job_{job_id}.json")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as fh:
        json.dump({"job_id": job_id, "bitstrings": bitstrings}, fh)
        fh.flush()
        os.fsync(fh.fileno())


def add_slices(db: sqlite3.Connection, job_id: str, bitstrings: list[str]) -> int:
    if any(len(b) != N_QUBITS or set(b) - {"0", "1"} for b in bitstrings):
        raise SystemExit("Unexpected result shape. Nothing was stored.")
    count = len(bitstrings) // SHOTS_PER_SLICE
    db.executemany(
        "INSERT INTO slots (job_id, slice_index, bitstrings) VALUES (?, ?, ?)",
        ((job_id, k, json.dumps(bitstrings[k * SHOTS_PER_SLICE:(k + 1) * SHOTS_PER_SLICE])) for k in range(count)),
    )
    db.execute("UPDATE jobs SET status = 'stored', finished_utc = ? WHERE job_id = ?", (now(), job_id))
    db.commit()
    return count


def run_job(shots: int) -> None:
    import run_quantum_job as rqj
    from qiskit.transpiler import generate_preset_pass_manager
    from qiskit_ibm_runtime import SamplerV2

    db = connect()
    service = rqj.get_service()
    backend = rqj.pick_backend(service, None)
    circuit = generate_preset_pass_manager(optimization_level=1, backend=backend).run(rqj.build_circuit(N_QUBITS))
    job = SamplerV2(mode=backend).run([circuit], shots=shots)
    db.execute("INSERT INTO jobs (job_id, backend, n_qubits, shots, dry_run, submitted_utc, status) "
               "VALUES (?, ?, ?, ?, 0, ?, 'submitted')", (job.job_id(), backend.name, N_QUBITS, shots, now()))
    db.commit()
    print(f"Job ID      : {job.job_id()} on {backend.name}, {shots} shots. Resume with fetch-job if interrupted.")
    collect(db, job)


def fetch_job(job_id: str) -> None:
    import run_quantum_job as rqj

    db = connect()
    job = rqj.get_service().job(job_id)
    if not db.execute("SELECT 1 FROM jobs WHERE job_id = ?", (job_id,)).fetchone():
        raise SystemExit(f"{job_id} was not submitted by this pool.")
    collect(db, job)


def collect(db: sqlite3.Connection, job) -> None:
    bitstrings = list(job.result()[0].data.meas.get_bitstrings())
    save_results(job.job_id(), bitstrings)
    count = add_slices(db, job.job_id(), bitstrings)
    print(f"Stored      : {count} slices from {len(bitstrings)} shots. Quantum seconds used: {job.usage()}")


def dry_job(shots: int) -> None:
    from qiskit.primitives import StatevectorSampler
    import run_quantum_job as rqj

    db = connect()
    job_id = f"dryrun-{int(time.time())}"
    db.execute("INSERT INTO jobs (job_id, backend, n_qubits, shots, dry_run, submitted_utc, status) "
               "VALUES (?, 'LOCAL_SIMULATOR_DRY_RUN', ?, ?, 1, ?, 'submitted')", (job_id, N_QUBITS, shots, now()))
    db.commit()
    bitstrings = list(StatevectorSampler().run([rqj.build_circuit(N_QUBITS)], shots=shots).result()[0].data.meas.get_bitstrings())
    save_results(job_id, bitstrings)
    print(f"Stored      : {add_slices(db, job_id, bitstrings)} dry-run slices ({job_id}). Never launch these on mainnet.")


def seal(job_id: str) -> None:
    """Commit to every slice of a stored job with one SHA-256 Merkle root (rule in RULE.md).

    Sealing twice recomputes the root and refuses if the slices no longer match it."""
    import verify_token

    db = connect()
    job = db.execute("SELECT backend, shots, dry_run, status, merkle_root, sealed_utc FROM jobs WHERE job_id = ?",
                     (job_id,)).fetchone()
    if job is None:
        raise SystemExit(f"{job_id} is not in the pool.")
    backend, shots, dry, job_status, sealed_root, sealed_utc = job
    if job_status != "stored":
        raise SystemExit(f"{job_id} is {job_status}; only a stored job can be sealed.")
    rows = db.execute("SELECT slice_index, bitstrings FROM slots WHERE job_id = ? ORDER BY slice_index",
                      (job_id,)).fetchall()
    if not rows or [k for k, _ in rows] != list(range(shots // SHOTS_PER_SLICE)):
        raise SystemExit(f"{job_id} does not hold exactly slices 0 to {shots // SHOTS_PER_SLICE - 1}.")
    leaves = [verify_token.leaf_hash(verify.bits_to_entropy(json.loads(bits))) for _, bits in rows]
    root = verify_token.merkle_root(leaves).hex()
    if sealed_root is None:
        sealed_utc = now()
        db.execute("UPDATE jobs SET merkle_root = ?, sealed_utc = ? WHERE job_id = ? AND merkle_root IS NULL",
                   (root, sealed_utc, job_id))
        db.commit()
    elif sealed_root != root:
        raise SystemExit(f"{job_id}: the slices no longer match the sealed root {sealed_root}")
    print(json.dumps({job_id: {"backend": backend, "shots": shots, "slices": len(rows), "root": root,
                               "sealedAt": sealed_utc}}, indent=2))
    if dry:
        print("DRY RUN pool: test only, never publish this seal.", file=sys.stderr)


def scan(task: tuple[bytes, int, int]) -> int | None:
    """First counter in [start, start + size) whose mint ends exactly in SUFFIX, or None."""
    from solders.keypair import Keypair

    entropy, start, size = task
    for n in range(start, start + size):
        if str(Keypair.from_seed(verify.counter_seed(entropy, n)).pubkey()).endswith(SUFFIX):
            return n
    return None


def find_counter(entropy: bytes, pool: multiprocessing.pool.Pool, workers: int) -> int:
    """Smallest counter for SUFFIX. Rounds finish before the next starts, so the first hit is minimal."""
    start = 0
    while True:
        tasks = [(entropy, start + i * CHUNK, CHUNK) for i in range(workers * 4)]
        hits = [n for n in pool.map(scan, tasks) if n is not None]
        if hits:
            return min(hits)
        start += len(tasks) * CHUNK


def grind(stock: int, workers: int, dry: bool) -> None:
    from solders.keypair import Keypair

    db = connect()
    dry_flag = 1 if dry else 0
    with multiprocessing.Pool(workers) as pool:
        while True:
            ready = db.execute(
                "SELECT COUNT(*) FROM slots JOIN jobs USING (job_id) WHERE slots.status = 'ready' AND jobs.dry_run = ?",
                (dry_flag,)).fetchone()[0]
            row = db.execute(
                "SELECT slots.id, slots.bitstrings FROM slots JOIN jobs USING (job_id) "
                "WHERE slots.status = 'new' AND jobs.dry_run = ? ORDER BY slots.id LIMIT 1", (dry_flag,)).fetchone()
            if ready >= stock or row is None:
                time.sleep(20)
                continue
            slot_id, bitstrings = row
            entropy = verify.bits_to_entropy(json.loads(bitstrings))
            started = time.monotonic()
            counter = find_counter(entropy, pool, workers)
            seed = verify.counter_seed(entropy, counter)
            mint = str(Keypair.from_seed(seed).pubkey())
            if verify.b58encode(verify.secret_to_public(seed)) != mint or not mint.endswith(SUFFIX):
                raise SystemExit(f"slot {slot_id}: solders and RFC 8032 disagree")
            db.execute("UPDATE slots SET counter = ?, mint = ?, status = 'ready' WHERE id = ? AND status = 'new'",
                       (counter, mint, slot_id))
            db.commit()
            print(f"Ready       : slot {slot_id} -> {mint} (n={counter}, {time.monotonic() - started:.0f}s, "
                  f"{ready + 1}/{stock} in stock)", flush=True)


def status() -> None:
    db = connect()
    rows = db.execute(
        "SELECT jobs.job_id, jobs.backend, jobs.dry_run, slots.status, COUNT(*) FROM slots JOIN jobs USING (job_id) "
        "GROUP BY jobs.job_id, slots.status ORDER BY jobs.submitted_utc, slots.status").fetchall()
    for job_id, backend, dry, slot_status, count in rows:
        print(f"{job_id:24} {backend:24} {'DRY ' if dry else '    '}{slot_status:9} {count}")
    if not rows:
        print("Pool is empty.")
    for job_id, root in db.execute("SELECT job_id, merkle_root FROM jobs ORDER BY submitted_utc"):
        print(f"{job_id:24} {'sealed, root ' + root if root else 'NOT SEALED, the API does not serve it'}")


def main() -> None:
    parser = argparse.ArgumentParser(description="QUBIT launchpad entropy pool.")
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run-job")
    run.add_argument("--shots", type=int, required=True)
    fetch = sub.add_parser("fetch-job")
    fetch.add_argument("job_id")
    dry = sub.add_parser("dry-job")
    dry.add_argument("--shots", type=int, default=4096)
    sealer = sub.add_parser("seal")
    sealer.add_argument("job_id")
    g = sub.add_parser("grind")
    g.add_argument("--stock", type=int, default=20)
    g.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    g.add_argument("--dry", action="store_true", help="grind the local-simulator test pool")
    sub.add_parser("status")
    args = parser.parse_args()

    if args.command in ("run-job", "dry-job") and (args.shots < SHOTS_PER_SLICE or args.shots % SHOTS_PER_SLICE):
        raise SystemExit(f"--shots must be a positive multiple of {SHOTS_PER_SLICE}")
    if args.command == "run-job":
        run_job(args.shots)
    elif args.command == "fetch-job":
        fetch_job(args.job_id)
    elif args.command == "dry-job":
        dry_job(args.shots)
    elif args.command == "seal":
        seal(args.job_id)
    elif args.command == "grind":
        grind(args.stock, args.workers, args.dry)
    else:
        status()


if __name__ == "__main__":
    main()
