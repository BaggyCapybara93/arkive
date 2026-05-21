mod file_manager;
mod arg_parser;
mod crypto;

use file_manager::FileManager;
use arg_parser::Command;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let parser = arg_parser::ParseArguments::new(args);

    let parsed = parser.parse().expect("Failed to parse arguments.");

    match parsed {
        Command::Help => {
            println!("Available commands:");
            println!("  move <src> <dest> [--recursive]");
            println!("  copy <src> <dest>");
            println!("  compress <src> <dest>");
        }

        Command::Move { src, dest, recursive } => {
            let fm = FileManager::new(src, dest);
            if recursive {
                fm.copy_path(true).expect("Failed to recursively copy directory");
                fm.delete_path(true).expect("Failed to delete original directory");
            }else{
                fm.move_path().expect("Failed to move file or directory");
            }
        }

        Command::Copy { src, dest } => {
            let fm = FileManager::new(src, dest);
            fm.copy_path(true).expect("Failed to copy file or directory");
        }

        Command::Compress { src, dest } => {
            let fm = FileManager::new(src, dest);
            fm.compress_path().expect("Failed to compress file or directory");
        }
    }
}
