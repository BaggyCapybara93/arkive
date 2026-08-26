use crate::batch_module::BatchHandler;
use crate::error::AppError;
use crate::file_module::FileManager;
use crate::file_module::cleanup;
use crate::file_module::compress::CompressionMethod;
use crate::file_module::deploy::{self, BackupKind};
use crate::file_module::ignore::{IgnoreMatcher, IgnoreOptions};
use crate::file_module::remove;
use crate::settings::Settings;
use clap::{Args, Parser, Subcommand, value_parser};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "arkive", about = "A simple file management tool")]
pub struct CLI {
    #[arg(long, help = "Disable trash, permanently delete files")]
    pub no_trash: bool,

    #[arg(short, long, help = "Enable verbose output")]
    pub verbose: bool,

    #[arg(long, help = "Preview operations without executing")]
    pub dry_run: bool,

    #[arg(long, help = "Path to use as last used directory for config")]
    pub last_used_directory: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args, Clone, Debug, Default)]
pub struct IgnoreArgs {
    /// Do not load the global Arkive ignore file
    #[arg(long)]
    no_global_ignore: bool,

    /// Do not load .arkiveignore from the source directory
    #[arg(long)]
    no_local_ignore: bool,

    /// Load an additional ignore file (may be repeated)
    #[arg(long = "ignore-file")]
    ignore_files: Vec<PathBuf>,

    /// Exclude a gitignore-style pattern (may be repeated)
    #[arg(long)]
    exclude: Vec<String>,

    /// Re-include a pattern with highest precedence (may be repeated)
    #[arg(long)]
    include: Vec<String>,

    /// Exclude files larger than a size such as 500MB or 2GB
    #[arg(long)]
    exclude_larger_than: Option<String>,
}

impl From<&IgnoreArgs> for IgnoreOptions {
    fn from(args: &IgnoreArgs) -> Self {
        Self {
            no_global: args.no_global_ignore,
            no_local: args.no_local_ignore,
            files: args.ignore_files.clone(),
            excludes: args.exclude.clone(),
            includes: args.include.clone(),
            exclude_larger_than: args.exclude_larger_than.clone(),
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Move a file or directory
    Move {
        /// Source path
        src: PathBuf,

        /// Destination path
        dest: PathBuf,
        #[arg(long, help = "Move directories recursively")]
        recursive: bool,

        /// Save portable metadata so this backup can be deployed later
        #[arg(long)]
        metadata: bool,

        #[command(flatten)]
        ignore: IgnoreArgs,
    },

    /// Copy a file or directory
    Copy {
        /// Source path
        src: PathBuf,

        /// Destination path
        dest: PathBuf,

        #[arg(long, help = "Copy directories recursively")]
        recursive: bool,

        /// Save portable metadata so this backup can be deployed later
        #[arg(long)]
        metadata: bool,

        #[command(flatten)]
        ignore: IgnoreArgs,
    },

    /// Compress a file or directory into a tar.gz archive
    Compress {
        /// Source path
        src: PathBuf,

        /// Destination path
        dest: PathBuf,

        #[arg(long, help = "Compression method (gzip or zstd)", value_parser = value_parser!(CompressionMethod))]
        method: Option<CompressionMethod>,

        /// Save portable metadata so this archive can be deployed later
        #[arg(long)]
        metadata: bool,

        #[command(flatten)]
        ignore: IgnoreArgs,
    },

    /// Restore a backup to its recorded original path
    Deploy {
        /// Copied path, moved path, or compressed archive to restore
        backup: PathBuf,

        /// Restore somewhere other than the recorded original path
        #[arg(long)]
        destination: Option<PathBuf>,

        /// Replace an existing destination
        #[arg(long)]
        force: bool,
    },

    /// Inspect ignore decisions
    Ignore {
        #[command(subcommand)]
        command: IgnoreCommand,
    },

    /// Rename a file or directory, or rename matching files/folders in a directory
    Rename {
        /// Source path or directory to rename
        name: PathBuf,

        /// New name for a single item, or a rename template such as "prefix-{name}{ext}" for bulk rename
        new_name: Option<PathBuf>,

        /// Pattern to match file or directory names (supports glob patterns like *.log, *.txt, or specific names)
        #[arg(short, long)]
        pattern: Option<String>,

        /// Extension to match (e.g., .log, .txt)
        #[arg(short, long, conflicts_with = "pattern")]
        extension: Option<String>,

        /// Rename matching files and folders recursively
        #[arg(long, help = "Rename matching files and folders recursively")]
        recursive: bool,
    },

    /// Execute a batch of commands from a JSON file
    Batch {
        /// Path to batch file
        file: PathBuf,
    },

    /// Empty the arkive trash directory
    EmptyTrash,

    /// List contents of the arkive trash directory
    ListTrash,

    /// Scan a directory for files with the same hash
    Deduplicate {
        /// Path of folder
        path: PathBuf,

        #[arg(long, help = "Keep deleted files in arkive trash")]
        trash: bool,
    },

    /// Clean up the workspace with multiple options
    Cleanup {
        /// Path to scan (default: current directory)
        path: Option<PathBuf>,

        #[arg(long, help = "Empty the arkive trash directory")]
        empty_trash: bool,

        #[arg(long, help = "Scan for and remove duplicate files")]
        deduplicate: bool,

        #[arg(long, help = "Scan for unused files (not accessed in 30+ days)")]
        scan_unused: bool,

        #[arg(long, help = "Scan for and remove empty directories")]
        scan_empty_dirs: bool,
    },

    /// Remove files based on name pattern or extension
    Remove {
        /// Path to scan (default: current directory)
        path: PathBuf,

        /// Pattern to match file names (supports glob patterns like *.log, *.txt, or specific names)
        #[arg(short, long)]
        pattern: String,

        /// Extension to match (e.g., .log, .txt)
        #[arg(short, long, conflicts_with = "pattern")]
        extension: Option<String>,

        /// Keep files in arkive trash instead of permanently deleting
        #[arg(long, help = "Move files to arkive trash")]
        trash: bool,
    },
}

#[derive(Subcommand)]
pub enum IgnoreCommand {
    /// Explain whether a path is included or excluded
    Check {
        /// File or directory to check
        path: PathBuf,

        /// Source root containing the local .arkiveignore
        #[arg(long, default_value = ".")]
        root: PathBuf,

        #[command(flatten)]
        ignore: IgnoreArgs,
    },
}

fn handle_move(
    src: &Path,
    dest: &Path,
    recursive: bool,
    metadata: bool,
    ignore_args: &IgnoreArgs,
    settings: &Settings,
) -> Result<(), AppError> {
    let original = std::fs::canonicalize(src)?;
    let matcher = IgnoreMatcher::build(src, &IgnoreOptions::from(ignore_args))?;
    let fm = FileManager::new(src, dest, settings);
    let mut partial_move = false;
    let backup = if recursive {
        let (backup, ignored) =
            fm.copy_path_filtered(true, settings.use_timestamp, Some(&matcher))?;
        if !settings.dry_run {
            if ignored.entries == 0 {
                fm.delete_path(src, true, false)?;
            } else {
                partial_move = true;
                matcher.remove_included_sources(src)?;
            }
        }
        print_ignore_summary(&ignored, settings);
        backup
    } else {
        fm.move_path()?
    };
    if metadata && !settings.dry_run {
        deploy::save_manifest_with_ignores(
            &original,
            &backup,
            BackupKind::Move,
            None,
            matcher.descriptions(),
            partial_move,
        )?;
    }
    Ok(())
}

fn handle_copy(
    src: &Path,
    dest: &Path,
    recursive: bool,
    metadata: bool,
    ignore_args: &IgnoreArgs,
    settings: &Settings,
) -> Result<(), AppError> {
    let original = std::fs::canonicalize(src)?;
    let matcher = IgnoreMatcher::build(src, &IgnoreOptions::from(ignore_args))?;
    let fm = FileManager::new(src, dest, settings);
    let (backup, ignored) =
        fm.copy_path_filtered(recursive, settings.use_timestamp, Some(&matcher))?;
    print_ignore_summary(&ignored, settings);
    if metadata && !settings.dry_run {
        deploy::save_manifest_with_ignores(
            &original,
            &backup,
            BackupKind::Copy,
            None,
            matcher.descriptions(),
            false,
        )?;
    }
    Ok(())
}

fn handle_compress(
    src: &Path,
    dest: &Path,
    method: Option<CompressionMethod>,
    metadata: bool,
    ignore_args: &IgnoreArgs,
    settings: &Settings,
) -> Result<(), AppError> {
    let original = std::fs::canonicalize(src)?;
    let matcher = IgnoreMatcher::build(src, &IgnoreOptions::from(ignore_args))?;
    let fm = FileManager::new(src, dest, settings);
    let compression_method = method.unwrap_or(settings.compression_method);
    let (backup, ignored) =
        fm.compress_path_filtered(compression_method, settings.use_timestamp, Some(&matcher))?;
    print_ignore_summary(&ignored, settings);
    if metadata && !settings.dry_run {
        deploy::save_manifest_with_ignores(
            &original,
            &backup,
            BackupKind::Compress,
            Some(compression_method),
            matcher.descriptions(),
            false,
        )?;
    }
    Ok(())
}

fn print_ignore_summary(stats: &crate::file_module::ignore::IgnoreStats, settings: &Settings) {
    if settings.verbose && stats.entries > 0 {
        println!(
            "Excluded {} item(s) totaling {} bytes",
            stats.entries, stats.bytes
        );
    }
}

fn handle_ignore(command: IgnoreCommand) -> Result<(), AppError> {
    match command {
        IgnoreCommand::Check { path, root, ignore } => {
            let matcher = IgnoreMatcher::build(&root, &IgnoreOptions::from(&ignore))?;
            let canonical_path = std::fs::canonicalize(&path)?;
            let metadata = std::fs::metadata(&canonical_path)?;
            let (excluded, reason) =
                matcher.decision(&canonical_path, metadata.is_dir(), metadata.len());
            println!(
                "{}: {:?}",
                if excluded { "EXCLUDED" } else { "INCLUDED" },
                path
            );
            println!("Matched rule: {}", reason.unwrap_or("none"));
            Ok(())
        }
    }
}

fn handle_rename(
    src: &Path,
    new_name: Option<&Path>,
    pattern: Option<&str>,
    extension: Option<&str>,
    recursive: bool,
    settings: &Settings,
) -> Result<(), AppError> {
    if let Some(dest) = new_name {
        if pattern.is_some() || extension.is_some() {
            let fm = FileManager::new(src, "", settings);
            fm.rename_matching_items(pattern, extension, recursive, &dest.to_string_lossy())?;
        } else {
            let fm = FileManager::new(src, dest, settings);
            fm.rename_path()?;
        }
    } else if pattern.is_some() || extension.is_some() {
        return Err(AppError::InvalidInput(
            "Bulk rename requires a rename template via the new_name argument".into(),
        ));
    } else {
        return Err(AppError::InvalidInput(
            "Provide a new name, or use --pattern/--extension with a rename template".into(),
        ));
    }

    Ok(())
}

fn handle_batch(file: &Path, settings: &Settings) -> Result<(), AppError> {
    let file_str = file.to_str().ok_or_else(|| {
        AppError::InvalidInput(format!(
            "Batch file path contains invalid UTF‑8: {:?}",
            file
        ))
    })?;

    let batch_handler = BatchHandler::from_file(file_str, settings)?;
    batch_handler.run()?;
    Ok(())
}

fn handle_deduplicate(path: &Path, to_trash: bool, settings: &Settings) -> Result<(), AppError> {
    let fm = FileManager::new(path, "", settings);
    fm.folder_deduplication(to_trash)?;
    Ok(())
}

fn handle_cleanup(
    path: Option<&Path>,
    options: cleanup::CleanupOptions,
    settings: &Settings,
) -> Result<(), AppError> {
    let path = path.map_or(Path::new("."), |p| p);
    let fm = FileManager::new(path, "", settings);
    fm.cleanup(options)?;
    Ok(())
}

fn handle_remove(
    path: &Path,
    pattern: &str,
    extension: Option<&str>,
    trash: bool,
    settings: &Settings,
) -> Result<(), AppError> {
    let fm = FileManager::new(path, "", settings);
    let options = remove::RemoveOptions {
        trash,
        dry_run: settings.dry_run,
        verbose: settings.verbose,
    };
    fm.remove_files(pattern, extension, options)?;
    Ok(())
}

fn handle_empty_trash(settings: &Settings) -> Result<(), AppError> {
    FileManager::empty_trash(settings)?;
    Ok(())
}

fn handle_list_trash(settings: &Settings) -> Result<(), AppError> {
    FileManager::list_trash(settings)?;
    Ok(())
}

pub fn cli_handler(cmd: Command, settings: &Settings) -> Result<(), AppError> {
    match cmd {
        Command::Move {
            src,
            dest,
            recursive,
            metadata,
            ignore,
        } => handle_move(&src, &dest, recursive, metadata, &ignore, settings),
        Command::Copy {
            src,
            dest,
            recursive,
            metadata,
            ignore,
        } => handle_copy(&src, &dest, recursive, metadata, &ignore, settings),
        Command::Compress {
            src,
            dest,
            method,
            metadata,
            ignore,
        } => handle_compress(&src, &dest, method, metadata, &ignore, settings),
        Command::Deploy {
            backup,
            destination,
            force,
        } => {
            deploy::deploy(&backup, destination.as_deref(), force, settings)?;
            Ok(())
        }
        Command::Ignore { command } => handle_ignore(command),
        Command::Rename {
            name,
            new_name,
            pattern,
            extension,
            recursive,
        } => handle_rename(
            &name,
            new_name.as_deref(),
            pattern.as_deref(),
            extension.as_deref(),
            recursive,
            settings,
        ),
        Command::Batch { file } => handle_batch(&file, settings),
        Command::EmptyTrash => handle_empty_trash(settings),
        Command::ListTrash => handle_list_trash(settings),
        Command::Deduplicate { path, trash } => handle_deduplicate(&path, trash, settings),
        Command::Cleanup {
            path,
            empty_trash,
            deduplicate,
            scan_unused,
            scan_empty_dirs,
        } => {
            let options = cleanup::CleanupOptions {
                empty_trash,
                deduplicate,
                scan_unused,
                scan_empty_dirs,
            };
            handle_cleanup(path.as_deref(), options, settings)
        }
        Command::Remove {
            path,
            pattern,
            extension,
            trash,
        } => handle_remove(&path, &pattern, extension.as_deref(), trash, settings),
    }
}
