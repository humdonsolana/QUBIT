use crate::keys::PROGRAM_ID;
use anyhow::{anyhow, Result};
use qubit_interface::{
    encode_create_vault, encode_execute, encode_recover, InnerSpec, FLAG_SIGNER, FLAG_WRITABLE,
};
use qubit_wots::Hash;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};

pub const SYSTEM_PROGRAM: Address = Address::from_str_const("11111111111111111111111111111111");
pub const TOKEN_PROGRAM: Address =
    Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Address =
    Address::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ASSOCIATED_TOKEN_PROGRAM: Address =
    Address::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const STAKE_PROGRAM: Address =
    Address::from_str_const("Stake11111111111111111111111111111111111111");
pub const CLOCK_SYSVAR: Address =
    Address::from_str_const("SysvarC1ock11111111111111111111111111111111");

pub fn system_transfer(from: &Address, to: &Address, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data,
    }
}

pub fn associated_token_address(
    owner: &Address,
    mint: &Address,
    token_program: &Address,
) -> Address {
    Address::find_program_address(
        &[owner.as_array(), token_program.as_array(), mint.as_array()],
        &ASSOCIATED_TOKEN_PROGRAM,
    )
    .0
}

pub fn create_associated_token_account_idempotent(
    payer: &Address,
    owner: &Address,
    mint: &Address,
    token_program: &Address,
) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(associated_token_address(owner, mint, token_program), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: vec![1],
    }
}

pub fn transfer_checked(
    token_program: &Address,
    source: &Address,
    mint: &Address,
    destination: &Address,
    authority: &Address,
    amount: u64,
    decimals: u8,
) -> Instruction {
    let mut data = vec![12u8];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Instruction {
        program_id: *token_program,
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

pub fn close_account(
    token_program: &Address,
    account: &Address,
    destination: &Address,
    owner: &Address,
) -> Instruction {
    Instruction {
        program_id: *token_program,
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data: vec![9],
    }
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum StakeAuthority {
    Staker = 0,
    Withdrawer = 1,
}

pub fn stake_authorize(
    stake: &Address,
    current_authority: &Address,
    new_authority: &Address,
    kind: StakeAuthority,
) -> Instruction {
    let mut data = 1u32.to_le_bytes().to_vec();
    data.extend_from_slice(new_authority.as_array());
    data.extend_from_slice(&(kind as u32).to_le_bytes());
    Instruction {
        program_id: STAKE_PROGRAM,
        accounts: vec![
            AccountMeta::new(*stake, false),
            AccountMeta::new_readonly(CLOCK_SYSVAR, false),
            AccountMeta::new_readonly(*current_authority, true),
        ],
        data,
    }
}

pub fn vault_create(
    payer: &Address,
    vault: &Address,
    treasury: &Address,
    root: &[u8; 32],
    pk_hash: &Hash,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*vault, false),
            AccountMeta::new(*treasury, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: encode_create_vault(root, pk_hash),
    }
}

/// `(address, flags)` pairs of an inner instruction, as hashed by the program.
pub fn inner_accounts(inner: &Instruction) -> Vec<([u8; 32], u8)> {
    inner
        .accounts
        .iter()
        .map(|meta| (*meta.pubkey.as_array(), flags(meta)))
        .collect()
}

fn flags(meta: &AccountMeta) -> u8 {
    (u8::from(meta.is_signer) * FLAG_SIGNER) | (u8::from(meta.is_writable) * FLAG_WRITABLE)
}

/// Wraps `inner` into an `Execute` instruction signed on-chain by the treasury.
pub fn vault_execute(
    vault: &Address,
    treasury: &Address,
    next_pk_hash: &Hash,
    signature: &[u8],
    inner: &Instruction,
) -> Result<Instruction> {
    let mut accounts = vec![
        AccountMeta::new(*vault, false),
        AccountMeta::new_readonly(*treasury, false),
    ];
    let mut index_of = |meta: AccountMeta| -> Result<u8> {
        let position = match accounts
            .iter()
            .position(|existing| existing.pubkey == meta.pubkey)
        {
            Some(position) => {
                accounts[position].is_writable |= meta.is_writable;
                accounts[position].is_signer |= meta.is_signer;
                position
            }
            None => {
                accounts.push(meta);
                accounts.len() - 1
            }
        };
        u8::try_from(position).map_err(|_| anyhow!("too many accounts in one transaction"))
    };
    let program_index = index_of(AccountMeta::new_readonly(inner.program_id, false))?;
    let mut spec = Vec::with_capacity(inner.accounts.len());
    for meta in &inner.accounts {
        let signs_outer = meta.is_signer && meta.pubkey != *treasury;
        let index = index_of(AccountMeta {
            pubkey: meta.pubkey,
            is_signer: signs_outer,
            is_writable: meta.is_writable,
        })?;
        spec.push((index, flags(meta)));
    }
    let data = encode_execute(
        next_pk_hash,
        signature,
        &InnerSpec {
            program_index,
            accounts: &spec,
            data: &inner.data,
        },
    )
    .ok_or_else(|| anyhow!("inner instruction too large"))?;
    Ok(Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    })
}

pub fn vault_recover(
    vault: &Address,
    next_pk_hash: &Hash,
    signature: &[u8],
    inner_digest: &Hash,
) -> Result<Instruction> {
    let data = encode_recover(next_pk_hash, signature, inner_digest)
        .ok_or_else(|| anyhow!("signature has the wrong length"))?;
    Ok(Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![AccountMeta::new(*vault, false)],
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qubit_wots::SIGNATURE_LEN;

    fn addr(byte: u8) -> Address {
        Address::new_from_array([byte; 32])
    }

    #[test]
    fn system_transfer_bytes() {
        let ix = system_transfer(&addr(1), &addr(2), 1_000);
        assert_eq!(ix.data, [2, 0, 0, 0, 232, 3, 0, 0, 0, 0, 0, 0]);
        assert!(ix.accounts[0].is_signer && ix.accounts[0].is_writable);
        assert!(!ix.accounts[1].is_signer && ix.accounts[1].is_writable);
    }

    #[test]
    fn token_instruction_bytes() {
        let ix = transfer_checked(
            &TOKEN_PROGRAM,
            &addr(1),
            &addr(2),
            &addr(3),
            &addr(4),
            250_000,
            6,
        );
        assert_eq!(ix.data, [12, 144, 208, 3, 0, 0, 0, 0, 0, 6]);
        assert!(ix.accounts[3].is_signer && !ix.accounts[3].is_writable);
        assert_eq!(
            close_account(&TOKEN_PROGRAM, &addr(1), &addr(2), &addr(3)).data,
            [9]
        );
        let ata = create_associated_token_account_idempotent(
            &addr(1),
            &addr(2),
            &addr(3),
            &TOKEN_PROGRAM,
        );
        assert_eq!(ata.data, [1]);
        assert_eq!(ata.accounts.len(), 6);
        assert_eq!(
            ata.accounts[1].pubkey,
            associated_token_address(&addr(2), &addr(3), &TOKEN_PROGRAM)
        );
    }

    #[test]
    fn known_associated_token_address() {
        let owner = Address::from_str_const("4Nd1mBQtrMJVYVfKf2PJy9NZUZdTAsp7D4xWLs4gDB4T");
        let usdc = Address::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(
            associated_token_address(&owner, &usdc, &TOKEN_PROGRAM).to_string(),
            "F8biqkCRK2tHR6EncrcXDGgVTkGRrtojqyW39w41Qspn"
        );
    }

    #[test]
    fn stake_authorize_bytes() {
        let ix = stake_authorize(&addr(1), &addr(2), &addr(3), StakeAuthority::Withdrawer);
        let mut expected = vec![1, 0, 0, 0];
        expected.extend_from_slice(&[3; 32]);
        expected.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(ix.data, expected);
        assert_eq!(ix.accounts[1].pubkey, CLOCK_SYSVAR);
    }

    #[test]
    fn execute_wrapper_dedupes_and_indexes() {
        let vault = addr(10);
        let treasury = addr(11);
        let inner = system_transfer(&treasury, &addr(12), 5);
        let ix = vault_execute(&vault, &treasury, &[0; 32], &[1; SIGNATURE_LEN], &inner).unwrap();
        let keys: Vec<Address> = ix.accounts.iter().map(|m| m.pubkey).collect();
        assert_eq!(keys, vec![vault, treasury, SYSTEM_PROGRAM, addr(12)]);
        assert!(ix.accounts[1].is_writable && !ix.accounts[1].is_signer);
        assert!(ix.accounts.iter().all(|m| !m.is_signer));
        let tail = &ix.data[1 + 32 + SIGNATURE_LEN..];
        assert_eq!(
            &tail[..6],
            &[2, 2, 1, FLAG_SIGNER | FLAG_WRITABLE, 3, FLAG_WRITABLE]
        );
        assert_eq!(&tail[6..8], &[12, 0]);
        assert_eq!(&tail[8..], &inner.data[..]);
        assert_eq!(inner_accounts(&inner), vec![([11; 32], 3), ([12; 32], 2)]);
    }
}
