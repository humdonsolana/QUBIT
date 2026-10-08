use crate::{
    ix,
    rpc::{AccountInfo, Rpc},
};
use anyhow::{anyhow, bail, Result};
use solana_address::Address;

pub const TOKEN_ACCOUNT_LEN: usize = 165;
const MINT_DECIMALS_OFFSET: usize = 44;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAccount {
    pub address: Address,
    pub program: Address,
    pub mint: Address,
    pub amount: u64,
}

/// Decodes the fixed prefix shared by Token and Token-2022 accounts.
pub fn parse_token_account(
    address: Address,
    program: Address,
    data: &[u8],
) -> Option<TokenAccount> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return None;
    }
    Some(TokenAccount {
        address,
        program,
        mint: Address::new_from_array(data[..32].try_into().ok()?),
        amount: u64::from_le_bytes(data[64..72].try_into().ok()?),
    })
}

pub fn list_token_accounts(rpc: &Rpc, owner: &Address) -> Result<Vec<TokenAccount>> {
    let mut out = Vec::new();
    for program in [ix::TOKEN_PROGRAM, ix::TOKEN_2022_PROGRAM] {
        for (address, info) in rpc.token_accounts(owner, &program)? {
            let account = parse_token_account(address, program, &info.data).ok_or_else(|| {
                anyhow!("token account {address} is shorter than {TOKEN_ACCOUNT_LEN} bytes")
            })?;
            out.push(account);
        }
    }
    Ok(out)
}

/// Returns `(token_program, decimals)` for a mint.
pub fn mint_info(rpc: &Rpc, mint: &Address) -> Result<(Address, u8)> {
    let info: AccountInfo = rpc
        .account(mint)?
        .ok_or_else(|| anyhow!("mint {mint} does not exist"))?;
    if info.owner != ix::TOKEN_PROGRAM && info.owner != ix::TOKEN_2022_PROGRAM {
        bail!("{mint} is not a token mint");
    }
    let decimals = *info
        .data
        .get(MINT_DECIMALS_OFFSET)
        .ok_or_else(|| anyhow!("mint {mint} data is too short"))?;
    Ok((info.owner, decimals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_account_prefix() {
        let mut data = vec![0u8; TOKEN_ACCOUNT_LEN];
        data[..32].copy_from_slice(&[1; 32]);
        data[64..72].copy_from_slice(&500u64.to_le_bytes());
        let parsed =
            parse_token_account(Address::new_from_array([3; 32]), ix::TOKEN_PROGRAM, &data)
                .unwrap();
        assert_eq!(parsed.mint, Address::new_from_array([1; 32]));
        assert_eq!(parsed.amount, 500);
        assert!(parse_token_account(parsed.address, ix::TOKEN_PROGRAM, &data[..100]).is_none());
    }
}
