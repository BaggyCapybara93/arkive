use clap::{Parser, Subcommand};
use crate::file_manager::FileManager;
use crate::batch_handler::BatchHandler;
use crate::error::AppError;

#[derive(Parser)]
#[command(
        name = "arkive", 
        version = "0.1", 
        about = "A simple file management tool"
)]

pub struct CLI {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Move a file or directory
    Move {
        /// Source path
        src: String,

        /// Destination path
        dest: String,
        #[arg(long, help = "Move directories recursively")]
        recursive: bool,
    },

    /// Copy a file or directory
    Copy {
        /// Source path
        src: String,

        /// Destination path
        dest: String,

        #[arg(long, help = "Copy directories recursively")]
        recursive: bool,
    },

    /// Compress a file or directory into a tar.gz archive
    Compress {
        /// Source path
        src: String,
        
        /// Destination path
        dest: String,
    },

    /// Execute a batch of commands from a JSON file
    Batch {
        /// Path to batch file
        file: String,
    },

    ///Scan a directory for files with the same hash
    Deduplicate {
        /// Path of folder
        path: String,

        #[arg(long, help = "Keep deleted files in arkive trash")]
        trash: bool,
    },
}

fn handle_move(src: &str, dest: &str, recursive: bool) ->  Result<(), AppError> {
    let fm = FileManager::new(src, dest);
    if recursive {
        fm.copy_path(true)?;
        fm.delete_path(src, true, false)?;
    } else {
        fm.move_path()?;
    }
    Ok(())
}

fn handle_copy(src: &str, dest: &str, recursive: bool) -> Result<(), AppError> {
    let fm = FileManager::new(src, dest);
    if recursive {
        fm.copy_path(true)?;
    } else {
        fm.copy_path(false)?;
    }
    Ok(())
}

fn handle_compress(src: &str, dest: &str) -> Result<(), AppError> {
    let fm = FileManager::new(src, dest);
    fm.compress_path()?;
    Ok(())
}

fn handle_batch(file: &str) -> Result<(), AppError> {
    let batch_handler = BatchHandler::from_file(file)?;
    batch_handler.run()?;
    Ok(())
}

fn handle_deduplicate(path: &str, to_trash: bool) -> Result<(), AppError>{
   let fm = FileManager::new(path, "");
   fm.folder_deduplication(to_trash)?;
    Ok(())
}

pub fn cli_handler(cmd: Command) -> Result<(), AppError> {
    match cmd {
        Command::Move { src, dest, recursive } => handle_move(&src, &dest, recursive),
        Command::Copy { src, dest, recursive } => handle_copy(&src, &dest, recursive),
        Command::Compress { src, dest } => handle_compress(&src, &dest),
        Command::Batch { file } => handle_batch(&file),
        Command::Deduplicate { path, trash } =>  handle_deduplicate(&path, trash),
    }
}