mod file_manager;
mod cli;
mod crypto;
mod batch_handler;
mod error;

use clap::Parser;
use crate::cli::cli_handler;

fn main() {
    let cli = crate::cli::CLI::parse();
    let parsed = cli.command;

    cli_handler(parsed).expect("Failed to execute command");
}
