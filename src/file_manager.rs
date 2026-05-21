use crate::crypto::hash_file;
use std::fs;
use std::io;
use std::path::Path;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;
use std::fs::File;

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
        fs::rename(src, dst).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("Failed to move {:?} to {:?}: {}", src, dst, e)
            )
        })?;

        Ok(())
    }

    pub fn copy_path(&self, recursive: bool) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        if !src.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Source path {:?} does not exist", src),
            ));
        }

        if src.is_dir() {
            if !recursive {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Use --recursive to copy directories: {:?}", src),
                ));
            }
            Self::copy_dir_recursive(src, dst)?;
        } else {
            fs::copy(src, dst).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("Failed to copy {:?} to {:?}: {}", src, dst, e)
                )
            })?;
        }

        if src.is_file() {
            let src_hash = hash_file(&self.file_path)?;
            let dst_hash = hash_file(&self.file_dest)?;

            if src_hash != dst_hash {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Hash mismatch after copy — file may be corrupted",
                ));
            }
        }

        Ok(())
    }

    pub fn delete_path(&self, recursive: bool) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);

        if src.is_dir() {
            if !recursive {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Use --recursive to delete directories: {:?}", src),
                ));
            }
            fs::remove_dir_all(src)?;
        } else {
            fs::remove_file(src)?;
        }

        Ok(())
    }

    pub fn compress_path(&self) -> std::io::Result<()> {
        let src = Path::new(&self.file_path);
        let dst = Path::new(&self.file_dest);

        // Ensure destination ends with .tar.gz
        if !self.file_dest.ends_with(".tar.gz") {
            return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
                "Destination must end with .tar.gz",
            ));
        }

        if !src.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Source path {:?} does not exist", src),
            ));
        }
        
        let tar_gz = File::create(dst)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = Builder::new(enc);

        if src.is_dir() {
            let src_name = src.file_name().unwrap();
            tar.append_dir_all(src_name, src)?;
        } else {
            tar.append_path(src)?;
        }

        tar.finish()?;
        Ok(())
    }

    pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }

        if !src.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("Source is not a directory: {:?}", src),
            ));
        }

        let source = src.canonicalize()?;
        let destination = dst.canonicalize().unwrap_or(dst.to_path_buf());

        if destination.starts_with(&source) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Destination {:?} cannot be inside source {:?}", destination, source),
            ));
        }

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path).map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("Failed to copy {:?} to {:?}: {}", src_path, dst_path, e)
                    )
                })?;
            }
        }

        Ok(())
    }

}
