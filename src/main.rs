mod file_manager;
mod cli;
mod crypto;
mod batch_handler;
mod error;

use clap::Parser;
use crate::cli::cli_handler;
use crate::error::AppError;

fn main() -> Result<(), AppError> {
    let cli = crate::cli::CLI::parse();
    let parsed = cli.command;

    cli_handler(parsed)?;
    Ok(())
}