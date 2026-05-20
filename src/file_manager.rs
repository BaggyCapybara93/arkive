use crate::crypto::hash_file;
use std::fs;
use std::path::Path;


pub struct FileManager {
    pub file_path: String,
    pub file_dest: String,
}

impl FileManager {
    pub fn new(file_path: String, file_dest: String) -> Self {
        FileManager { file_path, file_dest }
    }

    pub fn move_path(&self) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        // rename works for both files and directories
        fs::rename(src, dst)?;
        Ok(())
    }

    pub fn copy_path(&self, recursive: bool) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        if src.is_dir() {
            if !recursive {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Use --recursive to copy directories",
                ));
            }
            copy_dir_recursive(src, dst)?;
        } else {
            fs::copy(src, dst)?;
        }

        if let Ok(src_hash) = hash_file(&self.file_path) {
            if let Ok(dst_hash) = hash_file(&self.file_dest) {
                if src_hash != dst_hash {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Hash mismatch after copy, file may be corrupted",
                    ));
                }
            }
        }

        Ok(())
    }

    pub fn delete_path(&self, recursive: bool) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);

        if src.is_dir() {
            if !recursive {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Use --recursive to delete directories",
                ));
            }
            fs::remove_dir_all(src)?;
        } else {
            fs::remove_file(src)?;
        }

        Ok(())
    }
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            FileManager::copy_path(&FileManager::new(src_path.to_string_lossy().into_owned(), dst_path.to_string_lossy().into_owned()), true)?;
        }
    }

    Ok(())
}
