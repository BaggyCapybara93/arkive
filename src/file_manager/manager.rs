use crate::file_validation::handlers::{
    ensure_not_nested,
    valid_directory,
    validate_hash
};
use crate::file_validation::hash::hash_file;

use super::error::FileManagerError;

use std::fs;
use std::collections::HashMap;
use parking_lot::Mutex;
use std::path::Path;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;
use std::path::PathBuf;
use std::fs::File;

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

    //Move this to its own utillity module later
    pub fn delete_path(&self, path: String, recursive: bool, to_trash: bool) -> Result<(), FileManagerError> {
        let _guard = self.lock.lock();
        let src = Path::new(&path);

        if src.is_dir(){
            valid_directory(src)?;
        }

        if to_trash {
            let trash = Self::trash_dir()?;

            // Extract filename
            let file_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;

            // Build destination inside trash
            let dst = trash.join(file_name);

            // Move instead of delete
            fs::rename(src, dst)?;
            return Ok(());
        }

        // Normal delete
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
            let name = src.file_name()?;
            tar.append_path_with_name(src, name)?;
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

    pub fn folder_deduplication(&self, to_trash: bool) -> Result<(), FileManagerError> {
        let src = Path::new(&self.file_path);

        valid_directory(src)?;

        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Deduplication requires a directory".into(),
            ));
        }

        let mut seen: HashMap<String, String> = HashMap::new();

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                // Convert path to &str safely
                let path_str = path.to_str().ok_or_else(|| {
                    FileManagerError::InvalidInput("Invalid UTF-8 file path".into())
                })?;

                // Compute hash
                let hash = hash_file(path_str)?;

                if let Some(original) = seen.get(&hash) {
                    // Duplicate → delete it
                    self.delete_path(path.to_string_lossy().to_string(), true, to_trash)?;
                    println!("Removed duplicate: {:?} (original: {:?})", path, original);
                } else {
                    seen.insert(hash, path_str.to_string());
                }
            }
        }

        Ok(())
    }

    //Change this to be more customizable 
    pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
        let cwd = std::env::current_dir()?;
        let trash = cwd.join("arkive_trash");

        if !trash.exists() {
            std::fs::create_dir_all(&trash)?;
        }

        Ok(trash)
    }
}
