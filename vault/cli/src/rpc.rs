use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use solana_address::Address;
use solana_hash::Hash;
use std::str::FromStr;

pub struct Rpc {
    url: String,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct Envelope<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct WithContext<T> {
    value: T,
}

#[derive(Deserialize)]
struct RawAccount {
    owner: String,
    data: (String, String),
}

#[derive(Deserialize)]
struct KeyedRaw {
    pubkey: String,
    account: RawAccount,
}

#[derive(Clone, Debug)]
pub struct AccountInfo {
    pub owner: Address,
    pub data: Vec<u8>,
}

impl TryFrom<RawAccount> for AccountInfo {
    type Error = anyhow::Error;

    fn try_from(raw: RawAccount) -> Result<Self> {
        Ok(Self {
            owner: parse_address(&raw.owner)?,
            data: STANDARD
                .decode(raw.data.0)
                .context("account data is not base64")?,
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct Simulation {
    pub err: Option<Value>,
    pub logs: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignatureStatus {
    pub confirmation_status: Option<String>,
    pub err: Option<Value>,
}

pub fn parse_address(text: &str) -> Result<Address> {
    Address::from_str(text).map_err(|error| anyhow!("invalid address {text}: {error}"))
}

pub fn parse_hash(text: &str) -> Result<Hash> {
    Hash::from_str(text).map_err(|error| anyhow!("invalid hash {text}: {error}"))
}

impl Rpc {
    pub fn new(url: String) -> Self {
        Self {
            url,
            agent: ureq::Agent::new_with_defaults(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let envelope: Envelope<T> = self
            .agent
            .post(&self.url)
            .send_json(&body)
            .with_context(|| format!("{method} request to {} failed", self.url))?
            .body_mut()
            .read_json()
            .with_context(|| format!("{method}: invalid JSON-RPC response"))?;
        if let Some(error) = envelope.error {
            bail!("{method}: RPC error {}: {}", error.code, error.message);
        }
        envelope
            .result
            .ok_or_else(|| anyhow!("{method}: empty result"))
    }

    pub fn genesis_hash(&self) -> Result<Hash> {
        let text: String = self.call("getGenesisHash", json!([]))?;
        parse_hash(&text)
    }

    pub fn latest_blockhash(&self) -> Result<Hash> {
        #[derive(Deserialize)]
        struct Blockhash {
            blockhash: String,
        }
        let result: WithContext<Blockhash> =
            self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        parse_hash(&result.value.blockhash)
    }

    pub fn is_blockhash_valid(&self, blockhash: &Hash) -> Result<bool> {
        let result: WithContext<bool> = self.call(
            "isBlockhashValid",
            json!([blockhash.to_string(), { "commitment": "confirmed" }]),
        )?;
        Ok(result.value)
    }

    pub fn account(&self, address: &Address) -> Result<Option<AccountInfo>> {
        let result: WithContext<Option<RawAccount>> = self.call(
            "getAccountInfo",
            json!([address.to_string(), { "encoding": "base64", "commitment": "confirmed" }]),
        )?;
        result.value.map(AccountInfo::try_from).transpose()
    }

    pub fn balance(&self, address: &Address) -> Result<u64> {
        let result: WithContext<u64> = self.call(
            "getBalance",
            json!([address.to_string(), { "commitment": "confirmed" }]),
        )?;
        Ok(result.value)
    }

    pub fn minimum_balance(&self, data_len: usize) -> Result<u64> {
        self.call("getMinimumBalanceForRentExemption", json!([data_len]))
    }

    pub fn token_accounts(
        &self,
        owner: &Address,
        program: &Address,
    ) -> Result<Vec<(Address, AccountInfo)>> {
        let result: WithContext<Vec<KeyedRaw>> = self.call(
            "getTokenAccountsByOwner",
            json!([owner.to_string(), { "programId": program.to_string() }, { "encoding": "base64", "commitment": "confirmed" }]),
        )?;
        keyed(result.value)
    }

    pub fn program_accounts(
        &self,
        program: &Address,
        filters: Value,
    ) -> Result<Vec<(Address, AccountInfo)>> {
        let result: Vec<KeyedRaw> = self.call(
            "getProgramAccounts",
            json!([program.to_string(), { "encoding": "base64", "commitment": "confirmed", "filters": filters }]),
        )?;
        keyed(result)
    }

    pub fn simulate(&self, transaction: &str) -> Result<Simulation> {
        let result: WithContext<Simulation> = self.call(
            "simulateTransaction",
            json!([transaction, { "encoding": "base64", "commitment": "confirmed" }]),
        )?;
        Ok(result.value)
    }

    pub fn send(&self, transaction: &str) -> Result<String> {
        self.call(
            "sendTransaction",
            json!([transaction, { "encoding": "base64", "skipPreflight": true, "maxRetries": 3 }]),
        )
    }

    pub fn signature_status(&self, signature: &str) -> Result<Option<SignatureStatus>> {
        let result: WithContext<Vec<Option<SignatureStatus>>> = self.call(
            "getSignatureStatuses",
            json!([[signature], { "searchTransactionHistory": true }]),
        )?;
        Ok(result.value.into_iter().next().flatten())
    }

    pub fn feature_active(&self, feature: &Address) -> Result<bool> {
        Ok(self
            .account(feature)?
            .is_some_and(|account| account.data.first() == Some(&1)))
    }
}

fn keyed(raw: Vec<KeyedRaw>) -> Result<Vec<(Address, AccountInfo)>> {
    raw.into_iter()
        .map(|entry| {
            Ok((
                parse_address(&entry.pubkey)?,
                AccountInfo::try_from(entry.account)?,
            ))
        })
        .collect()
}
