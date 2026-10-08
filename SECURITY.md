# Security

The QUBIT Vault holds funds, so please report problems privately first.

**Reporting.** Open a private security advisory on this repository or contact the maintainer directly. Include the commit, the cluster and a way to reproduce. You will hear back within 72 hours.

**In scope.** Moving funds without a valid signature for the current key, getting two signatures accepted for one key, rotating the key without a signature, creating a vault at an address that does not commit to its first key, any difference between the Rust signature code and the Python reference, and any client path that could sign two different messages with one key.

**Out of scope.** Loss of the seed words, a break of SHA-256, validator-level attacks on Solana, and malware on the signing machine.

**Status.** The vault program is not deployed yet and has not been audited. Do not store more on any deployment than you can afford to lose.
