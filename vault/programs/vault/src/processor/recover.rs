use super::{load_vault, rotate, store_vault, verify_signature};
use pinocchio::{error::ProgramError, AccountView, ProgramResult};
use qubit_wots::Hash;

pub fn recover(
    accounts: &mut [AccountView],
    next_pk_hash: &Hash,
    signature: &[u8],
    inner_digest: &Hash,
) -> ProgramResult {
    let vault = accounts
        .first_mut()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    let mut state = load_vault(vault)?;
    verify_signature(
        vault.address(),
        &state,
        next_pk_hash,
        inner_digest,
        signature,
    )?;
    rotate(&mut state, next_pk_hash)?;
    store_vault(vault, &state)
}
