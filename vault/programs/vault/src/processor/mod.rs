mod create_vault;
mod execute;
mod recover;

pub use create_vault::create_vault;
pub use execute::execute;
pub use recover::recover;

use crate::{error::VaultError, hash::SyscallSha256, PROGRAM_ID};
use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};
use qubit_interface::{message_digest, Vault, TREASURY_SEED, VAULT_LEN};
use qubit_wots::{verify, Hash, Tweak};

pub(crate) fn load_vault(account: &AccountView) -> Result<Vault, ProgramError> {
    if !account.owned_by(&PROGRAM_ID) || !account.is_writable() {
        return Err(VaultError::InvalidVaultAccount.into());
    }
    let data = account.try_borrow()?;
    Vault::from_bytes(&data).ok_or_else(|| VaultError::InvalidVaultAccount.into())
}

pub(crate) fn store_vault(account: &mut AccountView, state: &Vault) -> ProgramResult {
    let mut data = account.try_borrow_mut()?;
    if data.len() != VAULT_LEN {
        return Err(VaultError::InvalidVaultAccount.into());
    }
    data.copy_from_slice(&state.to_bytes());
    Ok(())
}

pub(crate) fn check_treasury(vault: &Address, treasury: &Address, bump: u8) -> ProgramResult {
    let expected =
        Address::create_program_address(&[TREASURY_SEED, vault.as_array(), &[bump]], &PROGRAM_ID)
            .map_err(|_| VaultError::InvalidTreasury)?;
    if &expected != treasury {
        return Err(VaultError::InvalidTreasury.into());
    }
    Ok(())
}

pub(crate) fn verify_signature(
    vault: &Address,
    state: &Vault,
    next_pk_hash: &Hash,
    inner_digest: &Hash,
    signature: &[u8],
) -> ProgramResult {
    let tweak = Tweak {
        vault: state.root,
        key_index: state.key_index,
    };
    let message = message_digest::<SyscallSha256>(
        vault.as_array(),
        state.key_index,
        next_pk_hash,
        inner_digest,
    );
    if !verify::<SyscallSha256>(&tweak, &state.current_pk_hash, &message, signature) {
        return Err(VaultError::InvalidSignature.into());
    }
    Ok(())
}

pub(crate) fn rotate(state: &mut Vault, next_pk_hash: &Hash) -> ProgramResult {
    state.key_index = state
        .key_index
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    state.current_pk_hash = *next_pk_hash;
    Ok(())
}
