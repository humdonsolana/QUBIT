mod common;

use common::{keys, mollusk, program_id, system_transfer_data, vault_account, Host};
use mollusk_svm::{program::keyed_account_for_system_program, result::Check};
use qubit_interface::{
    encode_recover, inner_digest, message_digest, Vault, FLAG_SIGNER, FLAG_WRITABLE, VAULT_LEN,
};
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

#[test]
fn rotates_without_invoking() {
    let mollusk = mollusk();
    let keys = keys(30);
    let (system_id, _) = keyed_account_for_system_program();
    let destination = Pubkey::new_unique();
    let data = system_transfer_data(1);
    let resolved = [
        (keys.treasury.to_bytes(), FLAG_SIGNER | FLAG_WRITABLE),
        (destination.to_bytes(), FLAG_WRITABLE),
    ];
    let digest = inner_digest::<Host>(&system_id.to_bytes(), &resolved, &data).unwrap();
    let next = keys.pk_hash(1);
    let message = message_digest::<Host>(&keys.vault.to_bytes(), 0, &next, &digest);
    let signature = keys.sign(0, &message);
    let instruction = Instruction {
        program_id: program_id(),
        accounts: vec![AccountMeta::new(keys.vault, false)],
        data: encode_recover(&next, &signature, &digest).unwrap(),
    };
    let rent = mollusk.sysvars.rent.minimum_balance(VAULT_LEN);
    let accounts = vec![(keys.vault, vault_account(&keys.state(0), rent))];
    let expected = Vault {
        key_index: 1,
        current_pk_hash: next,
        ..keys.state(0)
    };
    let result = mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[
            Check::success(),
            Check::account(&keys.vault)
                .data(&expected.to_bytes())
                .build(),
        ],
    );
    println!("recover CU: {}", result.compute_units_consumed);

    let mut wrong_digest = instruction.clone();
    let len = wrong_digest.data.len();
    wrong_digest.data[len - 1] ^= 1;
    mollusk.process_and_validate_instruction(
        &wrong_digest,
        &accounts,
        &[Check::err(ProgramError::Custom(3))],
    );

    let mut unsigned_vault = accounts.clone();
    unsigned_vault[0].1.owner = Pubkey::new_unique();
    mollusk.process_and_validate_instruction(
        &instruction,
        &unsigned_vault,
        &[Check::err(ProgramError::Custom(1))],
    );
}
