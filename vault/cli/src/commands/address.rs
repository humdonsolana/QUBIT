use crate::cli::Context;
use anyhow::Result;

pub fn run(ctx: &Context) -> Result<()> {
    let wallet = ctx.wallet()?;
    println!("qubit:  {}", wallet.treasury);
    println!("vault: {}", wallet.vault);
    Ok(())
}
