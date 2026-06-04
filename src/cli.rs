use clap::{Parser, Subcommand};
use crate::batch_handler::BatchHandler;
use crate::error::AppError;
use crate::file_manager::cleanup;
use crate::file_manager::trash;
use crate::file_manager::FileManager;
use crate::settings::Settings;

#[derive(Parser)]
#[command(
        name = "arkive", 
        version = "0.1", 
        about = "A simple file management tool"
)]

pub struct CLI {
    #[arg(long, help = "Disable trash, permanently delete files")]
    pub no_trash: bool,

    #[arg(short, long, help = "Enable verbose output")]
    pub verbose: bool,

    #[arg(long, help = "Preview operations without executing")]
    pub dry_run: bool,

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

    /// Empty the arkive trash directory
    EmptyTrash,

    /// List contents of the arkive trash directory
    ListTrash,

    /// Scan a directory for files with the same hash
    Deduplicate {
        /// Path of folder
        path: String,

        #[arg(long, help = "Keep deleted files in arkive trash")]
        trash: bool,
    },

    /// Clean up the workspace with multiple options
    Cleanup {
        /// Path to scan (default: current directory)
        path: Option<String>,

        #[arg(long, help = "Empty the arkive trash directory")]
        empty_trash: bool,

        #[arg(long, help = "Scan for and remove duplicate files")]
        deduplicate: bool,

        #[arg(long, help = "Scan for unused files (not accessed in 30+ days)")]
        scan_unused: bool,

        #[arg(long, help = "Scan for and remove empty directories")]
        scan_empty_dirs: bool,
    },
}

fn handle_move(src: &str, dest: &str, recursive: bool, settings: &Settings) ->  Result<(), AppError> {
    let fm = FileManager::new(src, dest, settings);
    if recursive {
        fm.copy_path(true)?;
        fm.delete_path(src, true, false)?;
    } else {
        fm.move_path()?;
    }
    Ok(())
}

fn handle_copy(src: &str, dest: &str, recursive: bool, settings: &Settings) -> Result<(), AppError> {
    let fm = FileManager::new(src, dest, settings);
    if recursive {
        fm.copy_path(true)?;
    } else {
        fm.copy_path(false)?;
    }
    Ok(())
}

fn handle_compress(src: &str, dest: &str, settings: &Settings) -> Result<(), AppError> {
    let fm = FileManager::new(src, dest, settings);
    fm.compress_path()?;
    Ok(())
}

fn handle_batch(file: &str, settings: &Settings) -> Result<(), AppError> {
    let batch_handler = BatchHandler::from_file(file, settings)?;
    batch_handler.run()?;
    Ok(())
}

fn handle_deduplicate(path: &str, to_trash: bool, settings: &Settings) -> Result<(), AppError> {
    let fm = FileManager::new(path, "", settings);
    fm.folder_deduplication(to_trash)?;
    Ok(())
}

fn handle_cleanup(path: Option<String>, options: cleanup::CleanupOptions, settings: &Settings) -> Result<(), AppError> {
    let path_str = path.unwrap_or_else(|| std::env::current_dir().unwrap().to_string_lossy().to_string());
    let fm = FileManager::new(&path_str, "", settings);
    fm.cleanup(options)?;
    Ok(())
}

fn handle_empty_trash(settings: &Settings) -> Result<(), AppError> {
    trash::empty_trash(settings)?;
    Ok(())
}

fn handle_list_trash(settings: &Settings) -> Result<(), AppError> {
    trash::list_trash(settings)?;
    Ok(())
}

pub fn cli_handler(cmd: Command, settings: &Settings) -> Result<(), AppError> {
    match cmd {
        Command::Move { src, dest, recursive } => handle_move(&src, &dest, recursive, settings),
        Command::Copy { src, dest, recursive } => handle_copy(&src, &dest, recursive, settings),
        Command::Compress { src, dest } => handle_compress(&src, &dest, settings),
        Command::Batch { file } => handle_batch(&file, settings),
        Command::EmptyTrash => handle_empty_trash(settings),
        Command::ListTrash => handle_list_trash(settings),
        Command::Deduplicate { path, trash } => handle_deduplicate(&path, trash, settings),
        Command::Cleanup { path, empty_trash, deduplicate, scan_unused, scan_empty_dirs } => {
            let options = cleanup::CleanupOptions {
                empty_trash,
                deduplicate,
                scan_unused,
                scan_empty_dirs,
            };
            handle_cleanup(path, options, settings)
        }
    }
}