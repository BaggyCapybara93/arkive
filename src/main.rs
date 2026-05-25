mod file_manager;
mod cli;
mod crypto;
mod batch_handler;
mod error;

use file_manager::FileManager;
use clap::Parser;
use crate::cli::Command;
use crate::error::AppError;
use batch_handler::BatchHandler;

fn handle_move(src: &str, dest: &str, recursive: bool) ->  Result<(), AppError> {
    let fm = FileManager::new(src.into(), dest.into());
    if recursive {
        fm.copy_path(true)?;
        fm.delete_path(true)?;
    } else {
        fm.move_path()?;
    }
    Ok(())
}

fn handle_copy(src: &str, dest: &str, recursive: bool) -> Result<(), AppError> {
    let fm = FileManager::new(src.into(), dest.into());
    if recursive {
        fm.copy_path(true)?;
    } else {
        fm.copy_path(false)?;
    }
    Ok(())
}

fn handle_compress(src: &str, dest: &str) -> Result<(), AppError> {
    let fm = FileManager::new(src.into(), dest.into());
    fm.compress_path()?;
    Ok(())
}

fn handle_batch(file: &str) -> Result<(), AppError> {
    let batch_handler = BatchHandler::from_file(file)?;
    batch_handler.run()?;
    Ok(())
}

fn handler(cmd: Command) -> Result<(), AppError> {
    match cmd {
        Command::Move { src, dest, recursive } => handle_move(&src, &dest, recursive),
        Command::Copy { src, dest, recursive } => handle_copy(&src, &dest, recursive),
        Command::Compress { src, dest } => handle_compress(&src, &dest),
        Command::Batch { file } => handle_batch(&file),
    }
}

fn main() {
    let cli = crate::cli::CLI::parse();
    let parsed = cli.command;

    handler(parsed).expect("Failed to execute command");
}
