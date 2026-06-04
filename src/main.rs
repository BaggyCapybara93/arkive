mod file_manager;
mod file_validation;
mod cli;
mod batch_handler;
mod error;
mod settings;
mod config_manager;

use clap::Parser;
use crate::cli::cli_handler;
use crate::error::AppError;
use crate::settings::Settings;
use crate::config_manager::ConfigManager;

fn main() -> Result<(), AppError> {
    let cli = crate::cli::CLI::parse();
    
    let config_manager = ConfigManager::new();
    if let Err(e) = config_manager.create_default_config() {
        eprintln!("Warning: failed to create default config file: {}", e);
    }

    let mut config = config_manager.load();
    
    if let Some(ref last_dir) = cli.last_used_directory {
        config.last_used_directory = Some(last_dir.clone());
        if let Err(e) = config_manager.save(&config) {
            eprintln!("Warning: failed to save config: {}", e);
        }
    }

    let settings = Settings {
        enable_trash: if cli.no_trash { false } else { config.enable_trash },
        verbose: if cli.verbose { true } else { config.verbose },
        dry_run: if cli.dry_run { true } else { config.dry_run },
        recursive: config.recursive,
    };

    cli_handler(cli.command, &settings)?;
    Ok(())
}