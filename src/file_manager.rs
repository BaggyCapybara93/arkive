use crate::crypto::hash_file;
use std::fs;
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
}

impl FileManager {
    pub fn new(file_path: String, file_dest: String) -> Self {
        FileManager { file_path, file_dest }
    }

    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        // rename works for both files and directories
        fs::rename(src, dst)?;

        Ok(())
    }

    pub fn copy_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        if !src.exists() {
            return Err(FileManagerError::InvalidInput(format!(
                "Source path {:?} does not exist",
                src
            )));
        }

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
            let src_hash = hash_file(&self.file_path)?;
            let dst_hash = hash_file(&self.file_dest)?;

            if src_hash != dst_hash {
                return Err(FileManagerError::HashMismatch);
            }
        }

        Ok(())
    }

    pub fn delete_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let src = Path::new(&self.file_path);

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
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        // Ensure destination ends with .tar.gz
        if !self.file_dest.ends_with(".tar.gz") {
            return Err(FileManagerError::InvalidInput(
                "Destination must end with .tar.gz".into(),
            ));
        }

        if !src.exists() {
            return Err(FileManagerError::InvalidInput(format!(
                "Source path {:?} does not exist",
                src
            )));
        }
        
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
        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(format!(
                "Source is not a directory: {:?}",
                src
            )));
        }

        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }

        let source = src.canonicalize()?;
        let destination = dst.canonicalize().unwrap_or_else(|_| dst.to_path_buf());

        if destination.starts_with(&source) {
            return Err(FileManagerError::InvalidInput(format!(
                "Destination {:?} cannot be inside source {:?}",
                destination, source
            )));
        }

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
