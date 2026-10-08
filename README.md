# QUBIT

A memecoin whose mint address is derived from the output of an IBM quantum
computer. The code was published here before the quantum job ran, and anyone
can check the result with nothing but Python.

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

After launch, the measurement bits are published here as
`quantum_results.json`. No installation is needed:

```bash
git clone https://github.com/humdonsolana/QUBIT && cd QUBIT
python3 src/verify.py quantum_results.json <MINT_ADDRESS> <COUNTER>
```

`verify.py` uses only the Python standard library, and its ed25519 code is
copied verbatim from RFC 8032. A `MATCH` proves the address comes from the
published bits. That the bits came from IBM is shown by the job ID and a
screen recording of the job.

## Run the pipeline

Requires Python 3.10 or newer and an IBM Quantum Platform API key.

```bash
pip install -r requirements.txt
export IBM_QUANTUM_TOKEN="..."
python3 src/run_quantum_job.py    # run the circuit on IBM hardware, save the bits to private/
python3 src/derive_keypair.py     # search the counter, write the mint keypair to private/
```

Tests: `python3 -m unittest discover -s tests -v`
