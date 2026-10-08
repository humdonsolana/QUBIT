use crate::config::{write_seed_file, Paths, SeedFile};
use anyhow::{bail, Result};
use bip39::{Language, Mnemonic, WordCount};

pub fn run(paths: &Paths, passphrase: bool) -> Result<()> {
    if passphrase {
        let first = rpassword::prompt_password("BIP-39 passphrase: ")?;
        let second = rpassword::prompt_password("Repeat passphrase: ")?;
        if first != second {
            bail!("passphrases do not match");
        }
        if first.is_empty() {
            bail!("empty passphrase; omit --passphrase instead");
        }
    }
    let mnemonic = Mnemonic::generate_in(Language::English, WordCount::Words24)?;
    let words = mnemonic.to_string();
    write_seed_file(&paths.seed(), &SeedFile::new(words.clone(), passphrase))?;
    println!("Seed saved to {}", paths.seed().display());
    println!(
        "Write these 24 words down. They are the only backup of every qubit derived from them.\n"
    );
    println!("{words}\n");
    if passphrase {
        println!("The passphrase is not stored anywhere. Losing it loses the funds.");
    }
    println!("Next: qubit create");
    Ok(())
}
