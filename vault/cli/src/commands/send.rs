use crate::{
    amount::parse_amount,
    cli::Context,
    commands::{execute, token},
    ix,
    keys::Wallet,
    pending, vault,
};
use anyhow::{anyhow, bail, Result};
use solana_address::Address;
use solana_instruction::Instruction;
use solana_signer::Signer;

fn pick_amount(balance: u64, requested: Option<u64>, all: bool) -> Result<u64> {
    let amount = if all {
        balance
    } else {
        requested.ok_or_else(|| anyhow!("give an amount or --all"))?
    };
    if amount == 0 {
        bail!("nothing to send");
    }
    if amount > balance {
        bail!("qubit holds {balance} base units, cannot send {amount}");
    }
    Ok(amount)
}

/// Chooses the lamports to move so the treasury ends at zero or stays rent exempt.
pub fn plan_sol_amount(
    balance: u64,
    minimum: u64,
    requested: Option<u64>,
    all: bool,
) -> Result<u64> {
    let amount = pick_amount(balance, requested, all)?;
    let remaining = balance - amount;
    if remaining > 0 && remaining < minimum {
        bail!("sending {amount} would leave {remaining} lamports, below the rent-exempt minimum of {minimum}; send less or use --all");
    }
    Ok(amount)
}

pub fn run(
    ctx: &Context,
    amount: Option<&str>,
    to: &Address,
    mint: Option<&Address>,
    all: bool,
) -> Result<()> {
    let wallet = ctx.wallet()?;
    let state = vault::require(&ctx.rpc, &wallet.vault)?;
    if pending::load(&ctx.paths, &wallet.vault)?.is_some() {
        bail!("a pending operation exists; run `qubit resume` or `qubit resume --recover` first");
    }
    let (prelude, inner) = match mint {
        None => (Vec::new(), sol(ctx, &wallet, amount, to, all)?),
        Some(mint) => token(ctx, &wallet, amount, to, mint, all)?,
    };
    execute::begin(ctx, &wallet, state.key_index, prelude, inner)
}

fn sol(
    ctx: &Context,
    wallet: &Wallet,
    amount: Option<&str>,
    to: &Address,
    all: bool,
) -> Result<Instruction> {
    let balance = ctx.rpc.balance(&wallet.treasury)?;
    let minimum = ctx.rpc.minimum_balance(0)?;
    let requested = amount.map(|text| parse_amount(text, 9)).transpose()?;
    let lamports = plan_sol_amount(balance, minimum, requested, all)?;
    if ctx.rpc.account(to)?.is_none() && lamports < minimum {
        bail!("{to} does not exist yet; the first transfer must be at least {minimum} lamports");
    }
    Ok(ix::system_transfer(&wallet.treasury, to, lamports))
}

fn token(
    ctx: &Context,
    wallet: &Wallet,
    amount: Option<&str>,
    to: &Address,
    mint: &Address,
    all: bool,
) -> Result<(Vec<Instruction>, Instruction)> {
    let (program, decimals) = token::mint_info(&ctx.rpc, mint)?;
    let source = ix::associated_token_address(&wallet.treasury, mint, &program);
    let balance = token::list_token_accounts(&ctx.rpc, &wallet.treasury)?
        .into_iter()
        .find(|account| account.address == source)
        .map_or(0, |account| account.amount);
    let requested = amount
        .map(|text| parse_amount(text, decimals))
        .transpose()?;
    let units = pick_amount(balance, requested, all)?;
    let payer = ctx.fee_payer()?.pubkey();
    let destination = ix::associated_token_address(to, mint, &program);
    let prelude = vec![ix::create_associated_token_account_idempotent(
        &payer, to, mint, &program,
    )];
    let inner = ix::transfer_checked(
        &program,
        &source,
        mint,
        &destination,
        &wallet.treasury,
        units,
        decimals,
    );
    Ok((prelude, inner))
}

#[cfg(test)]
mod tests {
    use super::plan_sol_amount;

    #[test]
    fn respects_rent_rules() {
        let min = 890_880;
        assert_eq!(
            plan_sol_amount(5_000_000, min, Some(1_000_000), false).unwrap(),
            1_000_000
        );
        assert_eq!(
            plan_sol_amount(5_000_000, min, None, true).unwrap(),
            5_000_000
        );
        assert_eq!(
            plan_sol_amount(5_000_000, min, Some(5_000_000), false).unwrap(),
            5_000_000
        );
        assert!(plan_sol_amount(5_000_000, min, Some(4_500_000), false).is_err());
        assert!(plan_sol_amount(5_000_000, min, Some(6_000_000), false).is_err());
        assert!(plan_sol_amount(5_000_000, min, Some(0), false).is_err());
        assert!(plan_sol_amount(5_000_000, min, None, false).is_err());
    }
}
