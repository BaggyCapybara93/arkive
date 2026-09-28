//! Named, host-local mappings from game names to save paths.
#[cfg(test)]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    CONTROL, VaultLock, combine, ensure_unlocked, exists, invalid, io_at, require_directory, root,
    validate, with_scratch, write_new,
};
use crate::error::AppError;

const FORMAT: &str = "arkive-profile";
const VERSION: u32 = 1;
const MAX_PROFILE_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 128;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalBinding {
    vault: PathBuf,
    source: PathBuf,
}

// Profiles live on the shared vault, but approval of a host path must live on
// the host. A vault writer must not be able to choose another user's source.
fn binding_path(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    profile_name(name)?;
    #[cfg(test)]
    let home = root.parent().unwrap().join("test-home");
    #[cfg(not(test))]
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| invalid("A user home directory is required for profile bindings"))?;
    if !home.is_absolute() {
        return Err(invalid("Profile binding home must be an absolute path"));
    }
    #[cfg(test)]
    if !home.exists() {
        fs::create_dir(&home)?;
    }
    private_directory(&home, false)?;
    let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
    let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let bindings = home.join(".arkive-profile-bindings");
    private_directory(&bindings, true)?;
    let vault = bindings.join(key);
    private_directory(&vault, true)?;
    Ok(vault.join(format!("{name}.json")))
}

fn private_directory(path: &Path, create: bool) -> Result<(), AppError> {
    if create && !exists(path)? {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(path)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(path)?;
    }
    require_directory(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        let allowed = if create { 0o077 } else { 0o022 };
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & allowed != 0 {
            return Err(invalid(format!(
                "Profile binding directory is not private to this user: {path:?}"
            )));
        }
    }
    Ok(())
}

fn local_binding(root: &Path, name: &str) -> Result<Option<LocalBinding>, AppError> {
    let path = binding_path(root, name)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return io_at(Err(error), "inspect profile binding", &path),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Profile binding must be a regular file: {path:?}"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(invalid(format!("Profile binding is not private: {path:?}")));
        }
    }
    let mut bytes = Vec::new();
    io_at(
        File::open(&path)?
            .take(MAX_PROFILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes),
        "read profile binding",
        &path,
    )?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(invalid("Profile binding exceeds 64 KiB"));
    }
    let binding: LocalBinding = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid local profile binding: {error}")))?;
    Ok(Some(binding))
}

fn bind(root: &Path, name: &str, source: &Path, replace: bool) -> Result<(), AppError> {
    if let Some(binding) = local_binding(root, name)? {
        if binding.vault == root && binding.source == source {
            return Ok(());
        }
        if !replace {
            return Err(invalid(format!(
                "Profile {name:?} has a different local source binding"
            )));
        }
        let path = binding_path(root, name)?;
        io_at(fs::remove_file(&path), "replace profile binding", &path)?;
    }
    let path = binding_path(root, name)?;
    let bytes = serde_json::to_vec(&LocalBinding {
        vault: root.to_path_buf(),
        source: source.to_path_buf(),
    })
    .map_err(|error| invalid(format!("Could not encode profile binding: {error}")))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = io_at(options.open(&path), "create profile binding", &path)?;
    io_at(file.write_all(&bytes), "write profile binding", &path)?;
    io_at(file.sync_all(), "flush profile binding", &path)
}

fn require_binding(root: &Path, name: &str, source: &Path) -> Result<(), AppError> {
    let Some(binding) = local_binding(root, name)? else {
        return Err(invalid(format!(
            "Profile {name:?} has no local source binding; run vault profile add with its approved source on this host"
        )));
    };
    if binding.vault != root || binding.source != source {
        return Err(invalid(format!(
            "Profile {name:?} source differs from its locally approved path"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    format: String,
    version: u32,
    pub name: String,
    pub source: PathBuf,
    pub created_at: DateTime<Utc>,
}

fn profile_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', ':', '\0'])
    {
        return Err(invalid(format!(
            "Profile names must be 1-{MAX_NAME_BYTES} bytes and cannot contain path separators: {name:?}"
        )));
    }
    Ok(())
}

fn profile_path(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    profile_name(name)?;
    Ok(root
        .join(CONTROL)
        .join("profiles")
        .join(format!("{name}.json")))
}

fn validate_profile(profile: &Profile, name: &str) -> Result<(), AppError> {
    if profile.format != FORMAT || profile.version != VERSION {
        return Err(invalid("Unsupported profile format/version"));
    }
    profile_name(name)?;
    if profile.name != name {
        return Err(invalid(format!(
            "Profile name does not match its file: {name:?}"
        )));
    }
    if !profile.source.is_absolute() {
        return Err(invalid(format!(
            "Profile source must be an absolute path: {:?}",
            profile.source
        )));
    }
    Ok(())
}

fn load(root: &Path, name: &str) -> Result<Profile, AppError> {
    let path = profile_path(root, name)?;
    let metadata = io_at(fs::symlink_metadata(&path), "inspect profile", &path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!("Profile must be a regular file: {path:?}")));
    }
    let mut bytes = Vec::new();
    io_at(
        File::open(&path)?
            .take(MAX_PROFILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes),
        "read profile",
        &path,
    )?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(invalid(format!(
            "Profile exceeds {MAX_PROFILE_BYTES} bytes"
        )));
    }
    let profile: Profile = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid profile {name:?}: {error}")))?;
    validate_profile(&profile, name)?;
    Ok(profile)
}

fn all(root: &Path) -> Result<Vec<Profile>, AppError> {
    let directory = root.join(CONTROL).join("profiles");
    require_directory(&directory)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Profile file names must be UTF-8"))?;
        let name = file_name.strip_suffix(".json").ok_or_else(|| {
            invalid(format!(
                "Unexpected entry in profiles directory: {file_name:?}"
            ))
        })?;
        result.push(load(root, name)?);
    }
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

fn source_path(root: &Path, source: &Path) -> Result<PathBuf, AppError> {
    let source: PathBuf = source.components().collect();
    let metadata = io_at(
        fs::symlink_metadata(&source),
        "inspect profile source",
        &source,
    )?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid(
            "Profile source must be a regular file or directory, not a symlink",
        ));
    }
    let source = io_at(fs::canonicalize(&source), "resolve profile source", &source)?;
    if source.starts_with(root) || root.starts_with(&source) {
        return Err(invalid("Profile source and vault must not overlap"));
    }
    Ok(source)
}

/// Resolve and validate a profile's current source path before a snapshot comparison.
pub(super) fn source(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    let profile = load(root, name)?;
    require_binding(root, name, &profile.source)?;
    source_path(root, &profile.source)
}

pub fn add(path: &Path, name: &str, source: &Path, dry_run: bool) -> Result<(), AppError> {
    let root = root(path)?;
    if !validate(&root)? {
        return Err(invalid(format!(
            "No vault at {root:?}; run vault init first"
        )));
    }
    let destination = profile_path(&root, name)?;
    let source = source_path(&root, source)?;
    if dry_run {
        ensure_unlocked(&root)?;
        if exists(&destination)? {
            let existing = load(&root, name)?;
            if existing.source != source || local_binding(&root, name)?.is_some() {
                return Err(invalid(format!("Profile already exists: {name:?}")));
            }
            println!("[DRY-RUN] Would approve profile {name:?} for {source:?} on this host");
            return Ok(());
        }
        println!("[DRY-RUN] Would add profile {name:?} for {source:?}; no profile written");
        return Ok(());
    }

    let lock = VaultLock::acquire(&root)?;
    let result = (|| {
        validate(&root)?;
        if exists(&destination)? {
            let existing = load(&root, name)?;
            if existing.source != source || local_binding(&root, name)?.is_some() {
                return Err(invalid(format!("Profile already exists: {name:?}")));
            }
            bind(&root, name, &source, false)?;
            return Ok(());
        }
        let profile = Profile {
            format: FORMAT.into(),
            version: VERSION,
            name: name.into(),
            source: source.clone(),
            created_at: Utc::now(),
        };
        let bytes = serde_json::to_vec_pretty(&profile)
            .map_err(|error| invalid(format!("Could not encode profile: {error}")))?;
        bind(&root, name, &source, true)?;
        with_scratch(&root, |scratch| {
            let staged = scratch.join("profile.json");
            write_new(&staged, &bytes)?;
            if exists(&destination)? {
                return Err(invalid(format!(
                    "Profile appeared during creation: {name:?}"
                )));
            }
            io_at(
                fs::rename(&staged, &destination),
                "publish profile",
                &destination,
            )?;
            load(&root, name)?;
            Ok(())
        })
    })();
    combine(result, lock.release())?;
    println!("Profile {name:?} added");
    Ok(())
}

pub fn list(path: &Path, json: bool) -> Result<(), AppError> {
    let root = root(path)?;
    if !validate(&root)? {
        return Err(invalid(format!(
            "No vault at {root:?}; run vault init first"
        )));
    }
    let profiles = all(&root)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&profiles).map_err(|error| invalid(error.to_string()))?
        );
    } else if profiles.is_empty() {
        println!("No profiles in {root:?}");
    } else {
        for profile in profiles {
            println!("{}  source={:?}", profile.name, profile.source);
        }
    }
    Ok(())
}

pub fn remove(path: &Path, name: &str, dry_run: bool) -> Result<(), AppError> {
    let root = root(path)?;
    let profile = load(&root, name)?;
    let destination = profile_path(&root, name)?;
    if dry_run {
        ensure_unlocked(&root)?;
        println!(
            "[DRY-RUN] Would remove profile {:?}; snapshots and save data would be preserved",
            profile.name
        );
        return Ok(());
    }
    let lock = VaultLock::acquire(&root)?;
    let result = (|| {
        validate(&root)?;
        load(&root, name)?;
        let binding = binding_path(&root, name)?;
        if exists(&binding)? {
            io_at(
                fs::remove_file(&binding),
                "remove profile binding",
                &binding,
            )?;
        }
        io_at(
            fs::remove_file(&destination),
            "remove profile",
            &destination,
        )
    })();
    combine(result, lock.release())?;
    println!("Profile {name:?} removed; snapshots and save data were preserved");
    Ok(())
}
