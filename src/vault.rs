//! Filesystem vault foundation. Network mounts and credentials belong to the OS.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::file_module::ops::create_temp_dir_sibling;

const CONTROL: &str = ".arkive-vault";
const LOCK: &str = ".arkive-vault.lock";
const MANIFEST: &str = "vault.json";
const FORMAT: &str = "arkive-vault";
const VERSION: u32 = 1;
const DIRECTORIES: [&str; 3] = ["objects", "snapshots", "profiles"];

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    created_at: DateTime<Utc>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidInput(message.into())
}

fn io_at<T>(result: std::io::Result<T>, action: &str, path: &Path) -> Result<T, AppError> {
    result.map_err(|error| {
        AppError::FileError(std::io::Error::new(
            error.kind(),
            format!("Could not {action} {path:?}: {error}"),
        ))
    })
}

fn exists(path: &Path) -> Result<bool, AppError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => io_at(Err(error), "inspect", path),
    }
}

fn require_directory(path: &Path) -> Result<(), AppError> {
    let metadata = io_at(fs::symlink_metadata(path), "inspect directory", path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Expected a directory, not a symlink or another file type: {path:?}"
        )));
    }
    Ok(())
}

fn root(path: &Path) -> Result<PathBuf, AppError> {
    // Never create a missing mount path: that could silently write to local storage.
    // Strip trailing separators and `.` so `link/` cannot bypass lstat checks.
    let path: PathBuf = path.components().collect();
    require_directory(&path)?;
    io_at(fs::canonicalize(&path), "resolve vault directory", &path)
}

/// Validate only the versioned structure; this is not an object integrity audit.
fn validate(root: &Path) -> Result<bool, AppError> {
    let control = root.join(CONTROL);
    if !exists(&control)? {
        return Ok(false);
    }
    require_directory(&control)?;
    let path = control.join(MANIFEST);
    let metadata = io_at(fs::symlink_metadata(&path), "inspect vault manifest", &path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Vault manifest must be a regular file: {path:?}"
        )));
    }
    // Bound reads of a manifest supplied by another machine.
    let mut bytes = Vec::new();
    let file = io_at(File::open(&path), "open vault manifest", &path)?;
    io_at(
        file.take(65_537).read_to_end(&mut bytes),
        "read vault manifest",
        &path,
    )?;
    if bytes.len() > 65_536 {
        return Err(invalid("Vault manifest exceeds 64 KiB"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Invalid vault manifest {path:?}: {error}")))?;
    if manifest.format != FORMAT || manifest.version != VERSION {
        return Err(invalid(format!(
            "Unsupported vault format/version: {:?}/{}; expected {FORMAT}/{VERSION}",
            manifest.format, manifest.version
        )));
    }
    for name in DIRECTORIES {
        require_directory(&control.join(name))?;
    }
    Ok(true)
}

/// A directory reservation coordinates cooperating processes on a shared filesystem.
/// Never break a lock automatically: an apparently stale client may still be alive.
struct VaultLock(Option<PathBuf>);

impl VaultLock {
    fn acquire(root: &Path) -> Result<Self, AppError> {
        let path = root.join(LOCK);
        match fs::create_dir(&path) {
            Ok(()) => Ok(Self(Some(path))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(lock_error(&path))
            }
            Err(error) => io_at(Err(error), "acquire vault lock", &path),
        }
    }

    fn release(mut self) -> Result<(), AppError> {
        let path = self.0.take().expect("lock has not been released");
        io_at(fs::remove_dir(&path), "release vault lock", &path)
    }
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        // Best effort for unwinding. Normal exits report cleanup failures explicitly.
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir(path);
        }
    }
}

fn lock_error(path: &Path) -> AppError {
    invalid(format!(
        "Vault is locked at {path:?}. If a previous operation was interrupted, \
         remove this lock directory manually only after confirming all clients have stopped."
    ))
}

fn ensure_unlocked(root: &Path) -> Result<(), AppError> {
    let path = root.join(LOCK);
    if exists(&path)? {
        return Err(lock_error(&path));
    }
    Ok(())
}

fn require_empty(root: &Path) -> Result<(), AppError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name() != LOCK {
            return Err(invalid(format!(
                "Vault initialization needs an empty directory; found {:?}. \
                 Use a dedicated directory. Interrupted .arkive-tmp-* directories \
                 must be inspected and removed manually after all clients have stopped.",
                entry.path()
            )));
        }
    }
    Ok(())
}

fn combine(result: Result<(), AppError>, cleanup: Result<(), AppError>) -> Result<(), AppError> {
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(invalid(format!("{error}; cleanup also failed: {cleanup}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Remove only scratch space reserved by this invocation, including on failure.
fn with_scratch(
    root: &Path,
    operation: impl FnOnce(&Path) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let scratch = create_temp_dir_sibling(&root.join("vault"))?;
    let result = operation(&scratch);
    let cleanup = match fs::remove_dir_all(&scratch) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => io_at(Err(error), "remove vault scratch directory", &scratch),
    };
    combine(result, cleanup)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = io_at(
        OpenOptions::new().write(true).create_new(true).open(path),
        "create file without overwriting",
        path,
    )?;
    io_at(file.write_all(bytes), "write file", path)?;
    io_at(file.sync_all(), "flush file", path)?;
    drop(file);
    verify_bytes(path, bytes)
}

fn verify_bytes(path: &Path, expected: &[u8]) -> Result<(), AppError> {
    let file = io_at(File::open(path), "open verification file", path)?;
    let mut actual = Vec::new();
    io_at(
        file.take(expected.len() as u64 + 1)
            .read_to_end(&mut actual),
        "read verification file",
        path,
    )?;
    if Sha256::digest(&actual) != Sha256::digest(expected) {
        return Err(invalid(format!("SHA-256 verification failed for {path:?}")));
    }
    Ok(())
}

pub fn init(path: &Path, dry_run: bool) -> Result<(), AppError> {
    let root = root(path)?;
    if dry_run {
        ensure_unlocked(&root)?;
        if validate(&root)? {
            println!("[DRY-RUN] Vault already initialized at {root:?} (version {VERSION})");
        } else {
            require_empty(&root)?;
            println!("[DRY-RUN] Would initialize vault at {root:?} (version {VERSION})");
        }
        return Ok(());
    }
    let lock = VaultLock::acquire(&root)?;
    let result = (|| {
        if validate(&root)? {
            return Ok(());
        }
        require_empty(&root)?;
        with_scratch(&root, |scratch| {
            for name in DIRECTORIES {
                let path = scratch.join(name);
                io_at(fs::create_dir(&path), "create vault directory", &path)?;
            }
            let manifest = Manifest {
                format: FORMAT.into(),
                version: VERSION,
                created_at: Utc::now(),
            };
            let bytes = serde_json::to_vec_pretty(&manifest)
                .map_err(|error| invalid(format!("Could not encode vault manifest: {error}")))?;
            write_new(&scratch.join(MANIFEST), &bytes)?;
            let destination = root.join(CONTROL);
            // All cooperating writers hold the same filesystem lock. Refuse to
            // overwrite an entry that appeared since validation.
            if exists(&destination)? {
                return Err(invalid(format!("Vault already exists at {destination:?}")));
            }
            io_at(
                fs::rename(scratch, &destination),
                "publish staged vault",
                &destination,
            )?;
            validate(&root)?;
            Ok(())
        })
    })();
    combine(result, lock.release())?;
    println!("Vault ready at {root:?} (version {VERSION})");
    Ok(())
}

pub fn check(path: &Path, dry_run: bool) -> Result<(), AppError> {
    let root = root(path)?;
    if dry_run {
        ensure_unlocked(&root)?;
        validate(&root)?;
        println!(
            "[DRY-RUN] Would test read, write, flush, rename, delete, and SHA-256 \
             verification at {root:?}; filesystem behavior has not been tested"
        );
        return Ok(());
    }
    let lock = VaultLock::acquire(&root)?;
    let result = (|| {
        validate(&root)?;
        with_scratch(&root, probe)
    })();
    combine(result, lock.release())?;
    println!(
        "Filesystem check passed at {root:?}: read, write, flush, rename, delete, SHA-256.\n\
         This checks this client's filesystem operations only; it does not verify \
         that the path is mounted, cross-client locking, or crash durability."
    );
    Ok(())
}

fn probe(scratch: &Path) -> Result<(), AppError> {
    let directory = scratch.join("probe-dir");
    io_at(
        fs::create_dir(&directory),
        "create probe directory",
        &directory,
    )?;
    let source = directory.join("probe.bin");
    let destination = directory.join("renamed.bin");
    let bytes: Vec<u8> = (0..65_536).map(|index| (index % 251) as u8).collect();
    write_new(&source, &bytes)?;
    // Exercise the no-clobber primitive future object writes will rely on.
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source)
    {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return io_at(Err(error), "test exclusive creation", &source),
        Ok(_) => {
            return Err(invalid(
                "Filesystem allowed exclusive creation over an existing file",
            ));
        }
    }
    io_at(
        fs::rename(&source, &destination),
        "rename probe file",
        &destination,
    )?;
    if exists(&source)? {
        return Err(invalid("Probe source still exists after rename"));
    }
    verify_bytes(&destination, &bytes)?;
    // Vault initialization also relies on publishing a complete directory.
    let renamed_directory = scratch.join("renamed-dir");
    io_at(
        fs::rename(&directory, &renamed_directory),
        "rename probe directory",
        &renamed_directory,
    )?;
    if exists(&directory)? {
        return Err(invalid("Probe directory still exists after rename"));
    }
    let destination = renamed_directory.join("renamed.bin");
    verify_bytes(&destination, &bytes)?;
    io_at(
        fs::remove_file(&destination),
        "delete probe file",
        &destination,
    )?;
    if exists(&destination)? {
        return Err(invalid("Probe still exists after deletion"));
    }
    io_at(
        fs::remove_dir(&renamed_directory),
        "delete probe directory",
        &renamed_directory,
    )?;
    if exists(&renamed_directory)? {
        return Err(invalid("Probe directory still exists after deletion"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestDir;

    fn entries(root: &Path) -> Vec<std::ffi::OsString> {
        let mut entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn initialization_round_trip_is_idempotent_and_preserves_objects() {
        let temp = TestDir::new("vault-init");
        init(temp.path(), false).unwrap();
        assert!(validate(temp.path()).unwrap());
        let manifest_path = temp.path().join(CONTROL).join(MANIFEST);
        let original = fs::read(&manifest_path).unwrap();
        let manifest: Manifest = serde_json::from_slice(&original).unwrap();
        assert_eq!(manifest.format, FORMAT);
        assert_eq!(manifest.version, VERSION);
        let object = temp.path().join(CONTROL).join("objects/save");
        fs::write(&object, b"saved game").unwrap();

        init(temp.path(), false).unwrap();
        check(temp.path(), false).unwrap();

        assert_eq!(fs::read(manifest_path).unwrap(), original);
        assert_eq!(fs::read(object).unwrap(), b"saved game");
        assert_eq!(entries(temp.path()), [CONTROL]);
    }

    #[test]
    fn check_before_init_preserves_existing_files_and_cleans_up() {
        let temp = TestDir::new("vault-check");
        let existing = temp.path().join("save.bin");
        fs::write(&existing, b"precious data").unwrap();
        check(temp.path(), false).unwrap();
        assert_eq!(entries(temp.path()), ["save.bin"]);
        assert_eq!(fs::read(existing).unwrap(), b"precious data");
    }

    #[test]
    fn missing_roots_are_not_created() {
        let temp = TestDir::new("vault-missing");
        let missing = temp.path().join("offline-share/vault");
        for dry_run in [false, true] {
            assert!(init(&missing, dry_run).is_err());
            assert!(check(&missing, dry_run).is_err());
        }
        assert!(entries(temp.path()).is_empty());
    }

    #[test]
    fn initialization_refuses_nonempty_directories_without_changes() {
        let temp = TestDir::new("vault-nonempty");
        fs::write(temp.path().join("save.bin"), b"keep").unwrap();
        for dry_run in [false, true] {
            assert!(init(temp.path(), dry_run).is_err());
        }
        assert_eq!(entries(temp.path()), ["save.bin"]);
        assert_eq!(fs::read(temp.path().join("save.bin")).unwrap(), b"keep");
    }

    #[test]
    fn dry_runs_leave_empty_and_initialized_roots_unchanged() {
        let temp = TestDir::new("vault-dry-run");
        init(temp.path(), true).unwrap();
        check(temp.path(), true).unwrap();
        assert!(entries(temp.path()).is_empty());

        init(temp.path(), false).unwrap();
        let manifest = temp.path().join(CONTROL).join(MANIFEST);
        let before = fs::read(&manifest).unwrap();
        init(temp.path(), true).unwrap();
        check(temp.path(), true).unwrap();
        assert_eq!(fs::read(manifest).unwrap(), before);
        assert_eq!(entries(temp.path()), [CONTROL]);
    }

    #[test]
    fn held_lock_blocks_other_handles_and_is_never_broken() {
        let temp = TestDir::new("vault-lock");
        let lock = VaultLock::acquire(temp.path()).unwrap();
        assert!(VaultLock::acquire(temp.path()).is_err());
        for dry_run in [false, true] {
            assert!(init(temp.path(), dry_run).is_err());
            assert!(check(temp.path(), dry_run).is_err());
        }
        assert_eq!(entries(temp.path()), [LOCK]);
        lock.release().unwrap();
        init(temp.path(), false).unwrap();
    }

    #[test]
    fn unsupported_or_corrupt_manifests_are_not_rewritten() {
        let temp = TestDir::new("vault-version");
        init(temp.path(), false).unwrap();
        let path = temp.path().join(CONTROL).join(MANIFEST);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["version"] = serde_json::json!(2);
        let future = serde_json::to_vec(&manifest).unwrap();
        manifest["version"] = serde_json::json!(1);
        manifest["format"] = serde_json::json!("something-else");
        let foreign = serde_json::to_vec(&manifest).unwrap();
        for bytes in [future, foreign, b"{broken".to_vec(), vec![b' '; 65_537]] {
            fs::write(&path, &bytes).unwrap();
            for dry_run in [false, true] {
                assert!(init(temp.path(), dry_run).is_err());
                assert!(check(temp.path(), dry_run).is_err());
            }
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(entries(temp.path()), [CONTROL]);
        }
    }

    #[test]
    fn incomplete_layout_is_reported_without_silent_repair() {
        let temp = TestDir::new("vault-incomplete");
        init(temp.path(), false).unwrap();
        let objects = temp.path().join(CONTROL).join("objects");
        fs::remove_dir(&objects).unwrap();
        assert!(init(temp.path(), false).is_err());
        assert!(check(temp.path(), false).is_err());
        assert!(!objects.exists());
        assert_eq!(entries(temp.path()), [CONTROL]);
    }

    #[test]
    fn failed_scratch_operation_cleans_up_its_files_only() {
        let temp = TestDir::new("vault-cleanup");
        fs::write(temp.path().join("keep"), b"keep").unwrap();
        let result = with_scratch(temp.path(), |scratch| {
            write_new(&scratch.join("partial"), b"partial")?;
            Err(invalid("simulated operation failure"))
        });
        assert!(result.is_err());
        assert_eq!(entries(temp.path()), ["keep"]);
    }

    #[test]
    fn no_clobber_and_hash_verification_detect_bad_writes() {
        let temp = TestDir::new("vault-verification");
        let path = temp.path().join("probe");
        write_new(&path, b"expected").unwrap();
        assert!(write_new(&path, b"replacement").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"expected");
        fs::write(&path, b"corrupt!").unwrap();
        assert!(verify_bytes(&path, b"expected").is_err());
        fs::write(&path, b"expected plus trailing data").unwrap();
        assert!(verify_bytes(&path, b"expected").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_roots_locks_and_layout_entries_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = TestDir::new("vault-symlink");
        let real = temp.path().join("real");
        let link = temp.path().join("link");
        fs::create_dir(&real).unwrap();
        symlink(&real, &link).unwrap();
        assert!(init(&link, false).is_err());
        assert!(check(&link, false).is_err());
        for suffix in ["/", "/."] {
            let trailing = PathBuf::from(format!("{}{suffix}", link.display()));
            assert!(init(&trailing, false).is_err());
            assert!(check(&trailing, false).is_err());
        }

        let lock = real.join(LOCK);
        symlink(temp.path().join("missing"), &lock).unwrap();
        assert!(init(&real, false).is_err());
        assert!(check(&real, false).is_err());
        fs::remove_file(lock).unwrap();

        let control = real.join(CONTROL);
        symlink(temp.path().join("missing"), &control).unwrap();
        assert!(init(&real, false).is_err());
        assert!(check(&real, false).is_err());
        fs::remove_file(&control).unwrap();
        init(&real, false).unwrap();

        let objects = control.join("objects");
        fs::remove_dir(&objects).unwrap();
        symlink(temp.path(), &objects).unwrap();
        assert!(check(&real, false).is_err());
        fs::remove_file(objects).unwrap();
        fs::create_dir(control.join("objects")).unwrap();

        let manifest = control.join(MANIFEST);
        let external = temp.path().join("manifest.json");
        fs::rename(&manifest, &external).unwrap();
        symlink(&external, &manifest).unwrap();
        assert!(init(&real, false).is_err());
        assert!(check(&real, false).is_err());
        assert!(external.is_file());
    }
}
