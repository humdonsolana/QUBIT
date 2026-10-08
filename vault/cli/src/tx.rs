use crate::rpc::Rpc;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::{
    v1::{Message, TransactionConfig, MAX_TRANSACTION_SIZE},
    VersionedMessage,
};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Feature gate of SIMD-0385 (V1 transactions).
pub const V1_FEATURE: Address =
    Address::from_str_const("txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL");
pub const LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 32 * 1024 * 1024;
pub const CU_CREATE: u32 = 60_000;
pub const CU_EXECUTE: u32 = 250_000;
pub const CU_SWEEP: u32 = 200_000;
const BASE_FEE_PER_SIGNATURE: u64 = 5_000;
const POLL_INTERVAL: Duration = Duration::from_millis(1_500);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_ATTEMPTS: usize = 4;

pub struct TxConfig {
    pub compute_unit_limit: u32,
    pub priority_fee: Option<u64>,
}

impl TxConfig {
    /// Upper bound of the fee this transaction can cost its payer; V1 priority fees are total lamports.
    pub fn fee_lamports(&self, signatures: u64) -> u64 {
        BASE_FEE_PER_SIGNATURE * signatures + self.priority_fee.unwrap_or(0)
    }
}

fn build(
    payer: &Address,
    instructions: &[Instruction],
    blockhash: Hash,
    config: &TxConfig,
) -> Result<VersionedMessage> {
    let mut tx_config = TransactionConfig::empty()
        .with_compute_unit_limit(config.compute_unit_limit)
        .with_loaded_accounts_data_size_limit(LOADED_ACCOUNTS_DATA_SIZE_LIMIT);
    if let Some(fee) = config.priority_fee {
        tx_config = tx_config.with_priority_fee(fee);
    }
    let message = Message::try_compile_with_config(payer, instructions, blockhash, tx_config)
        .map_err(|error| anyhow!("cannot compile transaction: {error}"))?;
    Ok(VersionedMessage::V1(message))
}

fn sign(message: VersionedMessage, signers: &[&Keypair]) -> Result<VersionedTransaction> {
    VersionedTransaction::try_new(message, signers)
        .map_err(|error| anyhow!("signing failed: {error}"))
}

/// Serializes and base64-encodes a transaction, enforcing the V1 size limit.
pub fn encode(transaction: &VersionedTransaction) -> Result<String> {
    let bytes = wincode::serialize(transaction)
        .map_err(|error| anyhow!("cannot serialize transaction: {error:?}"))?;
    if bytes.len() > MAX_TRANSACTION_SIZE {
        bail!(
            "transaction is {} bytes, the limit is {MAX_TRANSACTION_SIZE}",
            bytes.len()
        );
    }
    Ok(STANDARD.encode(bytes))
}

/// Builds, signs with `payer`, simulates, sends and confirms `instructions`; rebuilds with a fresh blockhash when the previous one expires.
pub fn submit(
    rpc: &Rpc,
    payer: &Keypair,
    instructions: &[Instruction],
    config: &TxConfig,
) -> Result<String> {
    for attempt in 1..=MAX_ATTEMPTS {
        let blockhash = rpc.latest_blockhash()?;
        let message = build(&payer.pubkey(), instructions, blockhash, config)?;
        let encoded = encode(&sign(message, &[payer])?)?;
        let simulation = rpc.simulate(&encoded)?;
        if let Some(err) = simulation.err {
            bail!(
                "simulation failed: {err}\n{}",
                simulation.logs.unwrap_or_default().join("\n")
            );
        }
        let signature = rpc.send(&encoded)?;
        let started = Instant::now();
        while started.elapsed() < CONFIRM_TIMEOUT {
            thread::sleep(POLL_INTERVAL);
            match rpc.signature_status(&signature)? {
                Some(status) => {
                    if let Some(err) = status.err {
                        bail!("transaction {signature} failed: {err}");
                    }
                    if matches!(
                        status.confirmation_status.as_deref(),
                        Some("confirmed" | "finalized")
                    ) {
                        return Ok(signature);
                    }
                }
                None if !rpc.is_blockhash_valid(&blockhash)? => break,
                None => {}
            }
        }
        eprintln!(
            "blockhash expired before confirmation (attempt {attempt}/{MAX_ATTEMPTS}), retrying"
        );
    }
    bail!("transaction not confirmed after {MAX_ATTEMPTS} attempts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ix;
    use qubit_interface::{encode_execute, InnerSpec, FLAG_SIGNER, FLAG_WRITABLE};
    use qubit_wots::SIGNATURE_LEN;
    use solana_instruction::AccountMeta;
    use solana_signer::Signer;

    fn execute_like(extra_accounts: usize) -> Instruction {
        let accounts: Vec<AccountMeta> = (0..4 + extra_accounts)
            .map(|index| AccountMeta::new(Address::new_from_array([index as u8 + 1; 32]), false))
            .collect();
        let spec = InnerSpec {
            program_index: 2,
            accounts: &[(1, FLAG_SIGNER | FLAG_WRITABLE), (3, FLAG_WRITABLE)],
            data: &[0; 12],
        };
        Instruction {
            program_id: ix::SYSTEM_PROGRAM,
            accounts,
            data: encode_execute(&[0; 32], &[7; SIGNATURE_LEN], &spec).unwrap(),
        }
    }

    #[test]
    fn execute_fits_in_one_v1_transaction() {
        let payer = Keypair::new();
        let config = TxConfig {
            compute_unit_limit: CU_EXECUTE,
            priority_fee: Some(1_000),
        };
        let message = build(
            &payer.pubkey(),
            &[execute_like(0)],
            Hash::new_from_array([1; 32]),
            &config,
        )
        .unwrap();
        let encoded = encode(&sign(message, &[&payer]).unwrap()).unwrap();
        let size = STANDARD.decode(encoded).unwrap().len();
        assert!(size > 1232, "V1 needed: {size}");
        assert!(size <= MAX_TRANSACTION_SIZE);
    }

    #[test]
    fn encode_rejects_oversized() {
        let payer = Keypair::new();
        let config = TxConfig {
            compute_unit_limit: CU_EXECUTE,
            priority_fee: None,
        };
        let padded = Instruction {
            program_id: ix::SYSTEM_PROGRAM,
            accounts: Vec::new(),
            data: vec![0; 2_500],
        };
        let message = build(
            &payer.pubkey(),
            &[execute_like(0), padded],
            Hash::new_from_array([1; 32]),
            &config,
        )
        .unwrap();
        assert!(encode(&sign(message, &[&payer]).unwrap()).is_err());
    }

    #[test]
    fn fee_bound_includes_priority() {
        assert_eq!(
            TxConfig {
                compute_unit_limit: 200_000,
                priority_fee: None
            }
            .fee_lamports(1),
            5_000
        );
        assert_eq!(
            TxConfig {
                compute_unit_limit: 200_000,
                priority_fee: Some(1_000)
            }
            .fee_lamports(2),
            11_000
        );
    }
}
