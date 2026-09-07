use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_module::FileManager;
use crate::file_module::error::FileManagerError;
use crate::settings::Settings;

impl<'a> FileManager<'a> {
    pub(crate) fn trash_path() -> Result<PathBuf, FileManagerError> {
        Ok(if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join("arkive_trash")
        } else {
            std::env::current_dir()?.join("arkive_trash")
        })
    }

    pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
        let trash = Self::trash_path()?;

        if !trash.exists() {
            std::fs::create_dir_all(&trash)?;
        }

        Ok(trash)
    }

    /// Build a destination in the shared trash directory without replacing an
    /// item that is already there. This is used by every operation that moves
    /// an item to trash, so `list-trash` and `empty-trash` see the same files.
    pub(crate) fn unique_trash_path(src: &Path) -> Result<PathBuf, FileManagerError> {
        let file_name = src
            .file_name()
            .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
        let trash = Self::trash_dir()?;
        Self::unique_path(&trash.join(file_name))
    }

    fn unique_path(path: &Path) -> Result<PathBuf, FileManagerError> {
        if !path.exists() {
            return Ok(path.to_path_buf());
        }

        let parent = path.parent().ok_or_else(|| {
            FileManagerError::InvalidInput(format!("Trash destination has no parent: {:?}", path))
        })?;
        let file_name = path.file_name().ok_or_else(|| {
            FileManagerError::InvalidInput(format!(
                "Trash destination has no file name: {:?}",
                path
            ))
        })?;
        let stem = path.file_stem().unwrap_or(file_name);
        let extension = path.extension();

        for suffix in 1u64.. {
            let mut candidate_name = OsString::from(stem);
            candidate_name.push(format!("-{suffix}"));
            if let Some(extension) = extension {
                candidate_name.push(".");
                candidate_name.push(extension);
            }

            let candidate = parent.join(candidate_name);
            if !candidate.exists() {
                return Ok(candidate);
            }
        }

        unreachable!("u64 suffix space is exhausted")
    }

    pub fn empty_trash(settings: &Settings) -> Result<(), FileManagerError> {
        let lock_path = Self::trash_path()?;
        let _guard = Self::acquire_paths([lock_path.as_path()]);
        let trash = if settings.dry_run {
            let trash = Self::trash_path()?;
            if !trash.exists() {
                if settings.verbose {
                    println!("[DRY-RUN] Trash is already empty");
                }
                return Ok(());
            }
            trash
        } else {
            Self::trash_dir()?
        };

        if !trash.is_dir() {
            return Err(FileManagerError::InvalidDirectory(format!(
                "Trash path is not a directory: {:?}",
                trash
            )));
        }

        // List or remove contents. Dry-run must not create a trash directory
        // or change any of its entries.
        for entry in fs::read_dir(&trash)? {
            let entry = entry?;
            let path = entry.path();

            // Reject symlinks
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                return Err(FileManagerError::PermissionDenied(format!(
                    "Symlink found in trash: {:?}",
                    path
                )));
            }

            if settings.dry_run {
                if settings.verbose {
                    println!("[DRY-RUN] Would permanently delete {:?}", path);
                }
            } else if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }

        if settings.verbose {
            if settings.dry_run {
                println!("[DRY-RUN] Trash would be emptied");
            } else {
                println!("Trash emptied");
            }
        }

        Ok(())
    }

    pub fn list_trash(_settings: &Settings) -> Result<(), FileManagerError> {
        let lock_path = Self::trash_path()?;
        let _guard = Self::acquire_paths([lock_path.as_path()]);
        let trash = Self::trash_dir()?;

        for entry in std::fs::read_dir(trash)? {
            let entry = entry?;
            println!("{:?}", entry.path());
        }
        Ok(())
    }
}
