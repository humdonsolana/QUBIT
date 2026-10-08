#!/usr/bin/env python3
"""Run the quantum randomness job on real IBM Quantum hardware.

Builds a 16-qubit circuit with a Hadamard gate on every qubit, measures all of
them for 16 shots on the least busy operational IBM backend (simulators are
refused) and saves the ordered per-shot bitstrings to
private/quantum_results.json. 16 qubits x 16 shots = 256 bits, exactly the
entropy the derivation uses.

The job ID is written to private/quantum_job_pending.json before waiting in
IBM's queue, so an interrupted run can be resumed with --fetch. No output file
is ever overwritten.

Everything in private/ stays private until the token's create transaction is
confirmed on-chain. Until then these bits are the mint's private key.

Usage:
    export IBM_QUANTUM_TOKEN="..."                   # or use a saved account
    python3 src/run_quantum_job.py                   # submit, wait, save
    python3 src/run_quantum_job.py --fetch JOB_ID    # resume or re-pull a submitted job
    python3 src/run_quantum_job.py --dry-run         # local simulator, pipeline test only
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timezone

import qiskit
import qiskit_ibm_runtime
from qiskit import QuantumCircuit
from qiskit.primitives import StatevectorSampler
from qiskit.transpiler import generate_preset_pass_manager
from qiskit_ibm_runtime import QiskitRuntimeService, SamplerV2

N_QUBITS = 16
N_SHOTS = 16
PRIVATE_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "private")
OUTPUT_FILE = os.path.join(PRIVATE_DIR, "quantum_results.json")
PENDING_FILE = "quantum_job_pending.json"
DRY_RUN_FILE = os.path.join(PRIVATE_DIR, "dryrun", "quantum_results.json")
CONCATENATION_RULE = (
    "bitstrings exactly as returned by result[0].data.meas.get_bitstrings(), "
    "concatenated in shot order (shot 0 first); entropy = first 256 bits packed MSB-first; "
    "seed and address rule in verify.py"
)
SOFTWARE = {"qiskit": qiskit.__version__, "qiskit_ibm_runtime": qiskit_ibm_runtime.__version__}


def get_service() -> QiskitRuntimeService:
    """Connect with IBM_QUANTUM_TOKEN if set, otherwise with the saved default account."""
    token = os.environ.get("IBM_QUANTUM_TOKEN")
    try:
        if token:
            return QiskitRuntimeService(channel="ibm_quantum_platform", token=token,
                                        instance=os.environ.get("IBM_QUANTUM_INSTANCE"))
        return QiskitRuntimeService()
    except Exception as exc:
        sys.exit(f"Could not connect to IBM Quantum: {exc}\n"
                 "Set IBM_QUANTUM_TOKEN or save an account with QiskitRuntimeService.save_account().")


def build_circuit(n_qubits: int) -> QuantumCircuit:
    circuit = QuantumCircuit(n_qubits)
    circuit.h(range(n_qubits))
    circuit.measure_all()
    return circuit


def require_real_hardware(backend) -> None:
    name = backend.name.lower()
    if getattr(backend.configuration(), "simulator", False) or any(w in name for w in ("simulator", "fake", "aer")):
        sys.exit(f"Refusing backend {backend.name}: not real quantum hardware.")


def pick_backend(service: QiskitRuntimeService, name: str | None):
    if name:
        backend = service.backend(name)
    else:
        backend = service.least_busy(min_num_qubits=N_QUBITS, operational=True, simulator=False)
    require_real_hardware(backend)
    return backend


def write_private(path: str, obj: dict) -> None:
    """Write JSON atomically with mode 0600. An existing file is never replaced."""
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = f"{path}.{os.getpid()}.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(obj, fh, indent=2)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        os.link(tmp, path)
    except FileExistsError:
        sys.exit(f"{path} already exists. Refusing to overwrite it.")
    finally:
        os.unlink(tmp)


def extract_bitstrings(result) -> list[str]:
    bitstrings = list(result[0].data.meas.get_bitstrings())
    if len(bitstrings) != N_SHOTS or any(len(b) != N_QUBITS or set(b) - {"0", "1"} for b in bitstrings):
        sys.exit(f"Unexpected result shape ({len(bitstrings)} shots). Nothing was saved.")
    return bitstrings


def build_payload(job, record: dict) -> dict:
    bitstrings = extract_bitstrings(job.result())
    try:
        timestamps = job.metrics().get("timestamps")
    except Exception:  # metadata is optional, the bits must still be saved
        timestamps = None
    return {
        "job_id": record["job_id"],
        "backend": record["backend"],
        "n_qubits": N_QUBITS,
        "n_shots": N_SHOTS,
        "submitted_utc": record.get("submitted_utc"),
        "ibm_timestamps": timestamps,
        "physical_qubits": record.get("physical_qubits"),
        "software": SOFTWARE,
        "dry_run": False,
        "concatenation_rule": CONCATENATION_RULE,
        "bitstrings": bitstrings,
    }


def report(payload: dict, out_path: str) -> None:
    print(f"Saved       : {os.path.relpath(out_path)}")
    for shot, bits in enumerate(payload["bitstrings"]):
        print(f"Shot {shot:<7}: {bits}")
    if payload["dry_run"]:
        print("DRY RUN     : local simulator output, never launch with this")
    else:
        print("PRIVATE     : do not publish until the create tx is confirmed on-chain")


def pending_path_for(out_path: str) -> str:
    return os.path.join(os.path.dirname(os.path.abspath(out_path)), PENDING_FILE)


def run_real(args: argparse.Namespace) -> None:
    pending_path = pending_path_for(args.out)
    if os.path.exists(args.out):
        sys.exit(f"{args.out} already exists. Results are never overwritten.")
    if os.path.exists(pending_path):
        with open(pending_path) as fh:
            job_id = json.load(fh)["job_id"]
        sys.exit(f"Job {job_id} was already submitted. Resume it with: "
                 f"python3 src/run_quantum_job.py --fetch {job_id}")

    service = get_service()
    backend = pick_backend(service, args.backend)
    print(f"Backend     : {backend.name} ({backend.num_qubits} qubits)")

    circuit = generate_preset_pass_manager(optimization_level=1, backend=backend).run(build_circuit(N_QUBITS))
    job = SamplerV2(mode=backend).run([circuit], shots=N_SHOTS)
    record = {
        "job_id": job.job_id(),
        "backend": backend.name,
        "n_qubits": N_QUBITS,
        "n_shots": N_SHOTS,
        "physical_qubits": circuit.layout.final_index_layout(),
        "submitted_utc": datetime.now(timezone.utc).isoformat(),
    }
    write_private(pending_path, record)
    print(f"Job ID      : {record['job_id']}")
    print(f"Waiting     : queued at IBM. If interrupted, run --fetch {record['job_id']}")

    payload = build_payload(job, record)
    write_private(args.out, payload)
    report(payload, args.out)


def fetch(args: argparse.Namespace) -> None:
    service = get_service()
    job = service.job(args.fetch)
    backend = job.backend()
    if backend is not None:
        require_real_hardware(backend)
    print(f"Job ID      : {job.job_id()}")
    print(f"Backend     : {backend.name if backend else 'not reported'}")
    print(f"Status      : {job.status()}")

    record = {}
    pending_path = pending_path_for(args.out)
    if os.path.exists(pending_path):
        with open(pending_path) as fh:
            pending = json.load(fh)
        if pending.get("job_id") == args.fetch:
            record = pending
    record.setdefault("job_id", args.fetch)
    record.setdefault("backend", backend.name if backend else None)
    if "submitted_utc" not in record and job.creation_date:
        record["submitted_utc"] = job.creation_date.isoformat()

    payload = build_payload(job, record)
    if os.path.exists(args.out):
        with open(args.out) as fh:
            saved = json.load(fh)
        if saved.get("job_id") == args.fetch and saved.get("bitstrings") == payload["bitstrings"]:
            print("Verified    : bits on disk match IBM's stored result exactly")
            report(saved, args.out)
            return
        sys.exit(f"{args.out} holds different results. Refusing to overwrite it.")
    write_private(args.out, payload)
    report(payload, args.out)


def dry_run(args: argparse.Namespace) -> None:
    if os.path.exists(args.out):
        sys.exit(f"{args.out} already exists. Results are never overwritten.")
    result = StatevectorSampler().run([build_circuit(N_QUBITS)], shots=N_SHOTS).result()
    payload = {
        "job_id": None,
        "backend": "LOCAL_SIMULATOR_DRY_RUN",
        "n_qubits": N_QUBITS,
        "n_shots": N_SHOTS,
        "submitted_utc": datetime.now(timezone.utc).isoformat(),
        "ibm_timestamps": None,
        "physical_qubits": None,
        "software": SOFTWARE,
        "dry_run": True,
        "concatenation_rule": CONCATENATION_RULE,
        "bitstrings": extract_bitstrings(result),
    }
    write_private(args.out, payload)
    report(payload, args.out)


def main() -> None:
    parser = argparse.ArgumentParser(description="Run the quantum randomness job on IBM Quantum hardware.")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--fetch", metavar="JOB_ID", help="resume or re-pull a submitted job")
    mode.add_argument("--dry-run", action="store_true", help="run on a local simulator (pipeline test only)")
    parser.add_argument("--backend", help="IBM backend name (default: least busy real device)")
    parser.add_argument("--out", help="output path (default: private/quantum_results.json, "
                                      "or private/dryrun/quantum_results.json with --dry-run)")
    args = parser.parse_args()

    if args.dry_run:
        args.out = args.out or DRY_RUN_FILE
        dry_run(args)
        return
    args.out = args.out or OUTPUT_FILE
    if args.fetch:
        fetch(args)
    else:
        run_real(args)


if __name__ == "__main__":
    main()
