use crate::{
    cli::Context,
    ix,
    tx::{self, CU_CREATE},
    vault,
};
use anyhow::{bail, Result};
use solana_signer::Signer;

pub fn run(ctx: &Context) -> Result<()> {
    let wallet = ctx.wallet()?;
    if vault::fetch(&ctx.rpc, &wallet.vault)?.is_some() {
        bail!(
            "vault {} already exists; qubit address: {}",
            wallet.vault,
            wallet.treasury
        );
    }
    let payer = ctx.fee_payer()?;
    let instruction = ix::vault_create(
        &payer.pubkey(),
        &wallet.vault,
        &wallet.treasury,
        &wallet.root,
        &wallet.public_key_hash(0),
    );
    let signature = tx::submit(&ctx.rpc, &payer, &[instruction], &ctx.tx_config(CU_CREATE))?;
    println!("vault created: {signature}");
    println!("qubit address (send funds here): {}", wallet.treasury);
    println!("vault state account:              {}", wallet.vault);
    Ok(())
}
