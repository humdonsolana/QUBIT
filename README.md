# QUBIT

A memecoin whose mint address is derived from the output of an IBM quantum
computer. The code was published here before the quantum job ran, and anyone
can check the result with nothing but Python.

## Launch record

| | |
|---|---|
| Mint address | `ERkmk5rs8KKD9u2fxoDZUmwwuVc7rT3sSeowGTrQuBit` |
| Counter | `78704486` |
| IBM job | `db3vkeclf4us73c206fg` on `ibm_marrakesh` |
| Measurement bits | [`proof/quantum_results.json`](proof/quantum_results.json), SHA-256 `70f278b95b555fc6b6ea2637169228b81c47fa861afa5c4f3f0938658f802668` |
| Circuit as run | [`proof/circuit.qasm`](proof/circuit.qasm) |
| Launch transaction | [`2FDFF7GD…NyV2A`](https://solscan.io/tx/2FDFF7GDTWxLSmukVBwDM4pLPVt8XKxYDk4eub3xPxjqSHqvqMMeuXp3e4cRYDr3siCULWN8uNaPf7HSVVXNyV2A) |

Timeline, 2026-10-08 UTC:

| Time | Event |
|---|---|
| 20:13:20 | Derivation rule archived at commit `5fb8834` ([snapshot](https://web.archive.org/web/20261008201426/https://github.com/humdonsolana/QUBIT/tree/5fb88345bfa1da73bc764df687c2cde1e7ff05d5)) |
| 20:16:57 | Job submitted to IBM Quantum |
| 20:17:16 | Job finished on `ibm_marrakesh` |
| 22:13:20 | Token created. The transaction's memo records the job, the counter and the SHA-256 of the results file |
| After launch | Results published in `proof/` |

## How the address is derived

1. 16 qubits, a Hadamard gate on each, measured over 16 shots on real IBM
   Quantum hardware.
2. The 16 bitstrings, in shot order, give 256 bits of entropy.
3. `seed = SHA-256(entropy || n)`, where `n` is the smallest counter whose
   address ends in `qubit`, in any letter case.
4. The seed gives an ed25519 keypair (RFC 8032, the same derivation as
   Solana's `Keypair::from_seed`). Its public key is the mint address.

The exact rule is in the docstring of [`src/verify.py`](src/verify.py).

## Verify

No installation is needed:

```bash
git clone https://github.com/humdonsolana/QUBIT && cd QUBIT
python3 src/verify.py proof/quantum_results.json ERkmk5rs8KKD9u2fxoDZUmwwuVc7rT3sSeowGTrQuBit 78704486
sha256sum proof/quantum_results.json
```

`verify.py` uses only the Python standard library, and its ed25519 code is
copied verbatim from RFC 8032. A `MATCH` proves the address comes from the
published bits. The file's SHA-256 matches the memo written in the launch
transaction, so the bits were fixed before anyone could see them. That the
bits came from IBM is shown by the job ID and a screen recording of the job.

## Run the pipeline

Requires Python 3.10 or newer and an IBM Quantum Platform API key.

```bash
pip install -r requirements.txt
export IBM_QUANTUM_TOKEN="..."
python3 src/run_quantum_job.py    # run the circuit on IBM hardware, save the bits to private/
python3 src/derive_keypair.py     # search the counter, write the mint keypair to private/
```

Tests: `python3 -m unittest discover -s tests -v`
