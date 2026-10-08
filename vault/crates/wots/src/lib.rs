//! WOTS+ one-time signatures with the FIPS 205 parameters n = 32, w = 16.
//!
//! Every hash call is bound to a vault address and key index, so no two
//! chains anywhere in the system share a hash input.
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Hash output length in bytes.
pub const N: usize = 32;
/// Winternitz parameter.
pub const W: u8 = 16;
/// Number of message digits.
pub const LEN1: usize = 64;
/// Number of checksum digits.
pub const LEN2: usize = 3;
/// Number of hash chains.
pub const LEN: usize = LEN1 + LEN2;
/// Length in bytes of a serialized signature or public key.
pub const SIGNATURE_LEN: usize = LEN * N;
/// Prefix of every hash input in the QUBIT system.
pub const PREFIX: &[u8; 8] = b"QUBIT-v1";
/// Domain byte for hash-chain steps.
pub const DOMAIN_CHAIN: u8 = 0x01;
/// Domain byte for public-key compression.
pub const DOMAIN_PUBLIC_KEY: u8 = 0x02;
/// Domain byte for secret-key element derivation.
pub const DOMAIN_SECRET_KEY: u8 = 0x05;

/// A 32-byte hash value.
pub type Hash = [u8; N];
/// The chain elements of a secret key, public key or signature.
pub type Elements = [[u8; N]; LEN];

/// SHA-256 over the concatenation of `parts`.
pub trait Sha256 {
    /// Hashes the concatenation of all parts.
    fn hashv(parts: &[&[u8]]) -> Hash;
}

/// Public context that makes every hash unique to one vault and key index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tweak {
    /// Vault account address.
    pub vault: [u8; 32],
    /// Index of the one-time key, starting at zero.
    pub key_index: u64,
}

const CHAIN_INPUT_LEN: usize = PREFIX.len() + 1 + 32 + 8 + 1 + 1 + N;

fn chain_step<H: Sha256>(tweak: &Tweak, chain: u8, step: u8, value: &Hash) -> Hash {
    let mut input = [0u8; CHAIN_INPUT_LEN];
    input[..8].copy_from_slice(PREFIX);
    input[8] = DOMAIN_CHAIN;
    input[9..41].copy_from_slice(&tweak.vault);
    input[41..49].copy_from_slice(&tweak.key_index.to_le_bytes());
    input[49] = chain;
    input[50] = step;
    input[51..].copy_from_slice(value);
    H::hashv(&[&input])
}

/// Applies chain steps `start..start + count` to `value`.
pub fn chain<H: Sha256>(tweak: &Tweak, chain: u8, start: u8, count: u8, value: &Hash) -> Hash {
    let mut current = *value;
    for step in start..start + count {
        current = chain_step::<H>(tweak, chain, step, &current);
    }
    current
}

/// Splits a digest into 64 message digits followed by 3 checksum digits (FIPS 205 §5.1).
pub fn digits(message: &Hash) -> [u8; LEN] {
    let mut out = [0u8; LEN];
    let mut checksum: u16 = 0;
    for (i, byte) in message.iter().enumerate() {
        let high = byte >> 4;
        let low = byte & 0x0f;
        out[2 * i] = high;
        out[2 * i + 1] = low;
        checksum += u16::from(W - 1 - high) + u16::from(W - 1 - low);
    }
    out[LEN1] = ((checksum >> 8) & 0x0f) as u8;
    out[LEN1 + 1] = ((checksum >> 4) & 0x0f) as u8;
    out[LEN1 + 2] = (checksum & 0x0f) as u8;
    out
}

/// Derives the secret key for `tweak` from a 32-byte seed.
pub fn secret_key<H: Sha256>(seed: &[u8; 32], tweak: &Tweak, out: &mut Elements) {
    let key_index = tweak.key_index.to_le_bytes();
    for (chain, element) in out.iter_mut().enumerate() {
        *element = H::hashv(&[
            PREFIX,
            &[DOMAIN_SECRET_KEY],
            seed,
            &tweak.vault,
            &key_index,
            &[chain as u8],
        ]);
    }
}

/// Computes the public key of `secret`.
pub fn public_key<H: Sha256>(tweak: &Tweak, secret: &Elements, out: &mut Elements) {
    for (chain, (sk, pk)) in secret.iter().zip(out.iter_mut()).enumerate() {
        *pk = self::chain::<H>(tweak, chain as u8, 0, W - 1, sk);
    }
}

/// Compresses a public key to 32 bytes.
pub fn public_key_hash<H: Sha256>(tweak: &Tweak, public: &Elements) -> Hash {
    H::hashv(&[
        PREFIX,
        &[DOMAIN_PUBLIC_KEY],
        &tweak.vault,
        &tweak.key_index.to_le_bytes(),
        public.as_flattened(),
    ])
}

/// Signs a 32-byte digest. A secret key must sign exactly one digest, ever.
pub fn sign<H: Sha256>(tweak: &Tweak, secret: &Elements, message: &Hash, out: &mut Elements) {
    let digits = digits(message);
    for (chain, ((sk, sig), digit)) in secret.iter().zip(out.iter_mut()).zip(digits).enumerate() {
        *sig = self::chain::<H>(tweak, chain as u8, 0, digit, sk);
    }
}

/// Recomputes the public key from a signature over `message`.
///
/// Returns `None` when `signature` is not exactly [`SIGNATURE_LEN`] bytes.
pub fn public_key_from_signature<H: Sha256>(
    tweak: &Tweak,
    message: &Hash,
    signature: &[u8],
    out: &mut Elements,
) -> Option<()> {
    if signature.len() != SIGNATURE_LEN {
        return None;
    }
    let digits = digits(message);
    let mut rest = signature;
    for (chain, (pk, digit)) in out.iter_mut().zip(digits).enumerate() {
        let (element, tail) = rest.split_first_chunk::<N>()?;
        rest = tail;
        *pk = self::chain::<H>(tweak, chain as u8, digit, W - 1 - digit, element);
    }
    Some(())
}

/// Verifies `signature` over `message` against a compressed public key.
pub fn verify<H: Sha256>(tweak: &Tweak, expected: &Hash, message: &Hash, signature: &[u8]) -> bool {
    let mut public = [[0u8; N]; LEN];
    public_key_from_signature::<H>(tweak, message, signature, &mut public).is_some()
        && &public_key_hash::<H>(tweak, &public) == expected
}

/// SHA-256 for host-side code (CLI and tests); enabled by the `host` feature.
#[cfg(feature = "host")]
pub struct HostSha256;

#[cfg(feature = "host")]
impl Sha256 for HostSha256 {
    fn hashv(parts: &[&[u8]]) -> Hash {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        for part in parts {
            hasher.update(part);
        }
        hasher.finalize().into()
    }
}
