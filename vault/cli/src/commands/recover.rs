use crate::{
    cli::Context,
    commands::status,
    config::{self, write_seed_file, SeedFile},
    prompt,
};
use anyhow::Result;

pub fn run(ctx: &Context) -> Result<()> {
    let mnemonic = config::parse_mnemonic(&prompt::read_line("Enter the 24 words: ")?)?;
    let passphrase_protected = prompt::confirm("Was a BIP-39 passphrase used? [y/N] ")?;
    write_seed_file(
        &ctx.paths.seed(),
        &SeedFile::new(mnemonic.to_string(), passphrase_protected),
    )?;
    println!("Seed restored to {}", ctx.paths.seed().display());
    status::run(ctx)
}
