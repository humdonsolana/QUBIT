use bip39::Mnemonic;
use qubit_interface::{vault_id, ID, TREASURY_SEED, VAULT_SEED};
use qubit_wots::{
    public_key, public_key_hash, secret_key, sign, Elements, Hash, HostSha256, Sha256, Tweak, LEN,
    N, PREFIX,
};
use solana_address::Address;
use zeroize::Zeroizing;

pub const DOMAIN_ROOT: u8 = 0x06;
pub const DOMAIN_SEED: u8 = 0x07;

pub const PROGRAM_ID: Address = Address::new_from_array(ID);

pub struct Wallet {
    seed: Zeroizing<[u8; 32]>,
    pub root: [u8; 32],
    pub vault: Address,
    pub treasury: Address,
}

impl Wallet {
    /// Derives the cluster-specific wallet from a mnemonic, passphrase and genesis hash.
    pub fn derive(mnemonic: &Mnemonic, passphrase: &str, genesis_hash: &[u8; 32]) -> Self {
        let bip39_seed = Zeroizing::new(mnemonic.to_seed(passphrase));
        let seed = Zeroizing::new(HostSha256::hashv(&[
            PREFIX,
            &[DOMAIN_SEED],
            bip39_seed.as_slice(),
            genesis_hash,
        ]));
        let root = HostSha256::hashv(&[PREFIX, &[DOMAIN_ROOT], seed.as_slice()]);
        let first = Self::public_key_hash_for(
            &seed,
            &Tweak {
                vault: root,
                key_index: 0,
            },
        );
        let id = vault_id::<HostSha256>(&root, &first);
        let (vault, _) = Address::find_program_address(&[VAULT_SEED, &id], &PROGRAM_ID);
        let (treasury, _) =
            Address::find_program_address(&[TREASURY_SEED, vault.as_array()], &PROGRAM_ID);
        Self {
            seed,
            root,
            vault,
            treasury,
        }
    }

    fn public_key_hash_for(seed: &[u8; 32], tweak: &Tweak) -> Hash {
        let mut secret = Zeroizing::new([[0u8; N]; LEN]);
        secret_key::<HostSha256>(seed, tweak, &mut secret);
        let mut public = [[0u8; N]; LEN];
        public_key::<HostSha256>(tweak, &secret, &mut public);
        public_key_hash::<HostSha256>(tweak, &public)
    }

    pub fn tweak(&self, key_index: u64) -> Tweak {
        Tweak {
            vault: self.root,
            key_index,
        }
    }

    fn secret(&self, key_index: u64) -> Zeroizing<Elements> {
        let mut out = Zeroizing::new([[0u8; N]; LEN]);
        secret_key::<HostSha256>(&self.seed, &self.tweak(key_index), &mut out);
        out
    }

    pub fn public_key_hash(&self, key_index: u64) -> Hash {
        Self::public_key_hash_for(&self.seed, &self.tweak(key_index))
    }

    pub fn sign(&self, key_index: u64, message: &Hash) -> Vec<u8> {
        let secret = self.secret(key_index);
        let mut signature = [[0u8; N]; LEN];
        sign::<HostSha256>(&self.tweak(key_index), &secret, message, &mut signature);
        signature.as_flattened().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bip39::Language;
    use qubit_wots::verify;

    const WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    fn wallet(passphrase: &str, genesis: u8) -> Wallet {
        Wallet::derive(
            &Mnemonic::parse_in_normalized(Language::English, WORDS).unwrap(),
            passphrase,
            &[genesis; 32],
        )
    }

    #[test]
    fn derivation_is_deterministic_and_cluster_bound() {
        let a = wallet("", 1);
        let b = wallet("", 1);
        assert_eq!(a.vault, b.vault);
        assert_eq!(a.treasury, b.treasury);
        assert_eq!(a.public_key_hash(0), b.public_key_hash(0));
        assert_ne!(a.vault, wallet("", 2).vault);
        assert_ne!(a.vault, wallet("x", 1).vault);
        assert_ne!(a.public_key_hash(0), a.public_key_hash(1));
    }

    #[test]
    fn signatures_verify_against_published_hash() {
        let w = wallet("", 1);
        let message = [9u8; 32];
        let signature = w.sign(5, &message);
        assert!(verify::<HostSha256>(
            &w.tweak(5),
            &w.public_key_hash(5),
            &message,
            &signature
        ));
        assert!(!verify::<HostSha256>(
            &w.tweak(6),
            &w.public_key_hash(6),
            &message,
            &signature
        ));
    }
}
