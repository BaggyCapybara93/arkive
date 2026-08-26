use std::fs;
use std::io::Write;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::file_module::add_timestamp_to_path;
use crate::file_module::error::FileManagerError;
use crate::file_module::ignore::{IgnoreMatcher, IgnoreStats};
use crate::file_validation::handlers::{valid_directory, validate_compress_path};

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
            _ => Err(format!(
                "Invalid compression method: {}. Use 'gzip' or 'zstd'.",
                s
            )),
        }
    }
}

///Creates the encoder for the specific compression method using the methods sepcificied for said library
fn create_encoder(
    method: &CompressionMethod,
    file: fs::File,
) -> Result<Box<dyn Write>, FileManagerError> {
    match method {
        CompressionMethod::Gzip => Ok(Box::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ))),

        CompressionMethod::Zstd => {
            let encoder = zstd::Encoder::new(file, 3).map_err(FileManagerError::Io)?;
            Ok(Box::new(encoder.auto_finish()))
        }
    }
}

impl<'a> FileManager<'a> {
    /// Compress a file or directory into a tar.gz archive.
    pub fn compress_path(
        &self,
        method: CompressionMethod,
        add_timestamp: bool,
    ) -> Result<std::path::PathBuf, FileManagerError> {
        self.compress_path_filtered(method, add_timestamp, None)
            .map(|(path, _)| path)
    }

    pub fn compress_path_filtered(
        &self,
        method: CompressionMethod,
        add_timestamp: bool,
        matcher: Option<&IgnoreMatcher>,
    ) -> Result<(std::path::PathBuf, IgnoreStats), FileManagerError> {
        let _guard = self.acquire_lock();
        let mut ignore_stats = IgnoreStats::default();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        // Ensure destination is valid for compression
        validate_compress_path(dst)?;

        if src.is_dir() {
            valid_directory(src)?;
        }

        // Add timestamp to destination if requested
        let final_dst = if add_timestamp {
            add_timestamp_to_path(dst)?
        } else {
            dst.to_path_buf()
        };

        if self.settings.dry_run {
            if self.settings.verbose {
                println!(
                    "[DRY-RUN] Would compress {:?} to {:?} using {:?}",
                    src, final_dst, method
                );
            }
            return Ok((final_dst, ignore_stats));
        }

        let file = fs::File::create(&final_dst)?;
        let encoder = create_encoder(&method, file)?;
        let mut tar = tar::Builder::new(encoder);

        if src.is_dir() {
            let src_name = src
                .file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid directory name".into()))?;
            tar.append_dir(src_name, src)?;
            append_directory_filtered(
                &mut tar,
                src,
                std::path::Path::new(src_name),
                matcher,
                &mut ignore_stats,
            )?;
        } else {
            let name = src
                .file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
            tar.append_path_with_name(src, name)?;
        }

        tar.finish()?;
        Ok((final_dst, ignore_stats))
    }
}

fn append_directory_filtered<W: Write>(
    archive: &mut tar::Builder<W>,
    source: &std::path::Path,
    archive_path: &std::path::Path,
    matcher: Option<&IgnoreMatcher>,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let metadata = entry.metadata()?;
        if matcher
            .is_some_and(|matcher| matcher.is_excluded(&path, file_type.is_dir(), metadata.len()))
        {
            stats.record(&path)?;
            continue;
        }

        let destination = archive_path.join(entry.file_name());
        if file_type.is_dir() {
            archive.append_dir(&destination, &path)?;
            append_directory_filtered(archive, &path, &destination, matcher, stats)?;
        } else {
            archive.append_path_with_name(&path, &destination)?;
        }
    }
    Ok(())
}
