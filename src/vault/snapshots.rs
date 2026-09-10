//! Immutable, content-addressed snapshots. Only a published manifest is a snapshot.
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CONTROL, VaultLock, combine, exists, invalid, io_at, require_directory};
use crate::error::AppError;
use crate::file_module::ops::create_temp_dir_sibling;

const FORMAT: &str = "arkive-snapshot";
const VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum Entry {
    Directory {
        path: String,
    },
    File {
        path: String,
        sha256: String,
        size: u64,
    },
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Directory { path } | Self::File { path, .. } => path,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    created_at: DateTime<Utc>,
    label: Option<String>,
    source_name: String,
    entries: Vec<Entry>,
}

#[derive(Debug, Serialize)]
struct Snapshot {
    id: String,
    #[serde(flatten)]
    manifest: Manifest,
}

fn initialized(path: &Path) -> Result<PathBuf, AppError> {
    let root = super::root(path)?;
    if !super::validate(&root)? {
        return Err(invalid(format!(
            "No vault at {root:?}; run vault init first"
        )));
    }
    Ok(root)
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

fn component(name: &str) -> Result<(), AppError> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', ':', '\0']) {
        return Err(invalid(format!(
            "Unsupported snapshot path component: {name:?}"
        )));
    }
    Ok(())
}

fn label_check(label: Option<&str>) -> Result<(), AppError> {
    if label.is_some_and(|label| label.len() > 256) {
        return Err(invalid("Snapshot labels must be at most 256 UTF-8 bytes"));
    }
    Ok(())
}

fn regular_file(path: &Path) -> Result<fs::Metadata, AppError> {
    let metadata = io_at(fs::symlink_metadata(path), "inspect regular file", path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Expected a regular file, not a link or special file: {path:?}"
        )));
    }
    Ok(metadata)
}

/// Stream files to keep memory bounded, optionally capturing the bytes in staging.
fn hash_file(path: &Path, mut output: Option<&mut File>) -> Result<(String, u64), AppError> {
    regular_file(path)?;
    let mut file = io_at(File::open(path), "open snapshot file", path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(invalid(format!(
            "Source changed type while opening {path:?}"
        )));
    }
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = io_at(file.read(&mut buffer), "read snapshot file", path)?;
        if count == 0 {
            break;
        }
        if let Some(output) = &mut output {
            io_at(
                output.write_all(&buffer[..count]),
                "stage snapshot bytes from",
                path,
            )?;
        }
        hasher.update(&buffer[..count]);
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("Snapshot file is too large"))?;
    }
    let after = file.metadata()?;
    if before.len() != size || after.len() != size || before.modified()? != after.modified()? {
        return Err(invalid(format!(
            "File changed while reading {path:?}; close the game and retry"
        )));
    }
    Ok((hex(&hasher.finalize()), size))
}

fn scan(source: &Path) -> Result<Vec<Entry>, AppError> {
    let mut entries = Vec::new();
    scan_entry(source, "", 0, &mut entries)?;
    entries.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(entries)
}

fn scan_entry(
    source: &Path,
    relative: &str,
    depth: usize,
    entries: &mut Vec<Entry>,
) -> Result<(), AppError> {
    if depth > MAX_DEPTH || entries.len() >= MAX_ENTRIES {
        return Err(invalid(
            "Snapshot exceeds 128 directory levels or 100,000 entries",
        ));
    }
    let path = source.join(relative);
    // Joining an empty path adds a trailing separator, which is invalid for file roots.
    let path = if relative.is_empty() { source } else { &path };
    let metadata = io_at(fs::symlink_metadata(path), "inspect save path", path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Snapshot sources cannot contain symlinks: {path:?}"
        )));
    }
    if metadata.is_dir() {
        entries.push(Entry::Directory {
            path: relative.into(),
        });
        for child in fs::read_dir(path)? {
            let child = child?;
            let name = child.file_name().into_string().map_err(|_| {
                invalid(format!("Snapshot paths must be UTF-8: {:?}", child.path()))
            })?;
            component(&name)?;
            let relative = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            scan_entry(source, &relative, depth + 1, entries)?;
        }
    } else if metadata.is_file() {
        let (sha256, size) = hash_file(path, None)?;
        entries.push(Entry::File {
            path: relative.into(),
            sha256,
            size,
        });
    } else {
        return Err(invalid(format!(
            "Snapshot sources cannot contain special files: {path:?}"
        )));
    }
    Ok(())
}

fn object_path(root: &Path, hash: &str) -> PathBuf {
    root.join(CONTROL).join("objects").join(hash)
}

fn verify_object(root: &Path, hash: &str, size: u64) -> Result<(), AppError> {
    if !valid_id(hash) {
        return Err(invalid("Invalid SHA-256 object identifier"));
    }
    let directory = object_path(root, hash);
    require_directory(&directory)?;
    let data = directory.join("data");
    if hash_file(&data, None)? != (hash.into(), size) {
        return Err(invalid(format!(
            "Corrupt snapshot object {hash}: hash or size mismatch"
        )));
    }
    Ok(())
}

/// Directory publication avoids relying on hard-link support on SMB/NFS shares.
/// A complete object/snapshot directory is nonempty, so rename cannot replace it.
/// Cooperating writers must hold VaultLock throughout this operation.
fn publish(staged: &Path, destination: &Path) -> Result<(), AppError> {
    if exists(destination)? {
        return Err(invalid(format!(
            "Refusing to overwrite vault entry {destination:?}"
        )));
    }
    io_at(
        fs::rename(staged, destination),
        "publish vault entry",
        destination,
    )
}

fn store_object(
    root: &Path,
    scratch: &Path,
    source: &Path,
    hash: &str,
    size: u64,
) -> Result<(), AppError> {
    let destination = object_path(root, hash);
    if exists(&destination)? {
        // Never silently reuse or repair a corrupt object shared by older snapshots.
        return verify_object(root, hash, size);
    }
    let staged = scratch.join(hash);
    io_at(fs::create_dir(&staged), "stage object directory", &staged)?;
    let data = staged.join("data");
    let mut output = io_at(
        OpenOptions::new().write(true).create_new(true).open(&data),
        "stage object file",
        &data,
    )?;
    let captured = hash_file(source, Some(&mut output))?;
    io_at(output.sync_all(), "flush object file", &data)?;
    drop(output);
    if captured != (hash.into(), size) {
        return Err(invalid(format!(
            "Source changed while capturing {source:?}; close the game and retry"
        )));
    }
    if hash_file(&data, None)? != captured {
        return Err(invalid(format!(
            "Staged object failed SHA-256 verification: {data:?}"
        )));
    }
    publish(&staged, &destination)?;
    verify_object(root, hash, size)
}

fn totals(manifest: &Manifest) -> Result<(usize, u64), AppError> {
    let mut files = 0;
    let mut bytes = 0u64;
    for entry in &manifest.entries {
        if let Entry::File { size, .. } = entry {
            files += 1;
            bytes = bytes
                .checked_add(*size)
                .ok_or_else(|| invalid("Snapshot total size overflow"))?;
        }
    }
    Ok((files, bytes))
}

fn validate_manifest(manifest: &Manifest) -> Result<(), AppError> {
    if manifest.format != FORMAT || manifest.version != VERSION {
        return Err(invalid("Unsupported snapshot format/version"));
    }
    label_check(manifest.label.as_deref())?;
    component(&manifest.source_name)?;
    if manifest.entries.is_empty() || manifest.entries.len() > MAX_ENTRIES {
        return Err(invalid(
            "Snapshot must contain between 1 and 100,000 entries",
        ));
    }
    let mut directories = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for entry in &manifest.entries {
        let path = entry.path();
        if !paths.insert(path) {
            return Err(invalid(format!("Duplicate snapshot path: {path:?}")));
        }
        if !path.is_empty() {
            if path.split('/').count() > MAX_DEPTH {
                return Err(invalid("Snapshot path exceeds 128 components"));
            }
            for name in path.split('/') {
                component(name)?;
            }
            let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
            if !directories.contains(parent) {
                return Err(invalid(format!(
                    "Missing parent directory for snapshot path {path:?}"
                )));
            }
        } else if !directories.is_empty() || paths.len() != 1 {
            return Err(invalid("Snapshot root must be the first entry"));
        }
        match entry {
            Entry::Directory { .. } => {
                directories.insert(path);
            }
            Entry::File { sha256, .. } if !valid_id(sha256) => {
                return Err(invalid("Invalid snapshot object hash"));
            }
            Entry::File { .. } => {}
        }
    }
    if !paths.contains("") {
        return Err(invalid("Snapshot root entry is missing"));
    }
    totals(manifest)?;
    Ok(())
}

fn load(root: &Path, id: &str) -> Result<Snapshot, AppError> {
    if !valid_id(id) {
        return Err(invalid(
            "Snapshot ID must be 64 lowercase hexadecimal characters",
        ));
    }
    let directory = root.join(CONTROL).join("snapshots").join(id);
    require_directory(&directory)?;
    let path = directory.join("manifest.json");
    regular_file(&path)?;
    let mut bytes = Vec::new();
    io_at(
        File::open(&path)?
            .take(MAX_MANIFEST_BYTES as u64 + 1)
            .read_to_end(&mut bytes),
        "read snapshot manifest",
        &path,
    )?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("Snapshot manifest exceeds 16 MiB"));
    }
    if hex(&Sha256::digest(&bytes)) != id {
        return Err(invalid(format!("Snapshot manifest SHA-256 mismatch: {id}")));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid snapshot manifest {id}: {error}")))?;
    validate_manifest(&manifest)?;
    Ok(Snapshot {
        id: id.into(),
        manifest,
    })
}

fn encode_manifest(manifest: &Manifest) -> Result<Vec<u8>, AppError> {
    validate_manifest(manifest)?;
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| invalid(format!("Could not encode snapshot: {error}")))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("Snapshot manifest exceeds 16 MiB"));
    }
    Ok(bytes)
}

fn save_manifest(root: &Path, scratch: &Path, bytes: &[u8]) -> Result<String, AppError> {
    let id = hex(&Sha256::digest(bytes));
    let destination = root.join(CONTROL).join("snapshots").join(&id);
    if exists(&destination)? {
        load(root, &id)?;
        return Ok(id);
    }
    let staged = scratch.join("snapshot");
    fs::create_dir(&staged)?;
    super::write_new(&staged.join("manifest.json"), bytes)?;
    publish(&staged, &destination)?;
    load(root, &id)?;
    Ok(id)
}

fn capture(
    root: &Path,
    source: &Path,
    label: Option<String>,
    scratch: Option<&Path>,
) -> Result<Snapshot, AppError> {
    label_check(label.as_deref())?;
    let source: PathBuf = source.components().collect();
    let metadata = io_at(
        fs::symlink_metadata(&source),
        "inspect snapshot source",
        &source,
    )?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid(
            "Snapshot source must be a regular file or directory, not a symlink",
        ));
    }
    let source = fs::canonicalize(&source)?;
    if source.starts_with(root) || root.starts_with(&source) {
        return Err(invalid("Snapshot source and vault must not overlap"));
    }
    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Snapshot source must have a UTF-8 file name"))?
        .to_owned();
    component(&source_name)?;
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        created_at: Utc::now(),
        label,
        source_name,
        entries: scan(&source)?,
    };
    let bytes = encode_manifest(&manifest)?;
    let mut objects = BTreeMap::new();
    for entry in &manifest.entries {
        if let Entry::File { path, sha256, size } = entry {
            if let Some(previous) = objects.insert(sha256, *size) {
                if previous != *size {
                    return Err(invalid("Conflicting sizes for the same object hash"));
                }
                continue;
            }
            let file = if path.is_empty() {
                source.clone()
            } else {
                source.join(path)
            };
            match scratch {
                Some(scratch) => store_object(root, scratch, &file, sha256, *size)?,
                None if exists(&object_path(root, sha256))? => verify_object(root, sha256, *size)?,
                None => {}
            }
        }
    }
    ensure_unchanged(&source, &manifest.entries)?;
    let id = match scratch {
        Some(scratch) => save_manifest(root, scratch, &bytes)?,
        None => String::new(),
    };
    Ok(Snapshot { id, manifest })
}

fn ensure_unchanged(source: &Path, expected: &[Entry]) -> Result<(), AppError> {
    if scan(source)? != expected {
        return Err(invalid(
            "Save tree changed during snapshot; close the game and retry. No snapshot was published.",
        ));
    }
    Ok(())
}

pub fn create(
    path: &Path,
    source: &Path,
    label: Option<String>,
    dry_run: bool,
) -> Result<(), AppError> {
    let root = initialized(path)?;
    if dry_run {
        super::ensure_unlocked(&root)?;
        let snapshot = capture(&root, source, label, None)?;
        let (files, bytes) = totals(&snapshot.manifest)?;
        println!(
            "[DRY-RUN] Would snapshot {source:?}: {files} files, {bytes} bytes; no objects or manifest written"
        );
        return Ok(());
    }
    let lock = VaultLock::acquire(&root)?;
    let mut snapshot = None;
    let result = (|| {
        initialized(&root)?;
        super::with_scratch(&root, |scratch| {
            snapshot = Some(capture(&root, source, label, Some(scratch))?);
            Ok(())
        })
    })();
    combine(result, lock.release())?;
    let snapshot = snapshot.expect("successful capture has a snapshot");
    let (files, bytes) = totals(&snapshot.manifest)?;
    println!(
        "Snapshot {} created: {files} files, {bytes} bytes",
        snapshot.id
    );
    Ok(())
}

fn history(root: &Path) -> Result<Vec<Snapshot>, AppError> {
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(root.join(CONTROL).join("snapshots"))? {
        let entry = entry?;
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Invalid snapshot directory name"))?;
        snapshots.push(load(root, &id)?);
    }
    snapshots.sort_by(|left, right| {
        right
            .manifest
            .created_at
            .cmp(&left.manifest.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(snapshots)
}

pub fn list(path: &Path, json: bool) -> Result<(), AppError> {
    let root = initialized(path)?;
    let snapshots = history(&root)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshots).map_err(|error| invalid(error.to_string()))?
        );
    } else if snapshots.is_empty() {
        println!("No snapshots in {root:?}");
    } else {
        for snapshot in snapshots {
            let (files, bytes) = totals(&snapshot.manifest)?;
            println!(
                "{}  {}  {} files  {} bytes  source={:?}  label={:?}",
                snapshot.id,
                snapshot.manifest.created_at.to_rfc3339(),
                files,
                bytes,
                snapshot.manifest.source_name,
                snapshot.manifest.label.as_deref().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn verify_loaded(root: &Path, snapshot: &Snapshot) -> Result<usize, AppError> {
    let mut verified = BTreeSet::new();
    for entry in &snapshot.manifest.entries {
        if let Entry::File { sha256, size, .. } = entry
            && verified.insert((sha256, size))
        {
            verify_object(root, sha256, *size)?;
        }
    }
    Ok(verified.len())
}

pub fn verify(path: &Path, id: &str) -> Result<(), AppError> {
    let root = initialized(path)?;
    let snapshot = load(&root, id)?;
    let verified = verify_loaded(&root, &snapshot)?;
    println!(
        "Snapshot {id} verified: manifest and {} unique objects passed SHA-256 checks",
        verified
    );
    Ok(())
}

fn destination(path: &Path) -> Result<PathBuf, AppError> {
    let path: PathBuf = path.components().collect();
    if exists(&path)? {
        return Err(invalid(format!(
            "Restore destination already exists and will not be overwritten: {path:?}"
        )));
    }
    let parent = path.parent().ok_or_else(|| {
        invalid("Restore destination must name a new directory below an existing directory")
    })?;
    require_directory(parent)?;
    let parent = io_at(
        fs::canonicalize(parent),
        "resolve restore destination parent",
        parent,
    )?;
    let name = path.file_name().ok_or_else(|| {
        invalid("Restore destination must name a new directory below an existing directory")
    })?;
    Ok(parent.join(name))
}

fn restore_file(root: &Path, destination: &Path, hash: &str, size: u64) -> Result<(), AppError> {
    verify_object(root, hash, size)?;
    let source = object_path(root, hash).join("data");
    let mut output = io_at(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination),
        "create restored file without overwriting",
        destination,
    )?;
    let captured = hash_file(&source, Some(&mut output))?;
    io_at(output.sync_all(), "flush restored file", destination)?;
    drop(output);
    if captured != (hash.into(), size) || hash_file(destination, None)? != captured {
        return Err(invalid(format!(
            "SHA-256 verification failed while restoring {destination:?}"
        )));
    }
    Ok(())
}

fn materialize(root: &Path, snapshot: &Snapshot, staged: &Path) -> Result<(), AppError> {
    for entry in &snapshot.manifest.entries {
        match entry {
            Entry::Directory { path } if path.is_empty() => {}
            Entry::Directory { path } => {
                let directory = staged.join(path);
                io_at(
                    fs::create_dir(&directory),
                    "create restored directory",
                    &directory,
                )?;
            }
            Entry::File { path, sha256, size } => {
                let file = if path.is_empty() {
                    staged.join(&snapshot.manifest.source_name)
                } else {
                    staged.join(path)
                };
                restore_file(root, &file, sha256, *size)?;
            }
        }
    }
    Ok(())
}

fn restore_staged(root: &Path, snapshot: &Snapshot, destination: &Path) -> Result<(), AppError> {
    let staged = create_temp_dir_sibling(destination)?;
    let result = (|| {
        materialize(root, snapshot, &staged)?;
        publish(&staged, destination)
    })();
    let cleanup = match fs::remove_dir_all(&staged) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => io_at(Err(error), "remove failed restore staging", &staged),
    };
    combine(result, cleanup)
}

/// Restore into a new directory only. The caller must explicitly select a safe
/// test or replacement directory; live saves are never overwritten implicitly.
pub fn restore(path: &Path, id: &str, target: &Path, dry_run: bool) -> Result<(), AppError> {
    let root = initialized(path)?;
    let destination = destination(target)?;
    if destination.starts_with(&root) {
        return Err(invalid("Restore destination must not be inside the vault"));
    }
    if dry_run {
        super::ensure_unlocked(&root)?;
        let snapshot = load(&root, id)?;
        verify_loaded(&root, &snapshot)?;
        let (files, bytes) = totals(&snapshot.manifest)?;
        println!(
            "[DRY-RUN] Would restore snapshot {id} to {destination:?}: {files} files, {bytes} bytes; no files written"
        );
        return Ok(());
    }
    let lock = VaultLock::acquire(&root)?;
    let result = (|| {
        initialized(&root)?;
        let snapshot = load(&root, id)?;
        verify_loaded(&root, &snapshot)?;
        // Check again immediately before publication in case another process
        // created the requested directory after the initial validation.
        if exists(&destination)? {
            return Err(invalid(format!(
                "Restore destination already exists and will not be overwritten: {destination:?}"
            )));
        }
        restore_staged(&root, &snapshot, &destination)
    })();
    combine(result, lock.release())?;
    let snapshot = load(&root, id)?;
    let (files, bytes) = totals(&snapshot.manifest)?;
    println!("Snapshot {id} restored to {destination:?}: {files} files, {bytes} bytes");
    Ok(())
}
