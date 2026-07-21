use std::fs;
use std::io::Write;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::file_validation::handlers::{
    valid_directory, validate_compress_path
};
use crate::file_module::error::FileManagerError;

use super::manager::FileManager;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionMethod {
    Gzip,
    Zstd,
}

impl FromStr for CompressionMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "gzip" | "gz" => Ok(CompressionMethod::Gzip),
            "zstd" | "zst" => Ok(CompressionMethod::Zstd),
            _ => Err(format!("Invalid compression method: {}. Use 'gzip' or 'zstd'.", s)),
        }
    }
}

///Creates the encoder for the specific compression method using the methods sepcificied for said library
fn create_encoder(method: &CompressionMethod, file: fs::File) -> Result<Box<dyn Write>, FileManagerError> 
{
    match method {
        CompressionMethod::Gzip => {
            Ok(Box::new(flate2::write::GzEncoder::new(
                file,
                flate2::Compression::default(),
            )))
        }

        CompressionMethod::Zstd => {
            let encoder = zstd::Encoder::new(file, 3)
                .map_err(|e| FileManagerError::Io(e.into()))?;
            Ok(Box::new(encoder.auto_finish()))
        }
    }
}

impl<'a> FileManager<'a> {
    /// Compress a file or directory into a tar.gz archive.
    pub fn compress_path(&self, method: CompressionMethod) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        // Ensure destination is valid for compression
        validate_compress_path(dst)?;

        if src.is_dir() {
            valid_directory(src)?;
        }
        
        let file = fs::File::create(dst)?;
        let encoder = create_encoder(&method, file)?;
        let mut tar = tar::Builder::new(encoder);

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