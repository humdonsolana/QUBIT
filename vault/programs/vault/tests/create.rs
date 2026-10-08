mod common;

use common::{keys, mollusk, program_id, system_account};
use mollusk_svm::{program::keyed_account_for_system_program, result::Check};
use qubit_interface::{encode_create_vault, VAULT_LEN};
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

fn create_instruction(
    payer: Pubkey,
    vault: Pubkey,
    treasury: Pubkey,
    root: [u8; 32],
    pk_hash: [u8; 32],
) -> Instruction {
    let (system_id, _) = keyed_account_for_system_program();
    Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(vault, false),
            AccountMeta::new(treasury, false),
            AccountMeta::new_readonly(system_id, false),
        ],
        data: encode_create_vault(&root, &pk_hash),
    }
}

#[test]
fn creates_vault_and_funds_treasury() {
    let mollusk = mollusk();
    let keys = keys(1);
    let payer = Pubkey::new_unique();
    let instruction =
        create_instruction(payer, keys.vault, keys.treasury, keys.root, keys.pk_hash(0));
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (keys.vault, system_account(0)),
        (keys.treasury, system_account(0)),
        keyed_account_for_system_program(),
    ];
    let rent = &mollusk.sysvars.rent;
    let result = mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[
            Check::success(),
            Check::account(&keys.vault)
                .owner(&program_id())
                .data(&keys.state(0).to_bytes())
                .lamports(rent.minimum_balance(VAULT_LEN))
                .build(),
            Check::account(&keys.treasury)
                .lamports(rent.minimum_balance(0))
                .build(),
        ],
    );
    println!("create_vault CU: {}", result.compute_units_consumed);
    assert!(result.compute_units_consumed < 60_000);
}

#[test]
fn rejects_wrong_vault_address() {
    let mollusk = mollusk();
    let keys = keys(2);
    let payer = Pubkey::new_unique();
    let wrong = Pubkey::new_unique();
    let instruction = create_instruction(payer, wrong, keys.treasury, keys.root, keys.pk_hash(0));
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (wrong, system_account(0)),
        (keys.treasury, system_account(0)),
        keyed_account_for_system_program(),
    ];
    mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[Check::err(ProgramError::Custom(6))],
    );
}

#[test]
fn rejects_wrong_treasury() {
    let mollusk = mollusk();
    let keys = keys(3);
    let payer = Pubkey::new_unique();
    let wrong = Pubkey::new_unique();
    let instruction = create_instruction(payer, keys.vault, wrong, keys.root, keys.pk_hash(0));
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (keys.vault, system_account(0)),
        (wrong, system_account(0)),
        keyed_account_for_system_program(),
    ];
    mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[Check::err(ProgramError::Custom(2))],
    );
}

#[test]
fn rejects_unsigned_payer() {
    let mollusk = mollusk();
    let keys = keys(4);
    let payer = Pubkey::new_unique();
    let mut instruction =
        create_instruction(payer, keys.vault, keys.treasury, keys.root, keys.pk_hash(0));
    instruction.accounts[0].is_signer = false;
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (keys.vault, system_account(0)),
        (keys.treasury, system_account(0)),
        keyed_account_for_system_program(),
    ];
    mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[Check::err(ProgramError::MissingRequiredSignature)],
    );
}

#[test]
fn rejects_malformed_data() {
    let mollusk = mollusk();
    let keys = keys(5);
    let payer = Pubkey::new_unique();
    let mut instruction =
        create_instruction(payer, keys.vault, keys.treasury, keys.root, keys.pk_hash(0));
    instruction.data.pop();
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (keys.vault, system_account(0)),
        (keys.treasury, system_account(0)),
        keyed_account_for_system_program(),
    ];
    mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[Check::err(ProgramError::InvalidInstructionData)],
    );
}

#[test]
fn rejects_another_key_for_this_address() {
    let mollusk = mollusk();
    let keys = keys(6);
    let payer = Pubkey::new_unique();
    let instruction =
        create_instruction(payer, keys.vault, keys.treasury, keys.root, keys.pk_hash(1));
    let accounts = vec![
        (payer, system_account(10_000_000_000)),
        (keys.vault, system_account(0)),
        (keys.treasury, system_account(0)),
        keyed_account_for_system_program(),
    ];
    mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[Check::err(ProgramError::Custom(6))],
    );
}
