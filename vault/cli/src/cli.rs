use crate::{
    config::{self, Paths},
    keys::Wallet,
    rpc::Rpc,
    tx::TxConfig,
    vault,
};
use anyhow::{anyhow, Context as _, Result};
use clap::{Parser, Subcommand};
use solana_address::Address;
use solana_keypair::{read_keypair_file, Keypair};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "qubit",
    version,
    about = "Hash-locked Solana vault that does not depend on elliptic-curve signatures"
)]
pub struct Cli {
    /// Cluster name (mainnet, devnet, localnet) or RPC URL.
    #[arg(long, global = true, env = "QUBIT_URL", default_value = "mainnet", value_parser = config::resolve_url)]
    pub url: String,
    /// Directory holding seed and pending files (default: ~/.config/qubit).
    #[arg(long, global = true, env = "QUBIT_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,
    /// Keypair that pays transaction fees (default: ~/.config/solana/id.json).
    #[arg(long, global = true, env = "QUBIT_FEE_PAYER")]
    pub fee_payer: Option<PathBuf>,
    /// Extra priority fee for each transaction, in lamports (V1 transactions take a total, not a per-CU price).
    #[arg(long, global = true)]
    pub priority_fee: Option<u64>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Generate a new 24-word seed and store it locally.
    Keygen {
        /// Protect the seed with a BIP-39 passphrase (prompted).
        #[arg(long)]
        passphrase: bool,
    },
    #[command(flatten)]
    Online(OnlineCommand),
}

#[derive(Subcommand)]
pub enum OnlineCommand {
    /// Create the vault for this seed on the selected cluster.
    Create,
    /// Print the qubit (treasury) and vault addresses.
    Address,
    /// Show SOL and token balances held by the qubit.
    Balance,
    /// Show cluster, vault and pending-operation status.
    Status,
    /// Send SOL or tokens out of the qubit.
    Send {
        /// Amount in SOL or whole token units.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        amount: Option<String>,
        /// Destination wallet address.
        #[arg(long)]
        to: Address,
        /// Token mint; omit for SOL.
        #[arg(long)]
        mint: Option<Address>,
        /// Send the whole balance.
        #[arg(long)]
        all: bool,
    },
    /// Retry the pending operation, or rotate past it with --recover.
    Resume {
        /// Rotate the key without executing the pending instruction.
        #[arg(long)]
        recover: bool,
    },
    /// Move everything from a hot wallet into the qubit.
    Sweep {
        /// Keypair file of the wallet to empty.
        #[arg(long)]
        from: PathBuf,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Restore the seed from 24 words typed on stdin.
    Recover,
}

pub struct Context {
    pub paths: Paths,
    pub rpc: Rpc,
    pub fee_payer_path: PathBuf,
    pub priority_fee: Option<u64>,
    pub genesis_hash: [u8; 32],
}

impl Context {
    fn new(
        paths: Paths,
        url: String,
        fee_payer: Option<PathBuf>,
        priority_fee: Option<u64>,
    ) -> Result<Self> {
        let rpc = Rpc::new(url);
        let genesis_hash = rpc.genesis_hash()?.to_bytes();
        vault::check_v1(&rpc)?;
        let fee_payer_path = match fee_payer {
            Some(path) => path,
            None => config::default_fee_payer()?,
        };
        Ok(Self {
            paths,
            rpc,
            fee_payer_path,
            priority_fee,
            genesis_hash,
        })
    }

    pub fn wallet(&self) -> Result<Wallet> {
        let seed = config::read_seed_file(&self.paths.seed())?;
        let mnemonic = config::parse_mnemonic(&seed.mnemonic).context("seed file is damaged")?;
        let passphrase = if seed.passphrase_protected {
            rpassword::prompt_password("BIP-39 passphrase: ")?
        } else {
            String::new()
        };
        Ok(Wallet::derive(&mnemonic, &passphrase, &self.genesis_hash))
    }

    pub fn fee_payer(&self) -> Result<Keypair> {
        read_keypair_file(&self.fee_payer_path).map_err(|error| {
            anyhow!(
                "cannot read fee payer {}: {error}",
                self.fee_payer_path.display()
            )
        })
    }

    pub fn tx_config(&self, compute_unit_limit: u32) -> TxConfig {
        TxConfig {
            compute_unit_limit,
            priority_fee: self.priority_fee,
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::new(cli.config_dir)?;
    match cli.command {
        Command::Keygen { passphrase } => crate::commands::keygen::run(&paths, passphrase),
        Command::Online(command) => {
            let ctx = Context::new(paths, cli.url, cli.fee_payer, cli.priority_fee)
                .context("cannot connect to the cluster")?;
            online(&ctx, command)
        }
    }
}

fn online(ctx: &Context, command: OnlineCommand) -> Result<()> {
    use crate::commands::*;
    match command {
        OnlineCommand::Create => create::run(ctx),
        OnlineCommand::Address => address::run(ctx),
        OnlineCommand::Balance => balance::run(ctx),
        OnlineCommand::Status => status::run(ctx),
        OnlineCommand::Send {
            amount,
            to,
            mint,
            all,
        } => send::run(ctx, amount.as_deref(), &to, mint.as_ref(), all),
        OnlineCommand::Resume { recover } => resume::run(ctx, recover),
        OnlineCommand::Sweep { from, yes } => sweep::run(ctx, &from, yes),
        OnlineCommand::Recover => recover::run(ctx),
    }
}
