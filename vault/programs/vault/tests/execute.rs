mod common;

use common::{
    keys, mollusk, program_id, system_account, system_transfer_data, vault_account, Host, Keys,
};
use mollusk_svm::{program::keyed_account_for_system_program, result::Check, Mollusk};
use mollusk_svm_programs_token::token;
use qubit_interface::{
    encode_execute, inner_digest, message_digest, InnerSpec, Vault, FLAG_SIGNER, FLAG_WRITABLE,
    VAULT_LEN,
};
use qubit_wots::Hash;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

const SOL: u64 = 1_000_000_000;

struct Transfer {
    instruction: Instruction,
    accounts: Vec<(Pubkey, Account)>,
    destination: Pubkey,
    next: Hash,
}

/// Builds a signed `Execute` whose inner instruction moves `amount` lamports from the treasury.
fn sol_transfer(mollusk: &Mollusk, keys: &Keys, key_index: u64, amount: u64) -> Transfer {
    let (system_id, system_account_entry) = keyed_account_for_system_program();
    let destination = Pubkey::new_unique();
    let data = system_transfer_data(amount);
    let next = keys.pk_hash(key_index + 1);
    let resolved = [
        (keys.treasury.to_bytes(), FLAG_SIGNER | FLAG_WRITABLE),
        (destination.to_bytes(), FLAG_WRITABLE),
    ];
    let digest = inner_digest::<Host>(&system_id.to_bytes(), &resolved, &data).unwrap();
    let message = message_digest::<Host>(&keys.vault.to_bytes(), key_index, &next, &digest);
    let signature = keys.sign(key_index, &message);
    let spec = InnerSpec {
        program_index: 2,
        accounts: &[(1, FLAG_SIGNER | FLAG_WRITABLE), (3, FLAG_WRITABLE)],
        data: &data,
    };
    let instruction = Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(keys.vault, false),
            AccountMeta::new(keys.treasury, false),
            AccountMeta::new_readonly(system_id, false),
            AccountMeta::new(destination, false),
        ],
        data: encode_execute(&next, &signature, &spec).unwrap(),
    };
    let accounts = vec![
        (
            keys.vault,
            vault_account(
                &keys.state(key_index),
                mollusk.sysvars.rent.minimum_balance(VAULT_LEN),
            ),
        ),
        (keys.treasury, system_account(5 * SOL)),
        (system_id, system_account_entry),
        (destination, system_account(0)),
    ];
    Transfer {
        instruction,
        accounts,
        destination,
        next,
    }
}

fn expect_custom(mollusk: &Mollusk, transfer: &Transfer, code: u32) {
    mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &transfer.accounts,
        &[Check::err(ProgramError::Custom(code))],
    );
}

#[test]
fn transfers_sol_and_rotates_key() {
    let mollusk = mollusk();
    let keys = keys(10);
    let transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    let expected = Vault {
        key_index: 1,
        current_pk_hash: transfer.next,
        ..keys.state(0)
    };
    let result = mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &transfer.accounts,
        &[
            Check::success(),
            Check::account(&keys.treasury).lamports(4 * SOL).build(),
            Check::account(&transfer.destination).lamports(SOL).build(),
            Check::account(&keys.vault)
                .data(&expected.to_bytes())
                .build(),
        ],
    );
    println!(
        "execute(sol transfer) CU: {}",
        result.compute_units_consumed
    );
    assert!(result.compute_units_consumed < 250_000);
}

#[test]
fn works_at_a_high_key_index() {
    let mollusk = mollusk();
    let keys = keys(11);
    let transfer = sol_transfer(&mollusk, &keys, 123_456, SOL);
    mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &transfer.accounts,
        &[Check::success()],
    );
}

#[test]
fn rejects_corrupted_signature() {
    let mollusk = mollusk();
    let keys = keys(12);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    transfer.instruction.data[40] ^= 0x01;
    expect_custom(&mollusk, &transfer, 3);
}

#[test]
fn rejects_replay_after_rotation() {
    let mollusk = mollusk();
    let keys = keys(13);
    let transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    let first = mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &transfer.accounts,
        &[Check::success()],
    );
    mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &first.resulting_accounts,
        &[Check::err(ProgramError::Custom(3))],
    );
}

#[test]
fn rejects_tampered_next_key() {
    let mollusk = mollusk();
    let keys = keys(14);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    transfer.instruction.data[1] ^= 0x01;
    expect_custom(&mollusk, &transfer, 3);
}

#[test]
fn rejects_tampered_inner_data() {
    let mollusk = mollusk();
    let keys = keys(15);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    let len = transfer.instruction.data.len();
    transfer.instruction.data[len - 1] ^= 0x01;
    expect_custom(&mollusk, &transfer, 3);
}

#[test]
fn rejects_wrong_treasury() {
    let mollusk = mollusk();
    let keys = keys(16);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    let wrong = Pubkey::new_unique();
    transfer.instruction.accounts[1].pubkey = wrong;
    transfer.accounts[1] = (wrong, system_account(5 * SOL));
    expect_custom(&mollusk, &transfer, 2);
}

#[test]
fn rejects_self_invocation() {
    let mollusk = mollusk();
    let keys = keys(17);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    transfer.instruction.accounts[2].pubkey = program_id();
    transfer.accounts[2] = (
        program_id(),
        Account {
            lamports: 1,
            data: Vec::new(),
            owner: mollusk_svm::program::loader_keys::LOADER_V3,
            executable: true,
            rent_epoch: 0,
        },
    );
    expect_custom(&mollusk, &transfer, 4);
}

#[test]
fn execute_rejects_out_of_range_index() {
    let mollusk = mollusk();
    let keys = keys(18);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    let len = transfer.instruction.data.len();
    // inner layout after the signature: program_index, count, (idx, flags) x2, len u16, data(12)
    let pairs_start = len - 12 - 2 - 4;
    transfer.instruction.data[pairs_start + 2] = 9;
    expect_custom(&mollusk, &transfer, 5);
}

#[test]
fn rejects_vault_owned_by_another_program() {
    let mollusk = mollusk();
    let keys = keys(19);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    transfer.accounts[0].1.owner = Pubkey::new_unique();
    expect_custom(&mollusk, &transfer, 1);
}

#[test]
fn rejects_missing_accounts() {
    let mollusk = mollusk();
    let keys = keys(20);
    let mut transfer = sol_transfer(&mollusk, &keys, 0, SOL);
    transfer.instruction.accounts.truncate(1);
    transfer.accounts.truncate(1);
    mollusk.process_and_validate_instruction(
        &transfer.instruction,
        &transfer.accounts,
        &[Check::err(ProgramError::NotEnoughAccountKeys)],
    );
}

fn token_account_data(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Vec<u8> {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

fn mint_data(decimals: u8, supply: u64) -> Vec<u8> {
    let mut data = vec![0u8; 82];
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = decimals;
    data[45] = 1;
    data
}

fn token_account(mollusk: &Mollusk, data: Vec<u8>) -> Account {
    Account {
        lamports: mollusk.sysvars.rent.minimum_balance(data.len()),
        data,
        owner: token::ID,
        executable: false,
        rent_epoch: 0,
    }
}

#[test]
fn transfers_spl_tokens() {
    let mut mollusk = mollusk();
    token::add_program(&mut mollusk);
    let keys = keys(21);
    let mint = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let amount = 250_000u64;
    let mut data = vec![12u8];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(6);
    let next = keys.pk_hash(1);
    let resolved = [
        (source.to_bytes(), FLAG_WRITABLE),
        (mint.to_bytes(), 0),
        (destination.to_bytes(), FLAG_WRITABLE),
        (keys.treasury.to_bytes(), FLAG_SIGNER),
    ];
    let digest = inner_digest::<Host>(&token::ID.to_bytes(), &resolved, &data).unwrap();
    let message = message_digest::<Host>(&keys.vault.to_bytes(), 0, &next, &digest);
    let signature = keys.sign(0, &message);
    let spec = InnerSpec {
        program_index: 2,
        accounts: &[
            (3, FLAG_WRITABLE),
            (4, 0),
            (5, FLAG_WRITABLE),
            (1, FLAG_SIGNER),
        ],
        data: &data,
    };
    let instruction = Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(keys.vault, false),
            AccountMeta::new_readonly(keys.treasury, false),
            AccountMeta::new_readonly(token::ID, false),
            AccountMeta::new(source, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(destination, false),
        ],
        data: encode_execute(&next, &signature, &spec).unwrap(),
    };
    let accounts = vec![
        (
            keys.vault,
            vault_account(
                &keys.state(0),
                mollusk.sysvars.rent.minimum_balance(VAULT_LEN),
            ),
        ),
        (keys.treasury, system_account(SOL)),
        token::keyed_account(),
        (
            source,
            token_account(
                &mollusk,
                token_account_data(&mint, &keys.treasury, 1_000_000),
            ),
        ),
        (mint, token_account(&mollusk, mint_data(6, 1_000_000))),
        (
            destination,
            token_account(
                &mollusk,
                token_account_data(&mint, &Pubkey::new_unique(), 0),
            ),
        ),
    ];
    let result = mollusk.process_and_validate_instruction(
        &instruction,
        &accounts,
        &[
            Check::success(),
            Check::account(&source)
                .data_slice(64, &750_000u64.to_le_bytes())
                .build(),
            Check::account(&destination)
                .data_slice(64, &amount.to_le_bytes())
                .build(),
        ],
    );
    println!(
        "execute(spl transfer_checked) CU: {}",
        result.compute_units_consumed
    );
    assert!(result.compute_units_consumed < 250_000);
}
