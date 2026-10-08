use crate::{
    amount::format_amount,
    cli::Context,
    commands::token::{self, TokenAccount},
    ix, prompt,
    tx::{self, CU_SWEEP},
};
use anyhow::{anyhow, bail, Result};
use serde_json::json;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::{read_keypair_file, Keypair};
use solana_signer::Signer;
use std::path::Path;

const STAKE_ACCOUNT_LEN: usize = 200;
const STAKE_WITHDRAWER_OFFSET: usize = 44;
const TOKENS_PER_TX: usize = 4;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub tokens: Vec<TokenAccount>,
    pub empty_token_accounts: Vec<TokenAccount>,
    pub stakes: Vec<Address>,
    pub lamports: u64,
}

/// Splits token accounts into ones to move and ones to merely close; `lamports` is the SOL to move last.
pub fn plan_from_accounts(
    tokens: Vec<TokenAccount>,
    stakes: Vec<Address>,
    balance: u64,
    fee: u64,
) -> Plan {
    let (tokens, empty_token_accounts) = tokens.into_iter().partition(|account| account.amount > 0);
    Plan {
        tokens,
        empty_token_accounts,
        stakes,
        lamports: balance.saturating_sub(fee),
    }
}

fn stake_accounts(ctx: &Context, withdrawer: &Address) -> Result<Vec<Address>> {
    let filters = json!([
        { "dataSize": STAKE_ACCOUNT_LEN },
        { "memcmp": { "offset": STAKE_WITHDRAWER_OFFSET, "bytes": withdrawer.to_string() } }
    ]);
    Ok(ctx
        .rpc
        .program_accounts(&ix::STAKE_PROGRAM, filters)?
        .into_iter()
        .map(|(address, _)| address)
        .collect())
}

fn confirm(plan: &Plan, source: &Address, treasury: &Address) -> Result<bool> {
    println!("sweep {source} -> {treasury}");
    println!("  SOL:            {}", format_amount(plan.lamports, 9));
    for account in &plan.tokens {
        println!("  token {}: {} base units", account.mint, account.amount);
    }
    println!(
        "  empty token accounts to close: {}",
        plan.empty_token_accounts.len()
    );
    println!("  stake accounts to re-point:    {}", plan.stakes.len());
    prompt::confirm("Proceed? [y/N] ")
}

fn send(ctx: &Context, payer: &Keypair, instructions: &[Instruction], label: &str) -> Result<()> {
    let signature = tx::submit(&ctx.rpc, payer, instructions, &ctx.tx_config(CU_SWEEP))?;
    println!("{label}: {signature}");
    Ok(())
}

pub fn run(ctx: &Context, from: &Path, yes: bool) -> Result<()> {
    let wallet = ctx.wallet()?;
    let source = read_keypair_file(from)
        .map_err(|error| anyhow!("cannot read {}: {error}", from.display()))?;
    let source_key = source.pubkey();
    if source_key == wallet.treasury {
        bail!("source is the qubit itself");
    }
    let tokens = token::list_token_accounts(&ctx.rpc, &source_key)?;
    let stakes = stake_accounts(ctx, &source_key)?;
    let balance = ctx.rpc.balance(&source_key)?;
    let fee = ctx.tx_config(CU_SWEEP).fee_lamports(1);
    let plan = plan_from_accounts(tokens, stakes, balance, fee);
    if !yes && !confirm(&plan, &source_key, &wallet.treasury)? {
        bail!("aborted");
    }

    for batch in plan.tokens.chunks(TOKENS_PER_TX) {
        let mut instructions = Vec::new();
        for account in batch {
            let (_, decimals) = token::mint_info(&ctx.rpc, &account.mint)?;
            let destination =
                ix::associated_token_address(&wallet.treasury, &account.mint, &account.program);
            instructions.push(ix::create_associated_token_account_idempotent(
                &source_key,
                &wallet.treasury,
                &account.mint,
                &account.program,
            ));
            instructions.push(ix::transfer_checked(
                &account.program,
                &account.address,
                &account.mint,
                &destination,
                &source_key,
                account.amount,
                decimals,
            ));
        }
        send(ctx, &source, &instructions, "tokens moved")?;
    }
    let closable: Vec<&TokenAccount> = plan
        .tokens
        .iter()
        .chain(&plan.empty_token_accounts)
        .collect();
    for batch in closable.chunks(TOKENS_PER_TX * 2) {
        let instructions: Vec<Instruction> = batch
            .iter()
            .map(|account| {
                ix::close_account(&account.program, &account.address, &source_key, &source_key)
            })
            .collect();
        if let Err(error) = send(ctx, &source, &instructions, "token accounts closed") {
            eprintln!("could not close some token accounts (funds already moved): {error}");
        }
    }
    for stake in &plan.stakes {
        let instructions = [ix::StakeAuthority::Staker, ix::StakeAuthority::Withdrawer]
            .map(|kind| ix::stake_authorize(stake, &source_key, &wallet.treasury, kind));
        send(ctx, &source, &instructions, "stake re-pointed")?;
    }
    let balance = ctx.rpc.balance(&source_key)?;
    let lamports = balance.saturating_sub(fee);
    if lamports > 0 {
        send(
            ctx,
            &source,
            &[ix::system_transfer(&source_key, &wallet.treasury, lamports)],
            "SOL moved",
        )?;
    }
    println!("sweep complete; qubit: {}", wallet.treasury);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(amount: u64, tag: u8) -> TokenAccount {
        TokenAccount {
            address: Address::new_from_array([tag; 32]),
            program: ix::TOKEN_PROGRAM,
            mint: Address::new_from_array([tag + 1; 32]),
            amount,
        }
    }

    #[test]
    fn partitions_tokens_and_reserves_fee() {
        let plan = plan_from_accounts(
            vec![account(5, 1), account(0, 3)],
            vec![Address::new_from_array([7; 32])],
            1_000_000,
            5_000,
        );
        assert_eq!(plan.tokens, vec![account(5, 1)]);
        assert_eq!(plan.empty_token_accounts, vec![account(0, 3)]);
        assert_eq!(plan.stakes.len(), 1);
        assert_eq!(plan.lamports, 995_000);
        assert_eq!(
            plan_from_accounts(Vec::new(), Vec::new(), 100, 5_000).lamports,
            0
        );
    }
}
