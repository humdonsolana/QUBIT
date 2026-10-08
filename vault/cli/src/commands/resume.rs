use crate::{
    cli::Context,
    commands::execute,
    pending::{self, Resolution},
    vault,
};
use anyhow::{bail, Result};

pub fn run(ctx: &Context, recover: bool) -> Result<()> {
    let wallet = ctx.wallet()?;
    let Some(pending) = pending::load(&ctx.paths, &wallet.vault)? else {
        println!("nothing pending");
        return Ok(());
    };
    let state = vault::require(&ctx.rpc, &wallet.vault)?;
    match pending::resolve(pending.key_index, state.key_index) {
        Resolution::Clear => {
            pending::clear(&ctx.paths, &wallet.vault)?;
            println!("key index {} was already consumed on-chain; pending file removed", pending.key_index);
            Ok(())
        }
        Resolution::Resume if recover => execute::submit_recover(ctx, &wallet, &pending),
        Resolution::Resume => execute::submit(ctx, &wallet, &pending),
        Resolution::Corrupt => bail!(
            "pending file is at key index {} but the chain is at {}; this file does not belong to the current vault state, move it away manually",
            pending.key_index,
            state.key_index
        ),
    }
}
