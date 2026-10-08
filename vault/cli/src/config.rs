use anyhow::{anyhow, bail, Context, Result};
use bip39::{Language, Mnemonic};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAINNET_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEVNET_URL: &str = "https://api.devnet.solana.com";
pub const LOCALNET_URL: &str = "http://127.0.0.1:8899";

pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new(dir: Option<PathBuf>) -> Result<Self> {
        let dir = match dir {
            Some(dir) => dir,
            None => home()?.join(".config").join("qubit"),
        };
        Ok(Self { dir })
    }

    pub fn seed(&self) -> PathBuf {
        self.dir.join("seed.json")
    }

    pub fn pending(&self, vault: &Address) -> PathBuf {
        self.dir.join(format!("pending-{vault}.json"))
    }
}

pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

pub fn default_fee_payer() -> Result<PathBuf> {
    Ok(home()?.join(".config").join("solana").join("id.json"))
}

pub fn resolve_url(name: &str) -> Result<String> {
    Ok(match name {
        "mainnet" | "mainnet-beta" => MAINNET_URL.to_owned(),
        "devnet" => DEVNET_URL.to_owned(),
        "localnet" | "localhost" => LOCALNET_URL.to_owned(),
        url if url.starts_with("http://") || url.starts_with("https://") => url.to_owned(),
        other => bail!("unknown cluster or URL: {other}"),
    })
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Serialize, Deserialize)]
pub struct SeedFile {
    pub version: u8,
    pub mnemonic: String,
    pub passphrase_protected: bool,
    pub created_at: u64,
}

impl SeedFile {
    pub fn new(mnemonic: String, passphrase_protected: bool) -> Self {
        Self {
            version: 1,
            mnemonic,
            passphrase_protected,
            created_at: now(),
        }
    }
}

/// Creates the seed file with mode 0600; fails if it already exists.
pub fn write_seed_file(path: &Path, seed: &SeedFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).with_context(|| {
        format!(
            "cannot create {}; a seed file already exists",
            path.display()
        )
    })?;
    file.write_all(serde_json::to_string_pretty(seed)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub fn read_seed_file(path: &Path) -> Result<SeedFile> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "no seed file at {}; run `qubit keygen` or `qubit recover`",
            path.display()
        )
    })?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a valid seed file", path.display()))
}

/// Parses a mnemonic; bip39 itself splits on any whitespace.
pub fn parse_mnemonic(text: &str) -> Result<Mnemonic> {
    Mnemonic::parse_in_normalized(Language::English, text)
        .map_err(|error| anyhow!("invalid mnemonic: {error}"))
}

/// Creates `path` with `bytes` durably; fails if it already exists and never leaves a partial file.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let linked = fs::hard_link(&tmp, path);
    fs::remove_file(&tmp)?;
    linked.with_context(|| format!("{} already exists", path.display()))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_cluster_names_and_urls() {
        assert_eq!(resolve_url("mainnet").unwrap(), MAINNET_URL);
        assert_eq!(resolve_url("devnet").unwrap(), DEVNET_URL);
        assert_eq!(resolve_url("localnet").unwrap(), LOCALNET_URL);
        assert_eq!(
            resolve_url("https://rpc.example").unwrap(),
            "https://rpc.example"
        );
        assert!(resolve_url("testnet-ish").is_err());
    }

    #[test]
    fn mnemonic_parsing_ignores_extra_whitespace() {
        let words = "abandon ".repeat(23) + "art";
        let messy = format!("  {}\t\n", words.replace(' ', "   "));
        assert_eq!(parse_mnemonic(&messy).unwrap().to_string(), words);
    }

    #[test]
    fn seed_file_is_created_once() {
        let dir = std::env::temp_dir().join(format!("qubit-test-{}", std::process::id()));
        let path = dir.join("seed.json");
        let seed = SeedFile::new("abandon ".repeat(23) + "art", false);
        write_seed_file(&path, &seed).unwrap();
        assert!(write_seed_file(&path, &seed).is_err());
        assert_eq!(read_seed_file(&path).unwrap().mnemonic, seed.mnemonic);
        fs::remove_dir_all(dir).unwrap();
    }
}
