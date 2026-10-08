use qubit_wots::{Hash, Sha256};

/// SHA-256 through the runtime syscall; only callable on-chain.
pub struct SyscallSha256;

impl Sha256 for SyscallSha256 {
    #[cfg(target_os = "solana")]
    fn hashv(parts: &[&[u8]]) -> Hash {
        let mut out = [0u8; 32];
        // SAFETY: `parts` is a live slice of `&[u8]` fat pointers, which is the
        // layout the syscall expects, and `out` is a writable 32-byte buffer.
        unsafe {
            pinocchio::syscalls::sol_sha256(
                parts.as_ptr().cast(),
                parts.len() as u64,
                out.as_mut_ptr(),
            );
        }
        out
    }

    #[cfg(not(target_os = "solana"))]
    fn hashv(_parts: &[&[u8]]) -> Hash {
        unimplemented!(
            "the SHA-256 syscall exists only on-chain; host tests run the ELF under Mollusk"
        )
    }
}
