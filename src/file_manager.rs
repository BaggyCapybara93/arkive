use crate::file_validation::handlers::{
    ensure_not_nested,
    valid_directory,
    validate_hash
};
use std::fs;
use parking_lot::Mutex;
use std::sync::Arc;
use std::path::Path;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;
use thiserror::Error;
use std::fs::File;

//Error Handling
#[derive(Error, Debug)]
pub enum FileManagerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch after copy — file may be corrupted")]
    HashMismatch,

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

pub struct FileManager {
    pub file_path: String,
    pub file_dest: String,
    lock: Mutex<()>,
}

impl FileManager {
    pub fn new(file_path: String, file_dest: String) -> Self {
        FileManager { 
            file_path, 
            file_dest,
            lock: Mutex::new(()),
        }
    }

    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let _guard = self.lock.lock();
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        valid_directory(src)?;

        // rename works for both files and directories
        fs::rename(src, dst)?;

        Ok(())
    }

    pub fn copy_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let _guard = self.lock.lock();
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        valid_directory(src)?;

        if src.is_dir() {
            if !recursive {
                return Err(FileManagerError::InvalidInput(format!(
                    "Use --recursive to copy directories: {:?}",
                    src
                )));
            }
            Self::copy_dir_recursive(src, dst)?;
        } else {
            fs::copy(src, dst)?;
        }

        // Verify file integrity
        if src.is_file() {
            validate_hash(src, dst)?;
        }

        Ok(())
    }

    pub fn delete_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let _guard = self.lock.lock();
        let src = Path::new(&self.file_path);

        valid_directory(src)?;

        if src.is_dir() {
            if !recursive {
                return Err(FileManagerError::InvalidInput(format!(
                    "Use --recursive to delete directories: {:?}",
                    src
                )));
            }
            fs::remove_dir_all(src)?;
        } else {
            fs::remove_file(src)?;
        }

        Ok(())
    }

    pub fn compress_path(&self) -> Result<(), FileManagerError> {
        let _guard = self.lock.lock();
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        // Ensure destination ends with .tar.gz
        if !self.file_dest.ends_with(".tar.gz") {
            return Err(FileManagerError::InvalidInput(
                "Destination must end with .tar.gz".into(),
            ));
        }

        valid_directory(src)?;
        
        let tar_gz = File::create(dst)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = Builder::new(enc);

        if src.is_dir() {
            let src_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid directory name".into()))?;
            tar.append_dir_all(src_name, src)?;
        } else {
            tar.append_path(src)?;
        }

        tar.finish()?;
        Ok(())
    }

    pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), FileManagerError>{
        valid_directory(src)?;

        ensure_not_nested(src, dst)?;

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }

        Ok(())
    }

}
