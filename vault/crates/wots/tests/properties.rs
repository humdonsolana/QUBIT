use proptest::prelude::*;
use qubit_wots::{
    digits, public_key, public_key_hash, secret_key, sign, verify, Elements, Hash, HostSha256,
    Tweak, LEN, LEN1, N, SIGNATURE_LEN, W,
};

fn keypair(seed: &[u8; 32], tweak: &Tweak) -> (Elements, Hash) {
    let mut secret = [[0u8; N]; LEN];
    secret_key::<HostSha256>(seed, tweak, &mut secret);
    let mut public = [[0u8; N]; LEN];
    public_key::<HostSha256>(tweak, &secret, &mut public);
    (secret, public_key_hash::<HostSha256>(tweak, &public))
}

fn signature(secret: &Elements, tweak: &Tweak, message: &Hash) -> Vec<u8> {
    let mut out = [[0u8; N]; LEN];
    sign::<HostSha256>(tweak, secret, message, &mut out);
    out.as_flattened().to_vec()
}

proptest! {
    #[test]
    fn sign_then_verify(
        seed in any::<[u8; 32]>(),
        vault in any::<[u8; 32]>(),
        key_index in any::<u64>(),
        message in any::<[u8; 32]>(),
    ) {
        let tweak = Tweak { vault, key_index };
        let (secret, hash) = keypair(&seed, &tweak);
        let bytes = signature(&secret, &tweak, &message);
        prop_assert!(verify::<HostSha256>(&tweak, &hash, &message, &bytes));
    }

    #[test]
    fn corrupted_signature_fails(
        seed in any::<[u8; 32]>(),
        vault in any::<[u8; 32]>(),
        message in any::<[u8; 32]>(),
        position in 0..SIGNATURE_LEN,
        bit in 0u8..8,
    ) {
        let tweak = Tweak { vault, key_index: 3 };
        let (secret, hash) = keypair(&seed, &tweak);
        let mut bytes = signature(&secret, &tweak, &message);
        bytes[position] ^= 1 << bit;
        prop_assert!(!verify::<HostSha256>(&tweak, &hash, &message, &bytes));
    }

    #[test]
    fn other_context_fails(
        seed in any::<[u8; 32]>(),
        vault in any::<[u8; 32]>(),
        message in any::<[u8; 32]>(),
        other in any::<[u8; 32]>(),
    ) {
        prop_assume!(other != message && other != vault);
        let tweak = Tweak { vault, key_index: 0 };
        let (secret, hash) = keypair(&seed, &tweak);
        let bytes = signature(&secret, &tweak, &message);
        prop_assert!(!verify::<HostSha256>(&tweak, &hash, &other, &bytes));
        let next_index = Tweak { vault, key_index: 1 };
        prop_assert!(!verify::<HostSha256>(&next_index, &hash, &message, &bytes));
        let other_vault = Tweak { vault: other, key_index: 0 };
        prop_assert!(!verify::<HostSha256>(&other_vault, &hash, &message, &bytes));
        prop_assert!(!verify::<HostSha256>(&tweak, &hash, &message, &bytes[..SIGNATURE_LEN - 1]));
    }

    #[test]
    fn checksum_matches_definition(message in any::<[u8; 32]>()) {
        let d = digits(&message);
        let checksum: u16 = d[..LEN1].iter().map(|&x| u16::from(W - 1 - x)).sum();
        let encoded = (u16::from(d[LEN1]) << 8) | (u16::from(d[LEN1 + 1]) << 4) | u16::from(d[LEN1 + 2]);
        prop_assert_eq!(checksum, encoded);
        prop_assert!(d.iter().all(|&x| x < W));
    }
}
