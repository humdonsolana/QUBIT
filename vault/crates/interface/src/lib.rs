//! Byte layouts shared by the QUBIT vault program and its clients.
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

use alloc::vec::Vec;
use qubit_wots::{Hash, Sha256, PREFIX, SIGNATURE_LEN};

/// Program id (`3kQoxmBhrQztTBRVhqkGbMbUcpQpK64cbadrt2y9kbjX`).
pub const ID: [u8; 32] = [
    40, 214, 69, 52, 145, 35, 224, 190, 129, 104, 193, 5, 65, 26, 41, 13, 19, 0, 129, 151, 75, 196,
    2, 88, 26, 149, 207, 72, 126, 109, 203, 98,
];
/// Seed prefix of the vault PDA: `["vault", vault_id]`.
pub const VAULT_SEED: &[u8] = b"vault";
/// Seed prefix of the treasury PDA: `["treasury", vault]`.
pub const TREASURY_SEED: &[u8] = b"treasury";
/// Domain byte of the signed message digest.
pub const DOMAIN_MESSAGE: u8 = 0x03;
/// Domain byte of the inner-instruction digest.
pub const DOMAIN_INNER: u8 = 0x04;
/// Domain byte binding a vault id to its root and first key hash.
pub const DOMAIN_VAULT_ID: u8 = 0x08;
/// Size in bytes of a vault account.
pub const VAULT_LEN: usize = 80;
/// Discriminator of `CreateVault`.
pub const CREATE_VAULT: u8 = 0;
/// Discriminator of `Execute`.
pub const EXECUTE: u8 = 1;
/// Discriminator of `Recover`.
pub const RECOVER: u8 = 2;
/// Inner-account flag: the account signs the inner instruction.
pub const FLAG_SIGNER: u8 = 1;
/// Inner-account flag: the inner instruction may write the account.
pub const FLAG_WRITABLE: u8 = 2;

const VAULT_DISCRIMINATOR: u8 = 1;
const VAULT_VERSION: u8 = 1;

/// Vault account state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vault {
    /// Bump of the vault PDA.
    pub vault_bump: u8,
    /// Bump of the treasury PDA.
    pub treasury_bump: u8,
    /// Number of one-time keys consumed so far.
    pub key_index: u64,
    /// Compressed public key that the next spend must match.
    pub current_pk_hash: Hash,
    /// Secret-derived value that tweaks every hash chain of this vault.
    pub root: [u8; 32],
}

impl Vault {
    /// Parses account data; `None` for any other length, discriminator or version.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let data: &[u8; VAULT_LEN] = data.try_into().ok()?;
        if data[0] != VAULT_DISCRIMINATOR || data[1] != VAULT_VERSION {
            return None;
        }
        Some(Self {
            vault_bump: data[2],
            treasury_bump: data[3],
            key_index: u64::from_le_bytes(array(&data[8..16])?),
            current_pk_hash: array(&data[16..48])?,
            root: array(&data[48..80])?,
        })
    }

    /// Serializes to exactly [`VAULT_LEN`] bytes.
    pub fn to_bytes(&self) -> [u8; VAULT_LEN] {
        let mut out = [0u8; VAULT_LEN];
        out[0] = VAULT_DISCRIMINATOR;
        out[1] = VAULT_VERSION;
        out[2] = self.vault_bump;
        out[3] = self.treasury_bump;
        out[8..16].copy_from_slice(&self.key_index.to_le_bytes());
        out[16..48].copy_from_slice(&self.current_pk_hash);
        out[48..80].copy_from_slice(&self.root);
        out
    }
}

fn array<const L: usize>(slice: &[u8]) -> Option<[u8; L]> {
    slice.try_into().ok()
}

/// Inner instruction of `Execute`, with accounts given as indices into the outer instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inner<'a> {
    /// Index of the inner program's account.
    pub program_index: u8,
    accounts: &'a [u8],
    /// Inner instruction data.
    pub data: &'a [u8],
}

impl<'a> Inner<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        let (&program_index, rest) = bytes.split_first()?;
        let (&count, rest) = rest.split_first()?;
        let (accounts, rest) = rest.split_at_checked(usize::from(count) * 2)?;
        let (len, rest) = rest.split_first_chunk::<2>()?;
        let (data, rest) = rest.split_at_checked(usize::from(u16::from_le_bytes(*len)))?;
        rest.is_empty().then_some(Self {
            program_index,
            accounts,
            data,
        })
    }

    /// Number of inner accounts.
    pub fn account_count(&self) -> usize {
        self.accounts.len() / 2
    }

    /// Iterates `(outer_account_index, flags)` pairs.
    pub fn accounts(&self) -> impl Iterator<Item = (u8, u8)> + 'a {
        self.accounts.chunks_exact(2).map(|pair| (pair[0], pair[1]))
    }
}

/// A parsed program instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction<'a> {
    /// Create a vault whose address commits to `root` and `pk_hash`.
    CreateVault {
        /// Secret-derived value that tweaks every hash chain.
        root: &'a [u8; 32],
        /// Compressed public key of key index zero.
        pk_hash: &'a [u8; 32],
    },
    /// Verify a signature, rotate the key and run `inner` signed by the treasury.
    Execute {
        /// Compressed public key of the next key index.
        next_pk_hash: &'a [u8; 32],
        /// WOTS+ signature, [`SIGNATURE_LEN`] bytes.
        signature: &'a [u8],
        /// Instruction to invoke.
        inner: Inner<'a>,
    },
    /// Verify a signature and rotate the key without invoking anything.
    Recover {
        /// Compressed public key of the next key index.
        next_pk_hash: &'a [u8; 32],
        /// WOTS+ signature, [`SIGNATURE_LEN`] bytes.
        signature: &'a [u8],
        /// Digest the signature was made over.
        inner_digest: &'a [u8; 32],
    },
}

impl<'a> Instruction<'a> {
    /// Parses instruction data; `None` on any malformed input.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let (&tag, rest) = data.split_first()?;
        match tag {
            CREATE_VAULT => {
                let (root, rest) = rest.split_first_chunk::<32>()?;
                let (pk_hash, rest) = rest.split_first_chunk::<32>()?;
                rest.is_empty()
                    .then_some(Self::CreateVault { root, pk_hash })
            }
            EXECUTE => {
                let (next_pk_hash, rest) = rest.split_first_chunk::<32>()?;
                let (signature, rest) = rest.split_at_checked(SIGNATURE_LEN)?;
                let inner = Inner::parse(rest)?;
                Some(Self::Execute {
                    next_pk_hash,
                    signature,
                    inner,
                })
            }
            RECOVER => {
                let (next_pk_hash, rest) = rest.split_first_chunk::<32>()?;
                let (signature, rest) = rest.split_at_checked(SIGNATURE_LEN)?;
                let (inner_digest, rest) = rest.split_first_chunk::<32>()?;
                rest.is_empty().then_some(Self::Recover {
                    next_pk_hash,
                    signature,
                    inner_digest,
                })
            }
            _ => None,
        }
    }
}

/// Inner instruction to encode into `Execute`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InnerSpec<'a> {
    /// Index of the inner program's account in the outer instruction.
    pub program_index: u8,
    /// `(outer_account_index, flags)` pairs.
    pub accounts: &'a [(u8, u8)],
    /// Inner instruction data.
    pub data: &'a [u8],
}

/// Digest of a fully resolved inner instruction.
///
/// `None` when there are more than 255 accounts or more than 65535 data bytes.
pub fn inner_digest<H: Sha256>(
    program_id: &[u8; 32],
    accounts: &[([u8; 32], u8)],
    data: &[u8],
) -> Option<Hash> {
    let count = u8::try_from(accounts.len()).ok()?;
    let data_len = u16::try_from(data.len()).ok()?;
    let mut buf =
        Vec::with_capacity(PREFIX.len() + 1 + 32 + 1 + accounts.len() * 33 + 2 + data.len());
    buf.extend_from_slice(PREFIX);
    buf.push(DOMAIN_INNER);
    buf.extend_from_slice(program_id);
    buf.push(count);
    for (address, flags) in accounts {
        buf.extend_from_slice(address);
        buf.push(*flags);
    }
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.extend_from_slice(data);
    Some(H::hashv(&[&buf]))
}

/// Digest signed by the one-time key at `key_index`.
pub fn message_digest<H: Sha256>(
    vault: &[u8; 32],
    key_index: u64,
    next_pk_hash: &Hash,
    inner_digest: &Hash,
) -> Hash {
    H::hashv(&[
        PREFIX,
        &[DOMAIN_MESSAGE],
        vault,
        &key_index.to_le_bytes(),
        next_pk_hash,
        inner_digest,
    ])
}

/// Second seed of the vault PDA: commits the address to the root and the first key.
pub fn vault_id<H: Sha256>(root: &[u8; 32], pk_hash: &Hash) -> Hash {
    H::hashv(&[PREFIX, &[DOMAIN_VAULT_ID], root, pk_hash])
}

/// Encodes `CreateVault`.
pub fn encode_create_vault(root: &[u8; 32], pk_hash: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(65);
    out.push(CREATE_VAULT);
    out.extend_from_slice(root);
    out.extend_from_slice(pk_hash);
    out
}

/// Encodes `Execute`; `None` if `signature` has the wrong length or `inner` overflows its counters.
pub fn encode_execute(next_pk_hash: &Hash, signature: &[u8], inner: &InnerSpec) -> Option<Vec<u8>> {
    if signature.len() != SIGNATURE_LEN {
        return None;
    }
    let count = u8::try_from(inner.accounts.len()).ok()?;
    let data_len = u16::try_from(inner.data.len()).ok()?;
    let mut out = Vec::with_capacity(
        1 + 32 + SIGNATURE_LEN + 4 + inner.accounts.len() * 2 + inner.data.len(),
    );
    out.push(EXECUTE);
    out.extend_from_slice(next_pk_hash);
    out.extend_from_slice(signature);
    out.push(inner.program_index);
    out.push(count);
    for (index, flags) in inner.accounts {
        out.push(*index);
        out.push(*flags);
    }
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(inner.data);
    Some(out)
}

/// Encodes `Recover`; `None` if `signature` has the wrong length.
pub fn encode_recover(
    next_pk_hash: &Hash,
    signature: &[u8],
    inner_digest: &Hash,
) -> Option<Vec<u8>> {
    if signature.len() != SIGNATURE_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(1 + 32 + SIGNATURE_LEN + 32);
    out.push(RECOVER);
    out.extend_from_slice(next_pk_hash);
    out.extend_from_slice(signature);
    out.extend_from_slice(inner_digest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use qubit_wots::HostSha256 as Host;

    fn vault() -> Vault {
        Vault {
            vault_bump: 254,
            treasury_bump: 251,
            key_index: 9,
            current_pk_hash: [7; 32],
            root: [3; 32],
        }
    }

    // The bytes of `vault()`; web/tests/vault.test.mjs reads them from this file.
    const GOLDEN_VAULT: [u8; VAULT_LEN] = [
        1, 1, 254, 251, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
        7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
        3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    ];

    #[test]
    fn vault_round_trips() {
        assert_eq!(vault().to_bytes(), GOLDEN_VAULT);
        assert_eq!(Vault::from_bytes(&GOLDEN_VAULT), Some(vault()));
    }

    #[test]
    fn vault_rejects_other_layouts() {
        let mut bytes = vault().to_bytes();
        assert!(Vault::from_bytes(&bytes[..79]).is_none());
        bytes[0] = 2;
        assert!(Vault::from_bytes(&bytes).is_none());
        bytes[0] = 1;
        bytes[1] = 2;
        assert!(Vault::from_bytes(&bytes).is_none());
    }

    #[test]
    fn create_vault_round_trips() {
        let data = encode_create_vault(&[1; 32], &[2; 32]);
        assert_eq!(data.len(), 65);
        assert_eq!(
            Instruction::parse(&data),
            Some(Instruction::CreateVault {
                root: &[1; 32],
                pk_hash: &[2; 32]
            })
        );
        assert!(Instruction::parse(&data[..64]).is_none());
        let mut longer = data.clone();
        longer.push(0);
        assert!(Instruction::parse(&longer).is_none());
    }

    #[test]
    fn execute_round_trips() {
        let signature = vec![9u8; SIGNATURE_LEN];
        let spec = InnerSpec {
            program_index: 2,
            accounts: &[(1, FLAG_SIGNER | FLAG_WRITABLE), (3, FLAG_WRITABLE)],
            data: &[2, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0],
        };
        let data = encode_execute(&[4; 32], &signature, &spec).unwrap();
        match Instruction::parse(&data).unwrap() {
            Instruction::Execute {
                next_pk_hash,
                signature: parsed,
                inner,
            } => {
                assert_eq!(next_pk_hash, &[4; 32]);
                assert_eq!(parsed, signature.as_slice());
                assert_eq!(inner.program_index, 2);
                assert_eq!(inner.account_count(), 2);
                assert_eq!(inner.accounts().collect::<Vec<_>>(), vec![(1, 3), (3, 2)]);
                assert_eq!(inner.data, spec.data);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn execute_rejects_wrong_signature_length() {
        let spec = InnerSpec {
            program_index: 2,
            accounts: &[],
            data: &[],
        };
        assert!(encode_execute(&[0; 32], &[0; SIGNATURE_LEN - 1], &spec).is_none());
        assert!(encode_execute(&[0; 32], &[0; SIGNATURE_LEN + 1], &spec).is_none());
        let good = encode_execute(&[0; 32], &[0; SIGNATURE_LEN], &spec).unwrap();
        assert!(Instruction::parse(&good[..good.len() - 1]).is_none());
        let mut extra = good.clone();
        extra.push(0);
        assert!(Instruction::parse(&extra).is_none());
    }

    #[test]
    fn inner_rejects_truncated_accounts() {
        let mut data = vec![EXECUTE];
        data.extend_from_slice(&[0; 32]);
        data.extend_from_slice(&[0; SIGNATURE_LEN]);
        data.extend_from_slice(&[2, 3, 1, 2]);
        assert!(Instruction::parse(&data).is_none());
        data.truncate(1 + 32 + SIGNATURE_LEN);
        data.extend_from_slice(&[2, 0, 5, 0, 1, 2]);
        assert!(Instruction::parse(&data).is_none());
    }

    #[test]
    fn recover_round_trips() {
        let signature = vec![1u8; SIGNATURE_LEN];
        let data = encode_recover(&[2; 32], &signature, &[3; 32]).unwrap();
        assert_eq!(
            Instruction::parse(&data),
            Some(Instruction::Recover {
                next_pk_hash: &[2; 32],
                signature: &signature,
                inner_digest: &[3; 32]
            })
        );
        assert!(encode_recover(&[2; 32], &signature[1..], &[3; 32]).is_none());
    }

    #[test]
    fn unknown_tag_and_empty_input_fail() {
        assert!(Instruction::parse(&[]).is_none());
        assert!(Instruction::parse(&[3]).is_none());
    }

    #[test]
    fn digests_bind_every_field() {
        let base = inner_digest::<Host>(&[1; 32], &[([2; 32], FLAG_WRITABLE)], &[9]).unwrap();
        assert_ne!(
            base,
            inner_digest::<Host>(&[1; 32], &[([2; 32], FLAG_SIGNER)], &[9]).unwrap()
        );
        assert_ne!(
            base,
            inner_digest::<Host>(&[1; 32], &[([3; 32], FLAG_WRITABLE)], &[9]).unwrap()
        );
        assert_ne!(
            base,
            inner_digest::<Host>(&[1; 32], &[([2; 32], FLAG_WRITABLE)], &[8]).unwrap()
        );
        assert_ne!(
            base,
            inner_digest::<Host>(&[0; 32], &[([2; 32], FLAG_WRITABLE)], &[9]).unwrap()
        );
        assert!(inner_digest::<Host>(&[1; 32], &[], &vec![0; 65536]).is_none());
        let message = message_digest::<Host>(&[5; 32], 1, &[6; 32], &base);
        assert_ne!(
            message,
            message_digest::<Host>(&[5; 32], 2, &[6; 32], &base)
        );
        assert_ne!(
            message,
            message_digest::<Host>(&[5; 32], 1, &[7; 32], &base)
        );
        assert_ne!(
            message,
            message_digest::<Host>(&[4; 32], 1, &[6; 32], &base)
        );
    }
}
