use crate::{cli::Context, pending, vault};
use anyhow::Result;

pub fn run(ctx: &Context) -> Result<()> {
    let wallet = ctx.wallet()?;
    println!("rpc:     {}", ctx.rpc.url());
    println!("config:  {}", ctx.paths.dir.display());
    println!("qubit:    {}", wallet.treasury);
    println!("vault:   {}", wallet.vault);
    match vault::fetch(&ctx.rpc, &wallet.vault)? {
        None => println!("state:   not created on this cluster (run `qubit create`)"),
        Some(state) => {
            let matches = state.current_pk_hash == wallet.public_key_hash(state.key_index);
            println!(
                "state:   key index {}, next key {}",
                state.key_index,
                if matches {
                    "matches this seed"
                } else {
                    "DOES NOT MATCH this seed"
                }
            );
        }
    }
    match pending::load(&ctx.paths, &wallet.vault)? {
        None => println!("pending: none"),
        Some(p) => println!("pending: key index {} (run `qubit resume`)", p.key_index),
    }
    Ok(())
}
