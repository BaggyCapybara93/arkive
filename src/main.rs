mod batch_module;
mod cli;
mod config_module;
mod error;
mod file_module;
mod file_validation;
mod metadata_module;
mod settings;

use crate::cli::cli_handler;
use crate::config_module::ConfigManager;
use crate::error::AppError;
use crate::settings::Settings;
use clap::Parser;

fn main() -> Result<(), AppError> {
    let cli = crate::cli::CLI::parse();

    let config_manager = ConfigManager::new()?;
    config_manager.create_default_config()?;

    let config = config_manager.load()?;

    let settings = Settings {
        enable_trash: if cli.no_trash {
            false
        } else {
            config.enable_trash
        },
        verbose: if cli.verbose { true } else { config.verbose },
        dry_run: if cli.dry_run { true } else { config.dry_run },
        recursive: config.recursive,
        enable_metadata: config.enable_metadata,
        compression_method: config.compression_method,
        use_timestamp: config.use_timestamp,
    };

    cli_handler(cli.command, &settings)?;
    Ok(())
}
