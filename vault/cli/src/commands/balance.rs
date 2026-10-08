use crate::{amount::format_amount, cli::Context, commands::token};
use anyhow::Result;

pub fn run(ctx: &Context) -> Result<()> {
    let wallet = ctx.wallet()?;
    let lamports = ctx.rpc.balance(&wallet.treasury)?;
    println!("qubit:  {}", wallet.treasury);
    println!("SOL: {}", format_amount(lamports, 9));
    for account in token::list_token_accounts(&ctx.rpc, &wallet.treasury)? {
        let (_, decimals) = token::mint_info(&ctx.rpc, &account.mint)?;
        println!(
            "{}: {}",
            account.mint,
            format_amount(account.amount, decimals)
        );
    }
    Ok(())
}
