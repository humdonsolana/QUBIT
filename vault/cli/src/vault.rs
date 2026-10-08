use crate::{keys::PROGRAM_ID, rpc::Rpc, tx::V1_FEATURE};
use anyhow::{anyhow, bail, Result};
use qubit_interface::Vault;
use solana_address::Address;

pub fn fetch(rpc: &Rpc, vault: &Address) -> Result<Option<Vault>> {
    let Some(info) = rpc.account(vault)? else {
        return Ok(None);
    };
    if info.owner != PROGRAM_ID {
        bail!("account {vault} is not owned by the qubit program");
    }
    Vault::from_bytes(&info.data)
        .map(Some)
        .ok_or_else(|| anyhow!("vault {vault} has an unknown layout"))
}

pub fn require(rpc: &Rpc, vault: &Address) -> Result<Vault> {
    fetch(rpc, vault)?.ok_or_else(|| {
        anyhow!(
            "no vault for this seed on {}; run `qubit create`",
            rpc.url()
        )
    })
}

pub fn check_v1(rpc: &Rpc) -> Result<()> {
    if !rpc.feature_active(&V1_FEATURE)? {
        bail!(
            "{} has not activated V1 transactions (SIMD-0385); qubit needs them",
            rpc.url()
        );
    }
    Ok(())
}
