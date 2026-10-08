# qubit-interface

Everything the program and its clients must agree on, in one place: the program id, the PDA seeds, the 80-byte vault account layout, the three instructions and their encodings, and the hashes that bind a vault to its first key and a signature to one specific action.

`no_std`; the only dependency is `qubit-wots`.
