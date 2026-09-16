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

#[cfg(test)]
pub fn create(
    path: &Path,
    source: &Path,
    label: Option<String>,
    dry_run: bool,
) -> Result<(), AppError> {
    create_selected(path, Some(source), None, label, dry_run)
}

pub fn create_selected(
    path: &Path,
    source: Option<&Path>,
    profile: Option<&str>,
    label: Option<String>,
    dry_run: bool,
) -> Result<(), AppError> {
    let root = initialized(path)?;
    let source = match (source, profile) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                "Provide either a source path or --profile, not both",
            ));
        }
        (Some(source), None) => source.to_path_buf(),
        (None, Some(profile)) => super::profiles::source(&root, profile)?,
        (None, None) => return Err(invalid("Provide a source path or --profile")),
    };
    if dry_run {
        super::ensure_unlocked(&root)?;
        let snapshot = capture(&root, &source, label, None)?;
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
            snapshot = Some(capture(&root, &source, label, Some(scratch))?);
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

#[derive(Debug, Serialize)]
struct HealthReport {
    vault: String,
    snapshots: usize,
    referenced_objects: usize,
    stored_objects: usize,
    stored_bytes: u64,
    orphaned_objects: usize,
    orphaned_bytes: u64,
    issues: Vec<String>,
}

#[derive(Debug)]
struct ReclaimableObject {
    path: PathBuf,
}

#[derive(Debug)]
struct Audit {
    report: HealthReport,
    reclaimable: Vec<ReclaimableObject>,
}

fn inspect_stored_object(root: &Path, hash: &str) -> Result<u64, AppError> {
    let directory = object_path(root, hash);
    require_directory(&directory)?;
    let data = directory.join("data");
    let (actual, size) = hash_file(&data, None)?;
    if actual != hash {
        return Err(invalid(format!(
            "Object {hash} has SHA-256 {actual}, not its directory hash"
        )));
    }
    Ok(size)
}

fn audit(root: &Path) -> Result<Audit, AppError> {
    let mut issues = Vec::new();
    let mut references = BTreeMap::new();
    let snapshots_dir = root.join(CONTROL).join("snapshots");
    let mut snapshot_count = 0;

    for entry in fs::read_dir(&snapshots_dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = io_at(fs::symlink_metadata(&path), "inspect snapshot entry", &path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            issues.push(format!(
                "Snapshot entry is not a regular directory: {path:?}"
            ));
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            issues.push(format!("Snapshot entry has a non-UTF-8 name: {path:?}"));
            continue;
        };
        if !valid_id(&id) {
            issues.push(format!("Snapshot directory has an invalid ID: {path:?}"));
            continue;
        }
        let snapshot = match load(root, &id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                issues.push(format!("Snapshot {id} is invalid: {error}"));
                continue;
            }
        };
        snapshot_count += 1;
        for entry in snapshot.manifest.entries {
            if let Entry::File { sha256, size, .. } = entry
                && let Some(previous) = references.insert(sha256.clone(), size)
                && previous != size
            {
                issues.push(format!(
                    "Object {sha256} is referenced with conflicting sizes: {previous} and {size}"
                ));
            }
        }
    }

    let objects_dir = root.join(CONTROL).join("objects");
    let mut stored = BTreeMap::new();
    let mut stored_bytes = 0u64;
    for entry in fs::read_dir(&objects_dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = io_at(fs::symlink_metadata(&path), "inspect object entry", &path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            issues.push(format!("Object entry is not a regular directory: {path:?}"));
            continue;
        }
        let Some(hash) = entry.file_name().to_str().map(str::to_owned) else {
            issues.push(format!("Object entry has a non-UTF-8 name: {path:?}"));
            continue;
        };
        if !valid_id(&hash) {
            issues.push(format!("Object directory has an invalid hash: {path:?}"));
            continue;
        }
        match inspect_stored_object(root, &hash) {
            Ok(bytes) => {
                stored_bytes = stored_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| invalid("Stored object byte total overflow"))?;
                stored.insert(hash, Some(bytes));
            }
            Err(error) => {
                issues.push(format!("Object {hash} is invalid: {error}"));
                stored.insert(hash, None);
            }
        }
    }

    for (hash, expected_size) in &references {
        match stored.get(hash) {
            None => issues.push(format!("Referenced object is missing: {hash}")),
            Some(Some(actual_size)) if actual_size != expected_size => issues.push(format!(
                "Referenced object {hash} has size {actual_size}, expected {expected_size}"
            )),
            Some(Some(_)) | Some(None) => {}
        }
    }

    let mut reclaimable = Vec::new();
    let mut orphaned_bytes = 0u64;
    for (hash, size) in &stored {
        let Some(bytes) = size else {
            continue;
        };
        if !references.contains_key(hash) {
            orphaned_bytes = orphaned_bytes
                .checked_add(*bytes)
                .ok_or_else(|| invalid("Orphaned object byte total overflow"))?;
            reclaimable.push(ReclaimableObject {
                path: object_path(root, hash),
            });
        }
    }

    Ok(Audit {
        report: HealthReport {
            vault: root.display().to_string(),
            snapshots: snapshot_count,
            referenced_objects: references.len(),
            stored_objects: stored.len(),
            stored_bytes,
            orphaned_objects: reclaimable.len(),
            orphaned_bytes,
            issues,
        },
        reclaimable,
    })
}

fn print_health(report: &HealthReport, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report)
                .map_err(|error| invalid(format!("Could not encode health report: {error}")))?
        );
        return Ok(());
    }

    if report.issues.is_empty() {
        println!("Vault healthy: {:?}", report.vault);
    } else {
        println!("Vault health: {} issue(s)", report.issues.len());
        for issue in &report.issues {
            println!("- {issue}");
        }
    }
    println!(
        "{} snapshot(s), {} stored object(s), {} referenced object(s), {} orphaned object(s) ({} bytes)",
        report.snapshots,
        report.stored_objects,
        report.referenced_objects,
        report.orphaned_objects,
        report.orphaned_bytes
    );
    Ok(())
}

pub fn health(path: &Path, json: bool) -> Result<(), AppError> {
    let root = initialized(path)?;
    super::ensure_unlocked(&root)?;
    let audit = audit(&root)?;
    print_health(&audit.report, json)
}

#[derive(Debug, Serialize)]
struct GcReport {
    vault: String,
    dry_run: bool,
    objects: usize,
    bytes: u64,
}

fn print_gc(report: &GcReport, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| invalid(format!(
                "Could not encode garbage-collection report: {error}"
            )))?
        );
    } else if report.dry_run {
        println!(
            "[DRY-RUN] Would remove {} unreferenced object(s) ({} bytes) from {:?}; no files written",
            report.objects, report.bytes, report.vault
        );
    } else {
        println!(
            "Garbage collection removed {} unreferenced object(s) ({} bytes) from {:?}",
            report.objects, report.bytes, report.vault
        );
    }
    Ok(())
}

fn require_auditable(audit: &Audit) -> Result<(), AppError> {
    if audit.report.issues.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "Refusing garbage collection: vault health found {} issue(s); run `arkive vault health` for details",
            audit.report.issues.len()
        )))
    }
}

pub fn gc(path: &Path, dry_run: bool, json: bool) -> Result<(), AppError> {
    let root = initialized(path)?;
    if dry_run {
        super::ensure_unlocked(&root)?;
        let audit = audit(&root)?;
        require_auditable(&audit)?;
        let report = GcReport {
            vault: root.display().to_string(),
            dry_run: true,
            objects: audit.reclaimable.len(),
            bytes: audit.report.orphaned_bytes,
        };
        return print_gc(&report, json);
    }

    let lock = VaultLock::acquire(&root)?;
    let mut report = None;
    let result = (|| {
        initialized(&root)?;
        let audit = audit(&root)?;
        require_auditable(&audit)?;
        for object in &audit.reclaimable {
            let metadata = io_at(
                fs::symlink_metadata(&object.path),
                "inspect reclaimable object",
                &object.path,
            )?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(invalid(format!(
                    "Reclaimable object changed before deletion: {:?}",
                    object.path
                )));
            }
            io_at(
                fs::remove_dir_all(&object.path),
                "remove unreferenced object",
                &object.path,
            )?;
        }
        report = Some(GcReport {
            vault: root.display().to_string(),
            dry_run: false,
            objects: audit.reclaimable.len(),
            bytes: audit.report.orphaned_bytes,
        });
        Ok(())
    })();
    combine(result, lock.release())?;
    print_gc(
        &report.expect("successful garbage collection creates a report"),
        json,
    )
}

#[derive(Debug, Serialize)]
struct PrunedSnapshot {
    id: String,
    created_at: DateTime<Utc>,
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct PruneReport {
    vault: String,
    dry_run: bool,
    keep_last: usize,
    protected: Vec<String>,
    retained: usize,
    removed: Vec<PrunedSnapshot>,
}

#[derive(Debug)]
struct PrunePlan {
    report: PruneReport,
}

fn prune_plan(
    root: &Path,
    snapshots: &[Snapshot],
    keep_last: usize,
    protected: &[String],
    dry_run: bool,
) -> Result<PrunePlan, AppError> {
    if keep_last == 0 {
        return Err(invalid("--keep-last must be at least 1"));
    }

    let snapshot_ids: BTreeSet<_> = snapshots
        .iter()
        .map(|snapshot| snapshot.id.as_str())
        .collect();
    let mut protected_ids = BTreeSet::new();
    for id in protected {
        if !valid_id(id) {
            return Err(invalid(format!(
                "Protected snapshot ID must be 64 lowercase hexadecimal characters: {id:?}"
            )));
        }
        if !snapshot_ids.contains(id.as_str()) {
            return Err(invalid(format!("Protected snapshot was not found: {id}")));
        }
        protected_ids.insert(id.clone());
    }

    let retained_ids: BTreeSet<_> = snapshots
        .iter()
        .take(keep_last)
        .map(|snapshot| snapshot.id.as_str())
        .chain(protected_ids.iter().map(String::as_str))
        .collect();
    let removed: Vec<_> = snapshots
        .iter()
        .rev()
        .filter(|snapshot| !retained_ids.contains(snapshot.id.as_str()))
        .map(|snapshot| PrunedSnapshot {
            id: snapshot.id.clone(),
            created_at: snapshot.manifest.created_at,
            label: snapshot.manifest.label.clone(),
        })
        .collect();

    Ok(PrunePlan {
        report: PruneReport {
            vault: root.display().to_string(),
            dry_run,
            keep_last,
            protected: protected_ids.into_iter().collect(),
            retained: snapshots.len() - removed.len(),
            removed,
        },
    })
}

fn print_prune(report: &PruneReport, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report)
                .map_err(|error| invalid(format!("Could not encode prune report: {error}")))?
        );
        return Ok(());
    }

    if report.dry_run {
        println!(
            "[DRY-RUN] Would remove {} snapshot(s) from {:?}; {} would remain",
            report.removed.len(),
            report.vault,
            report.retained
        );
    } else {
        println!(
            "Pruned {} snapshot(s) from {:?}; {} remain",
            report.removed.len(),
            report.vault,
            report.retained
        );
    }
    for snapshot in &report.removed {
        println!(
            "{}  {}  label={:?}",
            snapshot.id,
            snapshot.created_at.to_rfc3339(),
            snapshot.label.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

pub fn prune(
    path: &Path,
    keep_last: usize,
    protected: &[String],
    dry_run: bool,
    json: bool,
) -> Result<(), AppError> {
    let root = initialized(path)?;
    if dry_run {
        super::ensure_unlocked(&root)?;
        let audit = audit(&root)?;
        require_auditable(&audit)?;
        let plan = prune_plan(&root, &history(&root)?, keep_last, protected, true)?;
        return print_prune(&plan.report, json);
    }

    let lock = VaultLock::acquire(&root)?;
    let mut report = None;
    let result = (|| {
        initialized(&root)?;
        let audit = audit(&root)?;
        require_auditable(&audit)?;
        let snapshots = history(&root)?;
        let plan = prune_plan(&root, &snapshots, keep_last, protected, false)?;

        for snapshot in &plan.report.removed {
            let directory = root.join(CONTROL).join("snapshots").join(&snapshot.id);
            require_directory(&directory)?;
        }
        for snapshot in &plan.report.removed {
            let directory = root.join(CONTROL).join("snapshots").join(&snapshot.id);
            io_at(
                fs::remove_dir_all(&directory),
                "remove pruned snapshot",
                &directory,
            )?;
        }
        report = Some(plan.report);
        Ok(())
    })();
    combine(result, lock.release())?;
    print_prune(&report.expect("successful pruning creates a report"), json)
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ChangeKind {
    Added,
    Modified,
    Removed,
    TypeChanged,
}

#[derive(Debug, Serialize)]
struct Change {
    change: ChangeKind,
    path: String,
    previous: Option<Entry>,
    current: Option<Entry>,
}

#[derive(Debug, Serialize)]
struct DiffReport {
    snapshot: Option<String>,
    source: String,
    changes: Vec<Change>,
}

fn compare_entries(previous: &[Entry], current: &[Entry]) -> Vec<Change> {
    let previous: BTreeMap<_, _> = previous
        .iter()
        .map(|entry| (entry.path().to_owned(), entry.clone()))
        .collect();
    let current: BTreeMap<_, _> = current
        .iter()
        .map(|entry| (entry.path().to_owned(), entry.clone()))
        .collect();
    let paths: BTreeSet<_> = previous.keys().chain(current.keys()).cloned().collect();

    paths
        .into_iter()
        .filter_map(|path| {
            let old = previous.get(&path);
            let new = current.get(&path);
            let change = match (old, new) {
                (Some(old), Some(new)) if old == new => return None,
                (Some(old), Some(new)) if entry_kind(old) != entry_kind(new) => {
                    ChangeKind::TypeChanged
                }
                (Some(_), Some(_)) => ChangeKind::Modified,
                (None, Some(_)) => ChangeKind::Added,
                (Some(_), None) => ChangeKind::Removed,
                (None, None) => return None,
            };
            Some(Change {
                change,
                path,
                previous: old.cloned(),
                current: new.cloned(),
            })
        })
        .collect()
}

fn comparison_source(root: &Path, source: &Path) -> Result<PathBuf, AppError> {
    let source: PathBuf = source.components().collect();
    let metadata = io_at(
        fs::symlink_metadata(&source),
        "inspect comparison source",
        &source,
    )?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid(
            "Comparison source must be a regular file or directory, not a symlink",
        ));
    }
    let source = fs::canonicalize(&source)?;
    if source.starts_with(root) || root.starts_with(&source) {
        return Err(invalid("Comparison source and vault must not overlap"));
    }
    Ok(source)
}

#[cfg(test)]
fn build_report(
    root: &Path,
    snapshot: Option<&Snapshot>,
    source: &Path,
) -> Result<DiffReport, AppError> {
    build_report_selected(root, snapshot, Some(source), None)
}

fn build_report_selected(
    root: &Path,
    snapshot: Option<&Snapshot>,
    source: Option<&Path>,
    profile: Option<&str>,
) -> Result<DiffReport, AppError> {
    let source = match (source, profile) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                "Provide either a source path or --profile, not both",
            ));
        }
        (Some(source), None) => source.to_path_buf(),
        (None, Some(profile)) => super::profiles::source(root, profile)?,
        (None, None) => return Err(invalid("Provide a source path or --profile")),
    };
    let source = comparison_source(root, &source)?;
    let current = scan(&source)?;
    let (snapshot, changes) = match snapshot {
        Some(snapshot) => (
            Some(snapshot.id.clone()),
            compare_entries(&snapshot.manifest.entries, &current),
        ),
        None => (None, Vec::new()),
    };
    Ok(DiffReport {
        snapshot,
        source: source.display().to_string(),
        changes,
    })
}

fn entry_kind(entry: &Entry) -> &'static str {
    match entry {
        Entry::Directory { .. } => "directory",
        Entry::File { .. } => "file",
    }
}

fn print_report(report: &DiffReport, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| invalid(error.to_string()))?
        );
        return Ok(());
    }

    let Some(snapshot) = &report.snapshot else {
        println!("No snapshots available for {:?}", report.source);
        return Ok(());
    };
    if report.changes.is_empty() {
        println!("Snapshot {snapshot} matches {:?}", report.source);
        return Ok(());
    }

    println!(
        "Snapshot {snapshot} differs from {:?}: {} change(s)",
        report.source,
        report.changes.len()
    );
    for change in &report.changes {
        let entry = change.current.as_ref().or(change.previous.as_ref());
        let kind = entry.map(entry_kind).unwrap_or("unknown");
        let path = if change.path.is_empty() {
            "."
        } else {
            &change.path
        };
        println!("{:?} {kind} {path:?}", change.change);
    }
    Ok(())
}

pub fn diff_selected(
    path: &Path,
    id: &str,
    source: Option<&Path>,
    profile: Option<&str>,
    json: bool,
) -> Result<(), AppError> {
    let root = initialized(path)?;
    let snapshot = load(&root, id)?;
    let report = build_report_selected(&root, Some(&snapshot), source, profile)?;
    print_report(&report, json)
}

pub fn status_selected(
    path: &Path,
    source: Option<&Path>,
    profile: Option<&str>,
    json: bool,
) -> Result<(), AppError> {
    let root = initialized(path)?;
    let snapshots = history(&root)?;
    let report = build_report_selected(&root, snapshots.first(), source, profile)?;
    print_report(&report, json)
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
