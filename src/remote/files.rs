use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::invalid;
use crate::error::AppError;
use crate::file_module::ops::{create_temp_dir_sibling, create_temp_file_sibling, move_no_replace};

const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 128;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum Entry {
    Directory {
        path: String,
    },
    File {
        path: String,
        size: u64,
        sha256: String,
    },
}

impl Entry {
    pub fn path(&self) -> &str {
        match self {
            Self::Directory { path } | Self::File { path, .. } => path,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: String,
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Manifest {
    pub fn encode(&self) -> Result<Vec<u8>, AppError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| invalid(format!("Could not encode file manifest: {error}")))?;
        if bytes.len() > super::protocol::MAX_MANIFEST_BYTES {
            return Err(invalid("File manifest is too large"));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.len() > super::protocol::MAX_MANIFEST_BYTES {
            return Err(invalid("File manifest is too large"));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("Invalid file manifest: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.format != "arkive-remote-files" || self.version != 1 {
            return Err(invalid("Unsupported remote file manifest format/version"));
        }
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRIES {
            return Err(invalid(
                "Remote file manifest must have 1 to 100,000 entries",
            ));
        }
        if self.entries[0].path() != "" {
            return Err(invalid("Remote file manifest needs a root entry"));
        }
        let mut paths = BTreeSet::new();
        let mut directories = BTreeSet::new();
        let root_is_file = matches!(self.entries[0], Entry::File { .. });
        for (index, entry) in self.entries.iter().enumerate() {
            let path = entry.path();
            validate_relative(path, true)?;
            if !paths.insert(path) {
                return Err(invalid(format!("Duplicate remote file path: {path:?}")));
            }
            if index > 0 {
                if root_is_file {
                    return Err(invalid("A single-file transfer cannot have child entries"));
                }
                let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
                if !directories.contains(parent) {
                    return Err(invalid(format!("Missing parent directory for {path:?}")));
                }
            }
            match entry {
                Entry::Directory { .. } => {
                    directories.insert(path);
                }
                Entry::File { sha256, .. } => {
                    if sha256.len() != 64
                        || !sha256
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    {
                        return Err(invalid("Invalid remote file SHA-256 hash"));
                    }
                }
            }
        }
        self.file_count_and_bytes()?;
        Ok(())
    }

    pub fn file_count_and_bytes(&self) -> Result<(usize, u64), AppError> {
        self.entries
            .iter()
            .try_fold((0, 0u64), |(count, bytes), entry| match entry {
                Entry::File { size, .. } => Ok((
                    count + 1,
                    bytes
                        .checked_add(*size)
                        .ok_or_else(|| invalid("Remote file size overflow"))?,
                )),
                Entry::Directory { .. } => Ok((count, bytes)),
            })
    }
}

pub(super) fn validate_relative(path: &str, allow_root: bool) -> Result<(), AppError> {
    if path.is_empty() {
        return if allow_root {
            Ok(())
        } else {
            Err(invalid("Remote path cannot be empty"))
        };
    }
    if path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
    {
        return Err(invalid(
            "Remote path is absolute, too long, or contains unsupported characters",
        ));
    }
    let components: Vec<_> = path.split('/').collect();
    if components.len() > MAX_DEPTH
        || components.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.starts_with(".arkive-stage-")
                || part.bytes().any(|byte| byte < 0x20)
        })
    {
        return Err(invalid(
            "Remote path contains a traversal, reserved, or invalid component",
        ));
    }
    Ok(())
}

pub(super) fn prepare_root(vault: &Path, configured: Option<&Path>) -> Result<PathBuf, AppError> {
    let path = configured
        .map(Path::to_path_buf)
        .unwrap_or_else(|| vault.join("remote-files"));
    if configured.is_none() {
        #[cfg(unix)]
        let created = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path)
        };
        #[cfg(not(unix))]
        let created = fs::create_dir(&path);
        match created {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Remote files root must be a real directory: {path:?}"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(invalid(
                "Remote files root must be owned by this user and not writable by others",
            ));
        }
    }
    let files_root = fs::canonicalize(path)?;
    let vault_root = fs::canonicalize(vault)?;
    if files_root == vault_root
        || vault_root.starts_with(&files_root)
        || files_root.starts_with(vault_root.join(".arkive-vault"))
    {
        return Err(invalid("Remote files root must not expose vault internals"));
    }
    Ok(files_root)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash_file(path: &Path) -> Result<(String, u64), AppError> {
    let mut file = File::open(path)?;
    hash_reader(&mut file)
}

fn hash_reader(reader: &mut impl Read) -> Result<(String, u64), AppError> {
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("Remote file size overflow"))?;
    }
    Ok((hex(&hash.finalize()), total))
}

fn scan_entry(path: &Path, relative: &str, entries: &mut Vec<Entry>) -> Result<(), AppError> {
    if entries.len() >= MAX_ENTRIES {
        return Err(invalid("Remote file tree has too many entries"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid(format!("Remote copy refuses symlinks: {path:?}")));
    }
    if metadata.is_dir() {
        entries.push(Entry::Directory {
            path: relative.into(),
        });
        let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| invalid("Remote copy needs UTF-8 file names"))?;
            let child_relative = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            validate_relative(&child_relative, false)?;
            scan_entry(&child.path(), &child_relative, entries)?;
        }
    } else if metadata.is_file() {
        let (sha256, size) = hash_file(path)?;
        entries.push(Entry::File {
            path: relative.into(),
            size,
            sha256,
        });
    } else {
        return Err(invalid(format!(
            "Remote copy refuses special files: {path:?}"
        )));
    }
    Ok(())
}

pub(super) struct LocalExport {
    source: PathBuf,
    pub manifest: Manifest,
    pub bytes: Vec<u8>,
}

impl LocalExport {
    pub fn scan(source: &Path) -> Result<Self, AppError> {
        let mut entries = Vec::new();
        scan_entry(source, "", &mut entries)?;
        let manifest = Manifest {
            format: "arkive-remote-files".into(),
            version: 1,
            entries,
        };
        let bytes = manifest.encode()?;
        Ok(Self {
            source: source.to_path_buf(),
            manifest,
            bytes,
        })
    }

    pub fn send(&self, stream: &mut impl Write) -> Result<u64, AppError> {
        let mut sent = 0u64;
        for entry in &self.manifest.entries {
            let Entry::File { path, size, sha256 } = entry else {
                continue;
            };
            let file_path = if path.is_empty() {
                self.source.clone()
            } else {
                self.source.join(path)
            };
            let metadata = fs::symlink_metadata(&file_path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid("Remote source changed type during upload"));
            }
            let mut input = File::open(&file_path)?;
            let mut hash = Sha256::new();
            let mut left = *size;
            let mut buffer = [0u8; 64 * 1024];
            while left > 0 {
                let chunk = buffer.len().min(left as usize);
                let count = input.read(&mut buffer[..chunk])?;
                if count == 0 {
                    return Err(invalid("Remote source ended early during upload"));
                }
                hash.update(&buffer[..count]);
                stream.write_all(&buffer[..count])?;
                left -= count as u64;
            }
            if input.read(&mut buffer[..1])? != 0 || hex(&hash.finalize()) != *sha256 {
                return Err(invalid("Remote source changed during upload"));
            }
            sent = sent
                .checked_add(*size)
                .ok_or_else(|| invalid("Upload byte total overflow"))?;
        }
        stream.flush()?;
        Ok(sent)
    }
}

fn receive_file(
    stream: &mut impl Read,
    output: &mut File,
    size: u64,
    expected: &str,
) -> Result<(), AppError> {
    let mut left = size;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    while left > 0 {
        let chunk = buffer.len().min(left as usize);
        let count = stream.read(&mut buffer[..chunk])?;
        if count == 0 {
            return Err(invalid("Remote download ended early"));
        }
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        left -= count as u64;
    }
    output.sync_all()?;
    if hex(&hash.finalize()) != expected {
        return Err(invalid("Remote download SHA-256 mismatch"));
    }
    Ok(())
}

pub(super) struct StagedDownload {
    staged: PathBuf,
    destination: PathBuf,
    root_is_file: bool,
    bytes: u64,
    published: bool,
}

impl StagedDownload {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn publish(mut self) -> Result<(), AppError> {
        move_no_replace(&self.staged, &self.destination)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for StagedDownload {
    fn drop(&mut self) {
        if !self.published {
            if self.root_is_file {
                let _ = fs::remove_file(&self.staged);
            } else {
                let _ = fs::remove_dir_all(&self.staged);
            }
        }
    }
}

pub(super) fn receive_to(
    stream: &mut impl Read,
    destination: &Path,
    manifest: &Manifest,
) -> Result<StagedDownload, AppError> {
    manifest.validate()?;
    validate_destination(destination)?;
    let root_is_file = matches!(manifest.entries[0], Entry::File { .. });
    let staged = if root_is_file {
        create_temp_file_sibling(destination)?
    } else {
        create_temp_dir_sibling(destination)?
    };
    let result = (|| {
        for entry in &manifest.entries {
            match entry {
                Entry::Directory { path } if !path.is_empty() => fs::create_dir(staged.join(path))?,
                Entry::Directory { .. } => {}
                Entry::File { path, size, sha256 } => {
                    let mut output = if path.is_empty() {
                        OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&staged)?
                    } else {
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(staged.join(path))?
                    };
                    receive_file(stream, &mut output, *size, sha256)?;
                }
            }
        }
        let bytes = manifest.file_count_and_bytes()?.1;
        Ok(StagedDownload {
            staged: staged.clone(),
            destination: destination.to_path_buf(),
            root_is_file,
            bytes,
            published: false,
        })
    })();
    if result.is_err() {
        if root_is_file {
            let _ = fs::remove_file(&staged);
        } else {
            let _ = fs::remove_dir_all(&staged);
        }
    }
    result
}

pub(super) fn validate_destination(destination: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(invalid("Download destination already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("Download destination needs an existing parent"))?;
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("Download parent must be a real directory"));
    }
    Ok(())
}
