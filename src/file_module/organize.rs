//! Shallow, reversible organization of regular files by extension.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::file_module::FileManager;
use crate::file_module::ops::create_temp_file_sibling;
use crate::settings::Settings;

const FOLDER_FILE: &str = ".arkive-folder.json";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveEntry {
    name: String,
    folder: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunManifest {
    format: String,
    version: u32,
    folder_metadata: bool,
    created_folders: Vec<String>,
    existing_metadata: Vec<String>,
    moves: Vec<MoveEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderMetadata {
    format: String,
    version: u32,
    rule: String,
    files: BTreeMap<String, String>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidInput(message.into())
}

fn regular_file(path: &Path) -> Result<bool, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(invalid(format!("Symbolic link is not allowed: {path:?}")));
            }
            if !metadata.is_file() {
                return Err(invalid(format!("Expected a regular file: {path:?}")));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn single_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn folder_for(name: &str) -> String {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension.is_empty() {
        "no-extension".into()
    } else if extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        extension
    } else {
        "other-extension".into()
    }
}

fn sha256(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn folder_path(root: &Path, folder: &str) -> Result<PathBuf, AppError> {
    let path = root.join(folder);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(path),
        Ok(_) => Err(invalid(format!(
            "Destination folder is not a regular directory: {path:?}"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error.into()),
    }
}

fn read_folder(path: &Path, folder: &str) -> Result<Option<FolderMetadata>, AppError> {
    if !regular_file(path)? {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid(format!("Folder metadata is too large: {path:?}")));
    }
    let metadata: FolderMetadata = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid folder metadata {path:?}: {error}")))?;
    if metadata.format != "arkive-folder"
        || metadata.version != 1
        || metadata.rule != format!("extension:{folder}")
    {
        return Err(invalid(format!(
            "Folder metadata has an incompatible rule: {path:?}"
        )));
    }
    if metadata.files.keys().any(|name| !single_name(name)) {
        return Err(invalid(format!(
            "Folder metadata has an unsafe file name: {path:?}"
        )));
    }
    Ok(Some(metadata))
}

fn write_folder(path: &Path, metadata: &FolderMetadata) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|error| invalid(format!("Could not encode folder metadata: {error}")))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid(format!(
            "Folder metadata would be too large: {path:?}"
        )));
    }
    let temporary = create_temp_file_sibling(path)?;
    let result = (|| -> Result<(), AppError> {
        let mut file = OpenOptions::new().write(true).open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn move_no_replace(source: &Path, destination: &Path) -> Result<(), AppError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Hard-link creation fails if the destination exists. Both paths are
        // inside the same directory tree, so this never crosses volumes.
        fs::hard_link(source, destination)?;
        fs::remove_file(source)?;
    }
    Ok(())
}

fn fresh_manifest_path(root: &Path) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%fZ");
    root.join(format!(
        ".arkive-organize-{timestamp}-{}.json",
        std::process::id()
    ))
}

pub fn by_extension(
    path: &Path,
    folder_metadata: bool,
    settings: &Settings,
) -> Result<(), AppError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(invalid("Organize root must not be a symbolic link"));
    }
    let root = fs::canonicalize(path)?;
    if !root.is_dir() {
        return Err(invalid(format!(
            "Organize root is not a directory: {root:?}"
        )));
    }
    if root.join(".arkive-vault").exists() {
        return Err(invalid("Cannot organize an Arkive vault directory"));
    }
    let _guard = FileManager::acquire_paths([root.as_path()]);
    let mut moves = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("File name is not UTF-8"))?;
        if name.starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid(format!(
                "Symbolic link is not allowed: {:?}",
                entry.path()
            )));
        }
        if !kind.is_file() {
            continue;
        }
        let folder = folder_for(&name);
        let destination = folder_path(&root, &folder)?.join(&name);
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(invalid(format!(
                    "Destination already exists: {destination:?}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        moves.push(MoveEntry {
            name,
            folder,
            sha256: sha256(&entry.path())?,
        });
    }
    moves.sort_by(|a, b| a.name.cmp(&b.name));
    if moves.is_empty() {
        println!("No files to organize in {}", root.display());
        return Ok(());
    }
    let mut folders = BTreeSet::new();
    for item in &moves {
        folders.insert(item.folder.clone());
        println!(
            "{}{} -> {}",
            if settings.dry_run { "[DRY-RUN] " } else { "" },
            item.name,
            Path::new(&item.folder).join(&item.name).display()
        );
    }
    let mut existing_metadata = Vec::new();
    if folder_metadata {
        for folder in &folders {
            let metadata_path = root.join(folder).join(FOLDER_FILE);
            if let Some(metadata) = read_folder(&metadata_path, folder)? {
                existing_metadata.push(folder.clone());
                for item in moves.iter().filter(|item| &item.folder == folder) {
                    if metadata.files.contains_key(&item.name) {
                        return Err(invalid(format!(
                            "Folder metadata already lists {:?}",
                            item.name
                        )));
                    }
                }
            }
        }
    }
    if settings.dry_run {
        println!(
            "[DRY-RUN] Would sort {} file(s) into {} folder(s)",
            moves.len(),
            folders.len()
        );
        return Ok(());
    }

    let created_folders = folders
        .iter()
        .filter(|folder| !root.join(folder).exists())
        .cloned()
        .collect();
    let manifest = RunManifest {
        format: "arkive-organize".into(),
        version: 1,
        folder_metadata,
        created_folders,
        existing_metadata,
        moves,
    };
    let manifest_path = fresh_manifest_path(&root);
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| invalid(format!("Could not encode organize manifest: {error}")))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid("Organize plan is too large for an undo manifest"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    println!("Undo manifest: {}", manifest_path.display());

    for item in &manifest.moves {
        let folder = folder_path(&root, &item.folder)?;
        if !folder.exists() {
            fs::create_dir(&folder)?;
        }
        let source = root.join(&item.name);
        let destination = folder.join(&item.name);
        if !regular_file(&source)? || sha256(&source)? != item.sha256 {
            return Err(invalid(format!(
                "Source changed; stop and inspect {:?}. Undo with {:?}",
                source, manifest_path
            )));
        }
        move_no_replace(&source, &destination)?;
        if folder_metadata {
            let metadata_path = folder.join(FOLDER_FILE);
            let mut metadata =
                read_folder(&metadata_path, &item.folder)?.unwrap_or(FolderMetadata {
                    format: "arkive-folder".into(),
                    version: 1,
                    rule: format!("extension:{}", item.folder),
                    files: BTreeMap::new(),
                });
            if metadata.files.contains_key(&item.name) {
                return Err(invalid(format!(
                    "Folder metadata already lists {:?}; undo with {:?}",
                    item.name, manifest_path
                )));
            }
            metadata
                .files
                .insert(item.name.clone(), item.sha256.clone());
            write_folder(&metadata_path, &metadata)?;
        }
    }
    println!(
        "Organized {} file(s) into {} folder(s)",
        manifest.moves.len(),
        folders.len()
    );
    Ok(())
}

pub fn undo(manifest_path: &Path, settings: &Settings) -> Result<(), AppError> {
    if !manifest_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".arkive-organize-") && name.ends_with(".json"))
    {
        return Err(invalid("Expected an .arkive-organize-*.json manifest"));
    }
    if !regular_file(manifest_path)? {
        return Err(invalid(format!(
            "Undo manifest is not a regular file: {manifest_path:?}"
        )));
    }
    let path = fs::canonicalize(manifest_path)?;
    let root = path
        .parent()
        .ok_or_else(|| invalid("Undo manifest has no parent"))?;
    let _guard = FileManager::acquire_paths([root]);
    if fs::metadata(&path)?.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("Undo manifest is too large"));
    }
    let manifest: RunManifest = serde_json::from_slice(&fs::read(&path)?)
        .map_err(|error| invalid(format!("Invalid undo manifest: {error}")))?;
    if manifest.format != "arkive-organize" || manifest.version != 1 {
        return Err(invalid("Unsupported undo manifest format/version"));
    }
    let created: BTreeSet<_> = manifest.created_folders.iter().collect();
    if created.len() != manifest.created_folders.len()
        || created.iter().any(|folder| !single_name(folder))
        || created
            .iter()
            .any(|folder| !manifest.moves.iter().any(|item| &item.folder == *folder))
    {
        return Err(invalid("Undo manifest contains an invalid folder"));
    }
    let existing_metadata: BTreeSet<_> = manifest.existing_metadata.iter().collect();
    if existing_metadata.len() != manifest.existing_metadata.len()
        || (!manifest.folder_metadata && !existing_metadata.is_empty())
        || existing_metadata.iter().any(|folder| {
            !single_name(folder)
                || created.contains(folder)
                || !manifest.moves.iter().any(|item| &item.folder == *folder)
        })
    {
        return Err(invalid("Undo manifest contains invalid metadata history"));
    }
    let mut names = BTreeSet::new();
    for item in &manifest.moves {
        if !single_name(&item.name)
            || !single_name(&item.folder)
            || item.name.starts_with('.')
            || folder_for(&item.name) != item.folder
            || item.sha256.len() != 64
            || !item.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !names.insert(&item.name)
        {
            return Err(invalid("Undo manifest contains an invalid move"));
        }
        let folder = folder_path(root, &item.folder)?;
        let source = root.join(&item.name);
        let destination = folder.join(&item.name);
        let at_source = regular_file(&source)?;
        let at_destination = regular_file(&destination)?;
        if at_source && at_destination {
            return Err(invalid(format!(
                "Both source and destination exist: {source:?}, {destination:?}"
            )));
        }
        if !at_source && !at_destination {
            return Err(invalid(format!(
                "Organized file is missing: {destination:?}"
            )));
        }
        let current = if at_destination {
            &destination
        } else {
            &source
        };
        if sha256(current)? != item.sha256 {
            return Err(invalid(format!(
                "File changed since organizing: {current:?}"
            )));
        }
        if manifest.folder_metadata
            && let Some(metadata) = read_folder(&folder.join(FOLDER_FILE), &item.folder)?
            && metadata
                .files
                .get(&item.name)
                .is_some_and(|hash| hash != &item.sha256)
        {
            return Err(invalid(format!(
                "Folder metadata changed for {:?}",
                item.name
            )));
        }
    }
    for item in manifest.moves.iter().rev() {
        let folder = root.join(&item.folder);
        let source = root.join(&item.name);
        let destination = folder.join(&item.name);
        if regular_file(&destination)? {
            println!(
                "{}{} -> {}",
                if settings.dry_run { "[DRY-RUN] " } else { "" },
                destination.display(),
                source.display()
            );
            if !settings.dry_run {
                move_no_replace(&destination, &source)?;
            }
        }
        if manifest.folder_metadata && !settings.dry_run {
            let metadata_path = folder.join(FOLDER_FILE);
            if let Some(mut metadata) = read_folder(&metadata_path, &item.folder)?
                && metadata.files.get(&item.name).map(String::as_str) == Some(&item.sha256)
            {
                metadata.files.remove(&item.name);
                if metadata.files.is_empty() {
                    if existing_metadata.contains(&item.folder) {
                        write_folder(&metadata_path, &metadata)?;
                    } else {
                        fs::remove_file(&metadata_path)?;
                    }
                } else {
                    write_folder(&metadata_path, &metadata)?;
                }
            }
        }
    }
    if !settings.dry_run {
        for folder in &manifest.created_folders {
            let _ = fs::remove_dir(root.join(folder)); // Only remove an empty folder.
        }
        fs::remove_file(&path)?;
    }
    println!(
        "{}Undo complete for {} file(s)",
        if settings.dry_run { "[DRY-RUN] " } else { "" },
        manifest.moves.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestDir;

    fn manifest(root: &Path) -> PathBuf {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".arkive-organize-")
            })
            .unwrap()
    }

    #[test]
    fn organize_metadata_and_undo_round_trip() {
        let temp = TestDir::new("organize-roundtrip");
        let root = temp.path();
        fs::write(root.join("a.TXT"), b"first").unwrap();
        fs::write(root.join("b.txt"), b"second").unwrap();
        fs::write(root.join("notes"), b"plain").unwrap();
        fs::write(root.join(".hidden.txt"), b"hidden").unwrap();
        by_extension(root, true, &Settings::default()).unwrap();
        assert_eq!(fs::read(root.join("txt/a.TXT")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("txt/b.txt")).unwrap(), b"second");
        assert!(root.join("no-extension/notes").exists());
        assert!(root.join(".hidden.txt").exists());
        let metadata: FolderMetadata =
            serde_json::from_slice(&fs::read(root.join("txt/.arkive-folder.json")).unwrap())
                .unwrap();
        assert_eq!(metadata.rule, "extension:txt");
        assert_eq!(metadata.files.len(), 2);
        undo(&manifest(root), &Settings::default()).unwrap();
        assert_eq!(fs::read(root.join("a.TXT")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"second");
        assert!(!root.join("txt").exists());
        assert!(!root.join("no-extension").exists());
    }

    #[test]
    fn dry_run_and_collision_leave_files_untouched() {
        let temp = TestDir::new("organize-collision");
        let root = temp.path();
        fs::write(root.join("a.txt"), b"source").unwrap();
        fs::write(root.join("b.pdf"), b"other").unwrap();
        let settings = Settings {
            dry_run: true,
            ..Settings::default()
        };
        by_extension(root, true, &settings).unwrap();
        assert!(root.join("a.txt").exists());
        assert!(!root.join("txt").exists());
        fs::create_dir(root.join("txt")).unwrap();
        fs::write(root.join("txt/a.txt"), b"existing").unwrap();
        assert!(by_extension(root, false, &Settings::default()).is_err());
        assert!(root.join("a.txt").exists());
        assert!(root.join("b.pdf").exists());
        assert_eq!(fs::read(root.join("txt/a.txt")).unwrap(), b"existing");
    }

    #[test]
    fn undo_refuses_modified_file() {
        let temp = TestDir::new("organize-modified");
        let root = temp.path();
        fs::write(root.join("a.txt"), b"original").unwrap();
        by_extension(root, false, &Settings::default()).unwrap();
        let path = manifest(root);
        fs::write(root.join("txt/a.txt"), b"changed").unwrap();
        assert!(undo(&path, &Settings::default()).is_err());
        assert!(root.join("txt/a.txt").exists());
        assert!(path.exists());
    }

    #[test]
    fn undo_preserves_preexisting_empty_folder_metadata() {
        let temp = TestDir::new("organize-existing-metadata");
        let root = temp.path();
        let folder = root.join("txt");
        fs::create_dir(&folder).unwrap();
        let metadata_path = folder.join(FOLDER_FILE);
        let metadata = FolderMetadata {
            format: "arkive-folder".into(),
            version: 1,
            rule: "extension:txt".into(),
            files: BTreeMap::new(),
        };
        write_folder(&metadata_path, &metadata).unwrap();
        fs::write(root.join("a.txt"), b"content").unwrap();
        by_extension(root, true, &Settings::default()).unwrap();
        undo(&manifest(root), &Settings::default()).unwrap();
        let restored: FolderMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert!(restored.files.is_empty());
        assert!(folder.exists());
    }
}
