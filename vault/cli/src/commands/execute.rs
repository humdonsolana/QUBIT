use crate::{
    cli::Context,
    ix,
    keys::Wallet,
    pending::{self, Pending, StoredInstruction},
    tx::{self, CU_EXECUTE},
};
use anyhow::{anyhow, Result};
use qubit_interface::{inner_digest, message_digest};
use qubit_wots::{Hash, HostSha256};
use solana_instruction::Instruction;
use solana_keypair::Keypair;

/// Commits key `key_index` to `inner`, persists the pending file, then submits.
pub fn begin(
    ctx: &Context,
    wallet: &Wallet,
    key_index: u64,
    prelude: Vec<Instruction>,
    inner: Instruction,
) -> Result<()> {
    let pending = Pending {
        key_index,
        next_pk_hash: wallet.public_key_hash(key_index + 1),
        prelude: prelude
            .iter()
            .map(StoredInstruction::from_instruction)
            .collect(),
        inner: StoredInstruction::from_instruction(&inner),
    };
    pending::store(&ctx.paths, &wallet.vault, &pending)?;
    submit(ctx, wallet, &pending)
}

fn digest_of(inner: &Instruction) -> Result<Hash> {
    inner_digest::<HostSha256>(
        inner.program_id.as_array(),
        &ix::inner_accounts(inner),
        &inner.data,
    )
    .ok_or_else(|| anyhow!("inner instruction too large"))
}

fn signed_message(wallet: &Wallet, pending: &Pending, digest: &Hash) -> Vec<u8> {
    let message = message_digest::<HostSha256>(
        wallet.vault.as_array(),
        pending.key_index,
        &pending.next_pk_hash,
        digest,
    );
    wallet.sign(pending.key_index, &message)
}

fn send(
    ctx: &Context,
    wallet: &Wallet,
    instructions: Vec<Instruction>,
    payer: &Keypair,
) -> Result<()> {
    let signature = tx::submit(&ctx.rpc, payer, &instructions, &ctx.tx_config(CU_EXECUTE))?;
    pending::clear(&ctx.paths, &wallet.vault)?;
    println!("confirmed: {signature}");
    Ok(())
}

/// Re-signs exactly the pending message and executes it.
pub fn submit(ctx: &Context, wallet: &Wallet, pending: &Pending) -> Result<()> {
    let inner = pending.inner.to_instruction()?;
    let signature = signed_message(wallet, pending, &digest_of(&inner)?);
    let mut instructions = pending
        .prelude
        .iter()
        .map(StoredInstruction::to_instruction)
        .collect::<Result<Vec<_>>>()?;
    instructions.push(ix::vault_execute(
        &wallet.vault,
        &wallet.treasury,
        &pending.next_pk_hash,
        &signature,
        &inner,
    )?);
    send(ctx, wallet, instructions, &ctx.fee_payer()?)
}

/// Re-signs the pending message and rotates the key without executing the inner instruction.
pub fn submit_recover(ctx: &Context, wallet: &Wallet, pending: &Pending) -> Result<()> {
    let digest = digest_of(&pending.inner.to_instruction()?)?;
    let signature = signed_message(wallet, pending, &digest);
    let instruction = ix::vault_recover(&wallet.vault, &pending.next_pk_hash, &signature, &digest)?;
    send(ctx, wallet, vec![instruction], &ctx.fee_payer()?)
}
