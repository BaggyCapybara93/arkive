mod file_module;
mod file_validation;
mod cli;
mod batch_module;
mod error;
mod settings;
mod config_module;
mod metadata_module;

use clap::Parser;
use crate::cli::cli_handler;
use crate::error::AppError;
use crate::settings::Settings;
use crate::config_module::ConfigManager;

fn main() -> Result<(), AppError> {
    let cli = crate::cli::CLI::parse();
    
    let config_manager = ConfigManager::new()?;
    config_manager.create_default_config()?;

    let config = config_manager.load()?;

    let settings = Settings {
        enable_trash: if cli.no_trash { false } else { config.enable_trash },
        verbose: if cli.verbose { true } else { config.verbose },
        dry_run: if cli.dry_run { true } else { config.dry_run },
        recursive: config.recursive,
        enable_metadata: config.enable_metadata,
        compression_method: config.compression_method,
    };

    cli_handler(cli.command, &settings)?;
    Ok(())
}