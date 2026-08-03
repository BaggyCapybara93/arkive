use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::file_module::error::FileManagerError;
use crate::file_module::FileManager;

impl<'a> FileManager<'a> {
    /// Clean up the workspace with multiple options
    /// 
    /// Flags:
    /// - empty_trash: Empty the arkive trash directory
    /// - deduplicate: Scan for and remove duplicate files
    /// - scan_unused: Scan for empty or unused files
    /// - scan_empty_dirs: Scan for and remove empty directories
    pub fn cleanup(&self, options: CleanupOptions) -> Result<(), FileManagerError> {
        if options.empty_trash {
            FileManager::empty_trash(&self.settings)?;
        }
        
        if options.deduplicate {
            self.folder_deduplication(true)?;
        }
        
        if options.scan_unused {
            self.scan_unused_files()?;
        }
        
        if options.scan_empty_dirs {
            self.scan_and_remove_empty_dirs()?;
        }
        
        Ok(())
    }

    /// Scan for unused files (files that haven't been accessed recently)
    fn scan_unused_files(&self) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();
        
        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Scan for unused files requires a directory".into(),
            ));
        }
        
        let now = SystemTime::now();
        let thirty_days = 30 * 24 * 60 * 60;
        
        Self::scan_directory_recursive(&src.to_path_buf(), now, thirty_days, self)?;
        
        if self.settings.verbose {
            println!("Scan complete");
        }
        
        Ok(())
    }

    fn scan_file(path: &PathBuf, now: SystemTime, thirty_days: u64, settings: &FileManager<'_>) -> Result<(), FileManagerError> {
        if let Ok(metadata) = fs::metadata(path) {
            if let Ok(accessed) = metadata.accessed() {
                let elapsed = now.duration_since(accessed).map_err(|e| FileManagerError::InvalidInput(e.to_string()))?;
                let elapsed_secs = elapsed.as_secs();
                    
                if elapsed_secs > thirty_days {
                    if settings.settings.verbose {
                        println!("Found unused file (not accessed in 30+ days): {:?}", path);
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }
        
    fn scan_directory_recursive(dir: &PathBuf, now: SystemTime, thirty_days: u64, settings: &FileManager<'_>) -> Result<(), FileManagerError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
                
            Self::scan_file(&path, now, thirty_days, settings)?;
                
            if path.is_dir() {
                Self::scan_directory_recursive(&path, now, thirty_days, settings)?;
            }
        }
        Ok(())
    }
    
    /// Scan for and remove empty directories
    fn scan_and_remove_empty_dirs(&self) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();
        
        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Scan for empty directories requires a directory".into(),
            ));
        }
        
        let mut empty_count = 0;
        let mut empty_dirs: Vec<PathBuf> = Vec::new();
        
        Self::find_empty_dirs_recursive(&src.to_path_buf(), &mut empty_dirs)?;
        
        // Remove empty directories (in reverse order to handle nested)
        for dir_path in empty_dirs.into_iter().rev() {
            if fs::remove_dir(&dir_path).is_ok() {
                if self.settings.verbose {
                    println!("Removing empty directory: {:?}", dir_path);
                }
                empty_count += 1;
            }
        }
        
        if self.settings.verbose {
            println!("Scan complete: {} empty directories removed", empty_count);
        }
        
        Ok(())
    }

    fn find_empty_dirs_recursive(path: &PathBuf, empty_dirs: &mut Vec<PathBuf>) -> Result<(), FileManagerError> {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let entry_path = entry.path();
                        
                    if entry_path.is_dir() {
                        Self::find_empty_dirs_recursive(&entry_path, empty_dirs)?;
                            
                        // Check if directory is empty after children processed
                        match fs::read_dir(&entry_path) {
                            Ok(entries) => {
                                if entries.count() == 0 {
                                     empty_dirs.push(entry_path.clone());
                                }
                            }
                            Err(_) => {}
                        }
                    }
                }
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }
}

/// Options for the cleanup operation
#[derive(Debug, Clone, Default)]
pub struct CleanupOptions {
    /// Empty the arkive trash directory
    pub empty_trash: bool,
    
    /// Scan for and remove duplicate files
    pub deduplicate: bool,
    
    /// Scan for unused files (not accessed in 30+ days)
    pub scan_unused: bool,
    
    /// Scan for and remove empty directories
    pub scan_empty_dirs: bool,
}
