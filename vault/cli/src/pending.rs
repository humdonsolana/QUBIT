use crate::{
    config::{self, Paths},
    rpc::parse_address,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use std::fs;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StoredAccount {
    pub address: String,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StoredInstruction {
    pub program: String,
    pub accounts: Vec<StoredAccount>,
    pub data: String,
}

impl StoredInstruction {
    pub fn from_instruction(instruction: &Instruction) -> Self {
        Self {
            program: instruction.program_id.to_string(),
            accounts: instruction
                .accounts
                .iter()
                .map(|meta| StoredAccount {
                    address: meta.pubkey.to_string(),
                    signer: meta.is_signer,
                    writable: meta.is_writable,
                })
                .collect(),
            data: STANDARD.encode(&instruction.data),
        }
    }

    pub fn to_instruction(&self) -> Result<Instruction> {
        Ok(Instruction {
            program_id: parse_address(&self.program)?,
            accounts: self
                .accounts
                .iter()
                .map(|account| {
                    Ok(AccountMeta {
                        pubkey: parse_address(&account.address)?,
                        is_signer: account.signer,
                        is_writable: account.writable,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            data: STANDARD
                .decode(&self.data)
                .context("pending instruction data is not base64")?,
        })
    }
}

/// A one-time key committed to one message; persisted before the first broadcast.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub key_index: u64,
    pub next_pk_hash: [u8; 32],
    pub prelude: Vec<StoredInstruction>,
    pub inner: StoredInstruction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The chain already consumed this key index; the pending file is stale.
    Clear,
    /// The chain is at this key index; only this message may be signed.
    Resume,
    /// The chain is behind the pending index; the file does not belong to this vault state.
    Corrupt,
}

pub fn resolve(pending_index: u64, on_chain_index: u64) -> Resolution {
    match on_chain_index.cmp(&pending_index) {
        std::cmp::Ordering::Greater => Resolution::Clear,
        std::cmp::Ordering::Equal => Resolution::Resume,
        std::cmp::Ordering::Less => Resolution::Corrupt,
    }
}

pub fn load(paths: &Paths, vault: &Address) -> Result<Option<Pending>> {
    let path = paths.pending(vault);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| format!("{} is not a valid pending file", path.display()))
}

pub fn store(paths: &Paths, vault: &Address, pending: &Pending) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(pending)?;
    bytes.push(b'\n');
    config::write_new(&paths.pending(vault), &bytes)
}

pub fn clear(paths: &Paths, vault: &Address) -> Result<()> {
    let path = paths.pending(vault);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ix;

    #[test]
    fn resolution_table() {
        assert_eq!(resolve(5, 6), Resolution::Clear);
        assert_eq!(resolve(5, 9), Resolution::Clear);
        assert_eq!(resolve(5, 5), Resolution::Resume);
        assert_eq!(resolve(5, 4), Resolution::Corrupt);
    }

    #[test]
    fn instruction_round_trips_through_json() {
        let inner = ix::system_transfer(
            &Address::new_from_array([1; 32]),
            &Address::new_from_array([2; 32]),
            77,
        );
        let stored = StoredInstruction::from_instruction(&inner);
        assert_eq!(stored.to_instruction().unwrap(), inner);
    }

    #[test]
    fn store_load_clear() {
        let dir = std::env::temp_dir().join(format!("qubit-pending-{}", std::process::id()));
        let paths = Paths { dir: dir.clone() };
        let vault = Address::new_from_array([3; 32]);
        let inner = ix::system_transfer(
            &Address::new_from_array([1; 32]),
            &Address::new_from_array([2; 32]),
            1,
        );
        let pending = Pending {
            key_index: 4,
            next_pk_hash: [9; 32],
            prelude: Vec::new(),
            inner: StoredInstruction::from_instruction(&inner),
        };
        assert!(load(&paths, &vault).unwrap().is_none());
        store(&paths, &vault, &pending).unwrap();
        assert_eq!(load(&paths, &vault).unwrap(), Some(pending));
        clear(&paths, &vault).unwrap();
        assert!(load(&paths, &vault).unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
