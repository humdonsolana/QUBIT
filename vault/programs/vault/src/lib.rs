//! QUBIT vault: a Solana vault spent with WOTS+ one-time signatures.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod error;
mod hash;
mod processor;

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};
use qubit_interface::{Instruction, ID};

#[cfg(target_os = "solana")]
pinocchio::program_entrypoint!(process_instruction);
#[cfg(target_os = "solana")]
pinocchio::default_allocator!();
#[cfg(target_os = "solana")]
pinocchio::nostd_panic_handler!();

/// Program id.
pub const PROGRAM_ID: Address = Address::new_from_array(ID);

/// Dispatches one instruction.
#[inline(never)]
pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    if program_id != &PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    match Instruction::parse(data).ok_or(ProgramError::InvalidInstructionData)? {
        Instruction::CreateVault { root, pk_hash } => {
            processor::create_vault(accounts, root, pk_hash)
        }
        Instruction::Execute {
            next_pk_hash,
            signature,
            inner,
        } => processor::execute(accounts, next_pk_hash, signature, inner),
        Instruction::Recover {
            next_pk_hash,
            signature,
            inner_digest,
        } => processor::recover(accounts, next_pk_hash, signature, inner_digest),
    }
}
