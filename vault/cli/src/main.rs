mod amount;
mod cli;
mod commands;
mod config;
mod ix;
mod keys;
mod pending;
mod prompt;
mod rpc;
mod tx;
mod vault;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
