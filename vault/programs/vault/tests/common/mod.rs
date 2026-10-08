#![allow(dead_code)]

use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use qubit_interface::{vault_id, Vault, ID, TREASURY_SEED, VAULT_SEED};
use qubit_wots::{
    public_key, public_key_hash, secret_key, sign, Elements, Hash, Sha256, Tweak, LEN, N,
};
use solana_account::Account;
use solana_pubkey::Pubkey;

pub use qubit_wots::HostSha256 as Host;

pub const ELF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/deploy/qubit_vault.so"
);

pub fn program_id() -> Pubkey {
    Pubkey::new_from_array(ID)
}

pub fn mollusk() -> Mollusk {
    let elf = std::fs::read(ELF).expect("run scripts/build-program.sh first");
    let mut mollusk = Mollusk::default();
    mollusk.add_program_with_loader_and_elf(&program_id(), &LOADER_V3, &elf);
    mollusk
}

pub struct Keys {
    pub seed: [u8; 32],
    pub root: [u8; 32],
    pub vault: Pubkey,
    pub vault_bump: u8,
    pub treasury: Pubkey,
    pub treasury_bump: u8,
}

pub fn keys(tag: u8) -> Keys {
    let seed = [tag; 32];
    let root = Host::hashv(&[b"root", &seed]);
    let first = public_key_hash_for(
        &seed,
        &Tweak {
            vault: root,
            key_index: 0,
        },
    );
    let id = vault_id::<Host>(&root, &first);
    let (vault, vault_bump) = Pubkey::find_program_address(&[VAULT_SEED, &id], &program_id());
    let (treasury, treasury_bump) =
        Pubkey::find_program_address(&[TREASURY_SEED, vault.as_ref()], &program_id());
    Keys {
        seed,
        root,
        vault,
        vault_bump,
        treasury,
        treasury_bump,
    }
}

fn keypair_for(seed: &[u8; 32], tweak: &Tweak) -> (Elements, Elements) {
    let mut secret = [[0u8; N]; LEN];
    secret_key::<Host>(seed, tweak, &mut secret);
    let mut public = [[0u8; N]; LEN];
    public_key::<Host>(tweak, &secret, &mut public);
    (secret, public)
}

fn public_key_hash_for(seed: &[u8; 32], tweak: &Tweak) -> Hash {
    public_key_hash::<Host>(tweak, &keypair_for(seed, tweak).1)
}

impl Keys {
    pub fn tweak(&self, key_index: u64) -> Tweak {
        Tweak {
            vault: self.root,
            key_index,
        }
    }

    fn keypair(&self, key_index: u64) -> (Elements, Elements) {
        keypair_for(&self.seed, &self.tweak(key_index))
    }

    pub fn pk_hash(&self, key_index: u64) -> Hash {
        let (_, public) = self.keypair(key_index);
        public_key_hash::<Host>(&self.tweak(key_index), &public)
    }

    pub fn sign(&self, key_index: u64, message: &Hash) -> Vec<u8> {
        let (secret, _) = self.keypair(key_index);
        let mut signature = [[0u8; N]; LEN];
        sign::<Host>(&self.tweak(key_index), &secret, message, &mut signature);
        signature.as_flattened().to_vec()
    }

    pub fn state(&self, key_index: u64) -> Vault {
        Vault {
            vault_bump: self.vault_bump,
            treasury_bump: self.treasury_bump,
            key_index,
            current_pk_hash: self.pk_hash(key_index),
            root: self.root,
        }
    }
}

pub fn vault_account(state: &Vault, lamports: u64) -> Account {
    Account {
        lamports,
        data: state.to_bytes().to_vec(),
        owner: program_id(),
        executable: false,
        rent_epoch: 0,
    }
}

pub fn system_account(lamports: u64) -> Account {
    Account {
        lamports,
        data: Vec::new(),
        owner: Pubkey::default(),
        executable: false,
        rent_epoch: 0,
    }
}

pub fn system_transfer_data(lamports: u64) -> Vec<u8> {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    data
}
