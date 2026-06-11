use std::fs;

use crate::file_validation::handlers::{
    valid_directory, validate_compress_path
};
use crate::file_manager::error::FileManagerError;

use super::manager::FileManager;

impl<'a> FileManager<'a> {
    /// Compress a file or directory into a tar.gz archive.
    pub fn compress_path(&self) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        // Ensure destination is valid for compression
        validate_compress_path(dst)?;

        if src.is_dir() {
            valid_directory(src)?;
        }
        
        let tar_gz = fs::File::create(dst)?;
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);

        if src.is_dir() {
            let src_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid directory name".into()))?;
            tar.append_dir_all(src_name, src)?;
        } else {
            let name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
            tar.append_path_with_name(src, name)?;
        }

        tar.finish()?;
        Ok(())
    }
}