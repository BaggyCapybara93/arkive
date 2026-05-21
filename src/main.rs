mod file_manager;
mod arg_parser;
mod crypto;
mod batch_handler;

use file_manager::FileManager;
use arg_parser::Command;
use batch_handler::BatchHandler;
use std::env;

fn handle_help() -> Result<(), String> {
    println!("Available commands:");
    println!("  move <src> <dest> [--recursive]");
    println!("  copy <src> <dest>");
    println!("  compress <src> <dest>");
    println!("  batch <batch_file>");
    Ok(())
}

fn handle_move(src: &str, dest: &str, recursive: bool) -> Result<(), String> {
    let fm = FileManager::new(src.to_string(), dest.to_string());
    if recursive {
        fm.copy_path(true).map_err(|e| format!("Failed to recursively copy directory: {}", e))?;
        fm.delete_path(true).map_err(|e| format!("Failed to delete original directory: {}", e))?;
    } else {
        fm.move_path().map_err(|e| format!("Failed to move file or directory: {}", e))?;
    }
    Ok(())
}

fn handle_copy(src: &str, dest: &str, recursive: bool) -> Result<(), String> {
    let fm = FileManager::new(src.to_string(), dest.to_string());
    if recursive {
        fm.copy_path(true).map_err(|e| format!("Failed to recursively copy directory: {}", e))?;
    } else {
        fm.copy_path(false).map_err(|e| format!("Failed to copy file or directory: {}", e))?;
    }
    Ok(())
}

fn handle_compress(src: &str, dest: &str) -> Result<(), String> {
    let fm = FileManager::new(src.to_string(), dest.to_string());
    fm.compress_path().map_err(|e| format!("Failed to compress file or directory: {}", e))?;
    Ok(())
}

fn handle_batch(file: &str) -> Result<(), String> {
    let batch_handler = BatchHandler::from_file(file).map_err(|e| format!("Failed to read batch file: {}", e))?;
    batch_handler.run();
    Ok(())
}

fn handler(cmd: Command) -> Result<(), String> {
    match cmd {
        Command::Help => handle_help(),
        Command::Move { src, dest, recursive } => handle_move(&src, &dest, recursive),
        Command::Copy { src, dest, recursive } => handle_copy(&src, &dest, recursive),
        Command::Compress { src, dest } => handle_compress(&src, &dest),
        Command::Batch { file } => handle_batch(&file),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let parser = arg_parser::ParseArguments::new(args);

    let parsed = parser.parse().expect("Failed to parse arguments.");

    handler(parsed).expect("Failed to execute command");
}
