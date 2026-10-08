use super::{check_treasury, load_vault, rotate, store_vault, verify_signature};
use crate::{error::VaultError, hash::SyscallSha256, PROGRAM_ID};
use alloc::vec::Vec;
use pinocchio::{
    cpi::{invoke_signed_with_slice, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    AccountView, ProgramResult,
};
use qubit_interface::{inner_digest, Inner, FLAG_SIGNER, FLAG_WRITABLE, TREASURY_SEED};
use qubit_wots::Hash;

pub fn execute(
    accounts: &mut [AccountView],
    next_pk_hash: &Hash,
    signature: &[u8],
    inner: Inner<'_>,
) -> ProgramResult {
    let [vault, treasury, ..] = &*accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let mut state = load_vault(vault)?;
    check_treasury(vault.address(), treasury.address(), state.treasury_bump)?;

    let program_index = usize::from(inner.program_index);
    let program = accounts
        .get(program_index)
        .ok_or(VaultError::InvalidInnerInstruction)?;
    if program.address() == &PROGRAM_ID {
        return Err(VaultError::SelfInvocation.into());
    }
    if !program.executable() {
        return Err(VaultError::InvalidInnerInstruction.into());
    }
    let mut resolved = Vec::with_capacity(inner.account_count());
    for (index, flags) in inner.accounts() {
        let account = accounts
            .get(usize::from(index))
            .ok_or(VaultError::InvalidInnerInstruction)?;
        resolved.push((*account.address().as_array(), flags));
    }
    let digest = inner_digest::<SyscallSha256>(program.address().as_array(), &resolved, inner.data)
        .ok_or(VaultError::InvalidInnerInstruction)?;
    verify_signature(vault.address(), &state, next_pk_hash, &digest, signature)?;

    rotate(&mut state, next_pk_hash)?;
    store_vault(&mut accounts[0], &state)?;

    let metas: Vec<InstructionAccount> = inner
        .accounts()
        .map(|(index, flags)| {
            InstructionAccount::new(
                accounts[usize::from(index)].address(),
                flags & FLAG_WRITABLE != 0,
                flags & FLAG_SIGNER != 0,
            )
        })
        .collect();
    let cpi_accounts: Vec<&AccountView> = inner
        .accounts()
        .map(|(index, _)| &accounts[usize::from(index)])
        .collect();
    let instruction = InstructionView {
        program_id: accounts[program_index].address(),
        data: inner.data,
        accounts: &metas,
    };
    let bump = [state.treasury_bump];
    let seeds = [
        Seed::from(TREASURY_SEED),
        Seed::from(accounts[0].address().as_array()),
        Seed::from(&bump),
    ];
    invoke_signed_with_slice(&instruction, &cpi_accounts, &[Signer::from(&seeds)])
}
