use super::store_vault;
use crate::{error::VaultError, hash::SyscallSha256, PROGRAM_ID};
use pinocchio::{
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};
use pinocchio_system::instructions::{CreateAccount, Transfer};
use qubit_interface::{vault_id, Vault, TREASURY_SEED, VAULT_LEN, VAULT_SEED};

pub fn create_vault(
    accounts: &mut [AccountView],
    root: &[u8; 32],
    pk_hash: &[u8; 32],
) -> ProgramResult {
    let [payer, vault, treasury, _system_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !payer.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let id = vault_id::<SyscallSha256>(root, pk_hash);
    let (expected_vault, vault_bump) =
        Address::find_program_address(&[VAULT_SEED, &id], &PROGRAM_ID);
    if vault.address() != &expected_vault {
        return Err(VaultError::InvalidBump.into());
    }
    let (expected_treasury, treasury_bump) =
        Address::find_program_address(&[TREASURY_SEED, expected_vault.as_array()], &PROGRAM_ID);
    if treasury.address() != &expected_treasury {
        return Err(VaultError::InvalidTreasury.into());
    }

    let rent = Rent::get()?;
    let bump = [vault_bump];
    let seeds = [Seed::from(VAULT_SEED), Seed::from(&id), Seed::from(&bump)];
    CreateAccount {
        from: &*payer,
        to: &*vault,
        lamports: rent.try_minimum_balance(VAULT_LEN)?,
        space: VAULT_LEN as u64,
        owner: &PROGRAM_ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;

    let state = Vault {
        vault_bump,
        treasury_bump,
        key_index: 0,
        current_pk_hash: *pk_hash,
        root: *root,
    };
    store_vault(vault, &state)?;

    let minimum = rent.try_minimum_balance(0)?;
    if treasury.lamports() < minimum {
        Transfer {
            from: &*payer,
            to: &*treasury,
            lamports: minimum - treasury.lamports(),
        }
        .invoke()?;
    }
    Ok(())
}
