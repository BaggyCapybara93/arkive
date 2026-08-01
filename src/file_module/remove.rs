use std::fs;
use std::path::{Path, PathBuf};

use crate::file_module::error::FileManagerError;
use crate::file_module::FileManager;

/// Options for file removal
#[derive(Debug)]
pub struct RemoveOptions {
    pub trash: bool,
    pub dry_run: bool,
    pub verbose: bool,
}

impl<'a> FileManager<'a> {
    /// Remove files based on name pattern or extension
    pub fn remove_files(&self, pattern: &str, extension: Option<&str>, options: RemoveOptions) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();
        
        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Remove files requires a directory".into(),
            ));
        }
        
        let files_to_remove = Self::find_files_to_remove(src, pattern, extension)?;
        
        if files_to_remove.is_empty() {
            if options.verbose {
                println!("No files matching the pattern '{}' found", pattern);
            }
            return Ok(());
        }
        
        if options.dry_run {
            if options.verbose {
                println!("Dry run mode - no files will be removed:");
            }
            for file in &files_to_remove {
                if options.verbose {
                    println!("  Would remove: {:?}", file);
                }
            }
            return Ok(());
        }
        
        // Remove files
        for file_path in &files_to_remove {
            if options.verbose {
                println!("Removing: {:?}", file_path);
            }
            
            if options.trash {
                // Move to trash
                let trash_path = self.get_trash_path(file_path);
                fs::rename(file_path, &trash_path)?;
            } else {
                // Permanently delete
                fs::remove_file(file_path)?;
            }
        }
        
        if options.verbose {
            println!("Removed {} file(s)", files_to_remove.len());
        }
        
        Ok(())
    }
    
    /// Find files matching the pattern or extension
    fn find_files_to_remove(dir: &Path, pattern: &str, extension: Option<&str>) -> Result<Vec<PathBuf>, FileManagerError> {
        let mut files = Vec::new();
        
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() {
                let file_name = path.file_name().ok_or_else(|| FileManagerError::InvalidInput(
                    "File has no name".into(),
                ))?;
                let file_name_str = file_name.to_string_lossy();
                
                let should_remove = if let Some(ext) = extension {
                    // Check if file has the specified extension
                    file_name_str.ends_with(ext)
                } else {
                    // Check if file name matches the pattern
                    Self::matches_pattern(&file_name_str, pattern)
                };
                
                if should_remove {
                    files.push(path);
                }
            } else if path.is_dir() {
                // Recursively search subdirectories
                let sub_files = Self::find_files_to_remove(&path, pattern, extension)?;
                files.extend(sub_files);
            }
        }
        
        Ok(files)
    }
    
    /// Check if a file name matches a glob pattern
    fn matches_pattern(file_name: &str, pattern: &str) -> bool {
        Self::match_glob(file_name, pattern)
    }
    
    /// Match a file name against a glob pattern
    fn match_glob(file_name: &str, pattern: &str) -> bool {
        // Convert glob pattern to regex
        let regex_pattern = Self::glob_to_regex(pattern);
        
        // Use regex crate for pattern matching
        match regex::Regex::new(&regex_pattern) {
            Ok(regex) => regex.is_match(file_name),
            Err(_) => false,
        }
    }
    
    /// Convert a glob pattern to a regex pattern
    fn glob_to_regex(pattern: &str) -> String {
        let mut regex = String::new();
        
        for c in pattern.chars() {
            match c {
                '*' => regex.push_str(".*"),
                '?' => regex.push_str("."),
                '[' => regex.push_str(r"\["),
                ']' => regex.push_str(r"\]"),
                '(' => regex.push_str(r"\("),
                ')' => regex.push_str(r"\)"),
                '{' => regex.push_str(r"\{"),
                '}' => regex.push_str(r"\}"),
                '$' => regex.push_str(r"\$"),
                '^' => regex.push_str(r"\^"),
                '+' => regex.push_str(r"\+"),
                '.' => regex.push_str(r"\."),
                '\\' => regex.push_str(r"\\\\"),
                _ => regex.push(c),
            }
        }
        
        regex
    }
    
    /// Get the trash path for a file
    fn get_trash_path(&self, file_path: &Path) -> PathBuf {
        let src = self.file_path.as_path();
        let relative_path = file_path.strip_prefix(src).unwrap_or(file_path);
        
        // Get the trash directory
        let trash_dir = if self.settings.enable_trash {
            src.join(".arkive_trash")
        } else {
            src.to_path_buf()
        };
        
        trash_dir.join(relative_path)
    }
}
