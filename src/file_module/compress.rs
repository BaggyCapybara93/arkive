use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::file_module::add_timestamp_to_path;
use crate::file_module::error::FileManagerError;
use crate::file_module::ignore::{IgnoreMatcher, IgnoreStats};
use crate::file_module::ops::create_temp_file_sibling;
use crate::file_validation::handlers::{
    ensure_not_nested, valid_directory, validate_compress_path,
};

use super::manager::FileManager;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionMethod {
    Gzip,
    Zstd,
    Lz4,
    Xz,
    Bzip2,
}

/// Container used for a compressed backup. Tar remains the default because it
/// preserves the existing POSIX-oriented archive behavior; ZIP is an explicit
/// portable interchange format with its own entry model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    #[default]
    Tar,
    Zip,
}

impl FromStr for ArchiveFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tar" => Ok(ArchiveFormat::Tar),
            "zip" => Ok(ArchiveFormat::Zip),
            _ => Err(format!("Invalid archive format: {s}. Use 'tar' or 'zip'.")),
        }
    }
}

impl FromStr for CompressionMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "gzip" | "gz" => Ok(CompressionMethod::Gzip),
            "zstd" | "zst" => Ok(CompressionMethod::Zstd),
            "lz4" => Ok(CompressionMethod::Lz4),
            "xz" => Ok(CompressionMethod::Xz),
            "bzip2" | "bz2" => Ok(CompressionMethod::Bzip2),
            _ => Err(format!(
                "Invalid compression method: {}. Use 'gzip', 'zstd', 'lz4', 'xz', or 'bzip2'.",
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

        CompressionMethod::Lz4 => Ok(Box::new(
            lz4_flex::frame::FrameEncoder::new(file).auto_finish(),
        )),

        CompressionMethod::Xz => Ok(Box::new(xz2::write::XzEncoder::new(file, 6))),

        CompressionMethod::Bzip2 => Ok(Box::new(bzip2::write::BzEncoder::new(
            file,
            bzip2::Compression::default(),
        ))),
    }
}

impl<'a> FileManager<'a> {
    /// Compress a file or directory into a gzip-, Zstandard-, LZ4-, XZ-, or bzip2-compressed tar archive.
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
        validate_compress_path(dst, method)?;

        if src.is_dir() {
            valid_directory(src)?;
        }

        // Add timestamp to destination if requested
        let final_dst = if add_timestamp {
            add_timestamp_to_path(dst)?
        } else {
            dst.to_path_buf()
        };

        ensure_not_nested(src, &final_dst)?;

        if self.settings.dry_run {
            if self.settings.verbose {
                println!(
                    "[DRY-RUN] Would compress {:?} to {:?} using {:?}",
                    src, final_dst, method
                );
            }
            return Ok((final_dst, ignore_stats));
        }

        let staging = create_temp_file_sibling(&final_dst)?;
        let result = (|| {
            let file = fs::File::create(&staging)?;
            let encoder = create_encoder(&method, file)?;
            let mut tar = tar::Builder::new(encoder);

            if src.is_dir() {
                let src_name = src.file_name().ok_or_else(|| {
                    FileManagerError::InvalidInput("Invalid directory name".into())
                })?;
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
            drop(tar);
            fs::rename(&staging, &final_dst)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }

        result.map(|()| (final_dst, ignore_stats))
    }

    /// Create a Deflate-compressed ZIP archive. ZIP is kept separate from the
    /// tar compression methods because it is a container format, not a codec.
    pub fn compress_zip_path_filtered(
        &self,
        add_timestamp: bool,
        matcher: Option<&IgnoreMatcher>,
    ) -> Result<(std::path::PathBuf, IgnoreStats), FileManagerError> {
        let _guard = self.acquire_lock();
        let mut ignore_stats = IgnoreStats::default();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        crate::file_validation::handlers::validate_zip_path(dst)?;
        let source_metadata = fs::symlink_metadata(src)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FileManagerError::InvalidInput(format!(
                "ZIP source must not be a symbolic link: {src:?}"
            )));
        }
        if source_metadata.is_dir() {
            valid_directory(src)?;
        } else if !source_metadata.is_file() {
            return Err(FileManagerError::InvalidInput(format!(
                "ZIP source must be a regular file or directory: {src:?}"
            )));
        }

        let final_dst = if add_timestamp {
            add_timestamp_to_path(dst)?
        } else {
            dst.to_path_buf()
        };
        ensure_not_nested(src, &final_dst)?;

        if self.settings.dry_run {
            if self.settings.verbose {
                println!("[DRY-RUN] Would create ZIP archive {final_dst:?} from {src:?}");
            }
            return Ok((final_dst, ignore_stats));
        }

        let staging = create_temp_file_sibling(&final_dst)?;
        let result = (|| {
            let file = fs::File::create(&staging)?;
            let mut archive = zip::ZipWriter::new(file);

            if source_metadata.is_dir() {
                let src_name = src.file_name().ok_or_else(|| {
                    FileManagerError::InvalidInput("Invalid directory name".into())
                })?;
                append_zip_directory_filtered(
                    &mut archive,
                    src,
                    std::path::Path::new(src_name),
                    matcher,
                    &mut ignore_stats,
                )?;
            } else {
                let name = src
                    .file_name()
                    .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
                append_zip_file(&mut archive, src, std::path::Path::new(name))?;
            }

            archive.finish().map_err(zip_error)?;
            fs::rename(&staging, &final_dst)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }

        result.map(|()| (final_dst, ignore_stats))
    }

    pub fn compress_zip_path(
        &self,
        add_timestamp: bool,
    ) -> Result<std::path::PathBuf, FileManagerError> {
        self.compress_zip_path_filtered(add_timestamp, None)
            .map(|(path, _)| path)
    }
}

fn zip_error(error: zip::result::ZipError) -> FileManagerError {
    FileManagerError::InvalidInput(format!("ZIP archive error: {error}"))
}

fn zip_options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

fn zip_entry_name(path: &Path) -> Result<String, FileManagerError> {
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => names.push(name.to_str().ok_or_else(|| {
                FileManagerError::InvalidInput(format!(
                    "ZIP archive paths must be valid UTF-8: {path:?}"
                ))
            })?),
            _ => {
                return Err(FileManagerError::InvalidInput(format!(
                    "ZIP archive path must be a relative descendant: {path:?}"
                )));
            }
        }
    }
    if names.is_empty() {
        return Err(FileManagerError::InvalidInput(
            "ZIP archive path must not be empty".into(),
        ));
    }
    if names.iter().any(|name| name.contains('\\')) {
        return Err(FileManagerError::InvalidInput(format!(
            "ZIP archive paths must not contain backslashes: {path:?}"
        )));
    }
    Ok(names.join("/"))
}

fn append_zip_file(
    archive: &mut zip::ZipWriter<fs::File>,
    source: &Path,
    archive_path: &Path,
) -> Result<(), FileManagerError> {
    let mut source_file = fs::File::open(source)?;
    archive
        .start_file(zip_entry_name(archive_path)?, zip_options())
        .map_err(zip_error)?;
    io::copy(&mut source_file, archive)?;
    Ok(())
}

fn append_zip_directory_filtered(
    archive: &mut zip::ZipWriter<fs::File>,
    source: &Path,
    archive_path: &Path,
    matcher: Option<&IgnoreMatcher>,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    archive
        .add_directory(format!("{}/", zip_entry_name(archive_path)?), zip_options())
        .map_err(zip_error)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(FileManagerError::InvalidInput(format!(
                "ZIP source must not contain symbolic links: {path:?}"
            )));
        }
        let metadata = entry.metadata()?;
        if matcher
            .is_some_and(|matcher| matcher.is_excluded(&path, file_type.is_dir(), metadata.len()))
        {
            stats.record(&path)?;
            continue;
        }

        let destination = archive_path.join(entry.file_name());
        if file_type.is_dir() {
            append_zip_directory_filtered(archive, &path, &destination, matcher, stats)?;
        } else if file_type.is_file() {
            append_zip_file(archive, &path, &destination)?;
        } else {
            return Err(FileManagerError::InvalidInput(format!(
                "ZIP source must contain only regular files and directories: {path:?}"
            )));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{ArchiveFormat, CompressionMethod, FileManager};
    use crate::settings::Settings;
    use crate::test::TestDir;
    use bzip2::read::BzDecoder;
    use flate2::read::GzDecoder;
    use lz4_flex::frame::FrameDecoder;
    use std::fs::File;
    use xz2::read::XzDecoder;

    #[test]
    fn bzip2_aliases_parse_to_bzip2() {
        assert_eq!(
            "bzip2".parse::<CompressionMethod>().unwrap(),
            CompressionMethod::Bzip2
        );
        assert_eq!(
            "bz2".parse::<CompressionMethod>().unwrap(),
            CompressionMethod::Bzip2
        );
    }

    #[test]
    fn archive_formats_parse() {
        assert_eq!("tar".parse::<ArchiveFormat>().unwrap(), ArchiveFormat::Tar);
        assert_eq!("zip".parse::<ArchiveFormat>().unwrap(), ArchiveFormat::Zip);
    }

    #[test]
    fn failed_compression_preserves_existing_archive() {
        let temp = TestDir::new("compress-rollback");
        let source = temp.path().join("missing");
        let destination = temp.path().join("backup.tar.gz");
        std::fs::write(&destination, b"keep me").unwrap();

        let settings = Settings::default();
        let result = FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Gzip, false);

        assert!(result.is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"keep me");
    }

    #[test]
    fn successful_compression_installs_a_valid_archive() {
        let temp = TestDir::new("compress-success");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.tar.gz");
        std::fs::write(&source, b"archive me").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Gzip, false)
            .unwrap();

        let file = File::open(destination).unwrap();
        let mut archive = tar::Archive::new(GzDecoder::new(file));
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap(), std::path::Path::new("source.txt"));
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"archive me");
        assert!(entries.next().is_none());
    }

    #[test]
    fn lz4_compression_installs_a_valid_archive() {
        let temp = TestDir::new("compress-lz4-success");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.tar.lz4");
        std::fs::write(&source, b"archive me quickly").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Lz4, false)
            .unwrap();

        let file = File::open(destination).unwrap();
        let mut archive = tar::Archive::new(FrameDecoder::new(file));
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap(), std::path::Path::new("source.txt"));
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"archive me quickly");
        assert!(entries.next().is_none());
    }

    #[test]
    fn xz_compression_installs_a_valid_archive() {
        let temp = TestDir::new("compress-xz-success");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.tar.xz");
        std::fs::write(&source, b"archive me compactly").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Xz, false)
            .unwrap();

        let file = File::open(destination).unwrap();
        let mut archive = tar::Archive::new(XzDecoder::new(file));
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap(), std::path::Path::new("source.txt"));
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"archive me compactly");
        assert!(entries.next().is_none());
    }

    #[test]
    fn bzip2_compression_installs_a_valid_archive() {
        let temp = TestDir::new("compress-bzip2-success");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.tar.bz2");
        std::fs::write(&source, b"archive me compatibly").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Bzip2, false)
            .unwrap();

        let file = File::open(destination).unwrap();
        let mut archive = tar::Archive::new(BzDecoder::new(file));
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap(), std::path::Path::new("source.txt"));
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"archive me compatibly");
        assert!(entries.next().is_none());
    }

    #[test]
    fn zip_compression_installs_a_valid_archive() {
        let temp = TestDir::new("compress-zip-success");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.zip");
        std::fs::write(&source, b"archive me portably").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .compress_zip_path(false)
            .unwrap();

        let file = File::open(destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut entry = archive.by_name("source.txt").unwrap();
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"archive me portably");
    }

    #[test]
    fn compression_rejects_archive_inside_source_directory() {
        let temp = TestDir::new("compress-nested");
        let source = temp.path().join("source");
        let destination = source.join("backup.tar.gz");
        std::fs::create_dir(&source).unwrap();

        let settings = Settings::default();
        let result = FileManager::new(&source, &destination, &settings)
            .compress_path(CompressionMethod::Gzip, false);

        assert!(result.is_err());
        assert!(!destination.exists());
    }
}
