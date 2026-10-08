use qubit_wots::{
    digits, public_key, public_key_hash, secret_key, sign, verify, Elements, HostSha256, Tweak,
    LEN, N, SIGNATURE_LEN,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    label: String,
    seed: String,
    vault: String,
    key_index: u64,
    message: String,
    digits: Vec<u8>,
    secret_key: String,
    public_key: String,
    public_key_hash: String,
    signature: String,
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn hex32(text: &str) -> [u8; 32] {
    hex(text).try_into().expect("32 bytes")
}

fn elements(bytes: &[u8]) -> Elements {
    let mut out = [[0u8; N]; LEN];
    for (dst, src) in out.iter_mut().zip(bytes.chunks_exact(N)) {
        dst.copy_from_slice(src);
    }
    out
}

#[test]
fn matches_python_reference() {
    let vectors: Vec<Vector> =
        serde_json::from_str(include_str!("vectors.json")).expect("valid vectors.json");
    assert_eq!(vectors.len(), 6);
    for v in vectors {
        let tweak = Tweak {
            vault: hex32(&v.vault),
            key_index: v.key_index,
        };
        let message = hex32(&v.message);
        assert_eq!(digits(&message).to_vec(), v.digits, "{}", v.label);

        let mut secret = [[0u8; N]; LEN];
        secret_key::<HostSha256>(&hex32(&v.seed), &tweak, &mut secret);
        assert_eq!(secret, elements(&hex(&v.secret_key)), "{}", v.label);

        let mut public = [[0u8; N]; LEN];
        public_key::<HostSha256>(&tweak, &secret, &mut public);
        assert_eq!(public, elements(&hex(&v.public_key)), "{}", v.label);

        let hash = public_key_hash::<HostSha256>(&tweak, &public);
        assert_eq!(hash, hex32(&v.public_key_hash), "{}", v.label);

        let mut signature = [[0u8; N]; LEN];
        sign::<HostSha256>(&tweak, &secret, &message, &mut signature);
        let bytes = signature.as_flattened();
        assert_eq!(bytes.len(), SIGNATURE_LEN);
        assert_eq!(bytes, hex(&v.signature).as_slice(), "{}", v.label);

        assert!(
            verify::<HostSha256>(&tweak, &hash, &message, bytes),
            "{}",
            v.label
        );
    }
}
