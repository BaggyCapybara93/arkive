use serde::Deserialize;
use crate::file_module::{FileManager, FileManagerError};
use crate::file_module::cleanup::CleanupOptions;
use std::sync::Arc;
use crate::settings::Settings;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum WorkType {
    Move,
    Copy,
    Compress,
    Rename,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BatchCompressionMethod {
    Gzip,
    Zstd,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
    pub work_type: WorkType,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
    pub cleanup: Option<bool>, //Cleanup after operation, can cause decrease in performance if set to true 
    pub compression_method: Option<BatchCompressionMethod>,
    pub timestamp: Option<bool>, // Add timestamp prefix to destination filename
    #[serde(skip)]
    pub settings: Option<Arc<Settings>>,
}

impl Job {
    pub fn execute(&self) -> Result<(), FileManagerError> {
        let settings = self.settings.as_ref()
            .ok_or_else(|| FileManagerError::InvalidInput("Settings not provided".to_string()))?;
        let recursive = self.recursive.unwrap_or(settings.recursive);
        let dest = match &self.destination {
            Some(dest) => dest.clone(),
            None => self.source.clone(),
        };
        
        let fm = FileManager::new(self.source.clone(), dest, settings);

        match self.work_type {
            WorkType::Move => {
                if recursive {
                    let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                    fm.copy_path(true, add_timestamp)?;
                    // Only delete source if copy succeeded (destination exists)
                    if fm.file_dest.exists() {
                        fm.delete_path(self.source.clone(), true, false)?;
                    } else {
                        // Rollback on failure
                        fm.delete_path(&fm.file_dest, true, false)?;
                        return Err(FileManagerError::InvalidInput(
                            "Recursive move failed: destination not created".to_string()
                        ));
                    }
                } else {
                    fm.move_path()?;
                }
            }
            WorkType::Copy => {
                let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                fm.copy_path(recursive, add_timestamp)?;
            }
            WorkType::Compress => {
                let compression_method = self.compression_method
                    .as_ref()
                    .map(|m| m.clone().into())
                    .or_else(|| self.settings.as_ref().map(|s| s.compression_method));
                let method = compression_method.unwrap_or(settings.compression_method);
                let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                fm.compress_path(method, add_timestamp)?;
            }
            WorkType::Rename => fm.rename_path()?,
        }
        
        
        if self.cleanup.unwrap_or(false) {
            // TEMP: Create default cleanup options with all flags enabled
            let cleanup_options = CleanupOptions::default();
            fm.cleanup(cleanup_options)?;
        }

        Ok(())
    }
}