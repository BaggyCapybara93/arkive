use clap::{Parser, Subcommand};

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
}