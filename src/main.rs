mod file_manager;
mod file_validation;
mod cli;
mod batch_handler;
mod error;
mod settings;

use clap::Parser;
use crate::cli::cli_handler;
use crate::error::AppError;
use crate::settings::Settings;

fn main() -> Result<(), AppError> {
    let cli = crate::cli::CLI::parse();
    
    let settings = Settings {
        enable_trash: !cli.no_trash,
        verbose: cli.verbose,
        dry_run: cli.dry_run,
        ..Default::default()
    };

    cli_handler(cli.command, &settings)?;
    Ok(())
}