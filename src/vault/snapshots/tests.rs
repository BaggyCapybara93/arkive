use super::*;
use crate::test::TestDir;

fn setup(name: &str) -> (TestDir, PathBuf, PathBuf) {
    let temp = TestDir::new(name);
    let vault = temp.path().join("vault");
    let saves = temp.path().join("saves");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&saves).unwrap();
    crate::vault::init(&vault, false).unwrap();
    (temp, vault, saves)
}

fn count(root: &Path, area: &str) -> usize {
    fs::read_dir(root.join(CONTROL).join(area)).unwrap().count()
}

fn clean_root(root: &Path) {
    let entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [CONTROL]);
}

#[test]
fn captures_nested_binary_saves_empty_directories_and_reuses_objects() {
    let (_temp, vault, saves) = setup("snapshots-roundtrip");
    fs::create_dir_all(saves.join("nested/empty")).unwrap();
    let bytes: Vec<u8> = (0..150_000).map(|index| (index % 251) as u8).collect();
    fs::write(saves.join("slot.bin"), &bytes).unwrap();
    fs::write(saves.join("nested/duplicate.bin"), &bytes).unwrap();
    fs::write(saves.join("empty.bin"), b"").unwrap();
    let before = scan(&saves).unwrap();

    create(&vault, &saves, Some("Before boss fight".into()), false).unwrap();
    let first = history(&vault).unwrap().remove(0);
    assert_eq!(first.manifest.entries, before);
    assert_eq!(first.manifest.label.as_deref(), Some("Before boss fight"));
    assert_eq!(totals(&first.manifest).unwrap(), (3, 300_000));
    assert_eq!(count(&vault, "objects"), 2);
    let manifest_path = vault
        .join(CONTROL)
        .join("snapshots")
        .join(&first.id)
        .join("manifest.json");
    let original = fs::read(&manifest_path).unwrap();
    assert_eq!(hex(&Sha256::digest(&original)), first.id);
    let decoded: Manifest = serde_json::from_slice(&original).unwrap();
    assert_eq!(decoded.entries, before);

    create(&vault, &saves, Some("Same saves".into()), false).unwrap();
    assert_eq!(count(&vault, "objects"), 2);
    assert_eq!(count(&vault, "snapshots"), 2);
    assert_eq!(scan(&saves).unwrap(), before);

    fs::write(saves.join("slot.bin"), b"new progress").unwrap();
    create(&vault, &saves, None, false).unwrap();
    assert_eq!(count(&vault, "objects"), 3);
    assert_eq!(count(&vault, "snapshots"), 3);
    assert_eq!(fs::read(&manifest_path).unwrap(), original);
    for snapshot in history(&vault).unwrap() {
        verify(&vault, &snapshot.id).unwrap();
    }
    // Original bytes remain recoverable even after the live save changes.
    assert_eq!(
        fs::read(object_path(&vault, &hex(&Sha256::digest(&bytes))).join("data")).unwrap(),
        bytes
    );
    clean_root(&vault);
}

#[test]
fn individual_file_snapshot_uses_standard_sha256_and_empty_root_path() {
    let (_temp, vault, saves) = setup("snapshot-file");
    let source = saves.join("save.dat");
    fs::write(&source, b"abc").unwrap();
    create(&vault, &source, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    assert_eq!(snapshot.manifest.source_name, "save.dat");
    assert_eq!(
        snapshot.manifest.entries,
        [Entry::File {
            path: "".into(),
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            size: 3,
        }]
    );
    verify(&vault, &snapshot.id).unwrap();
}

#[test]
fn empty_directory_is_a_valid_snapshot() {
    let (_temp, vault, saves) = setup("snapshot-empty");
    create(&vault, &saves, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    assert_eq!(
        snapshot.manifest.entries,
        [Entry::Directory { path: "".into() }]
    );
    assert_eq!(count(&vault, "objects"), 0);
    verify(&vault, &snapshot.id).unwrap();
}

#[test]
fn restore_recreates_directory_and_file_snapshots_without_overwriting() {
    let (temp, vault, saves) = setup("snapshot-restore");
    fs::create_dir_all(saves.join("nested/empty")).unwrap();
    fs::write(saves.join("save.bin"), b"before boss").unwrap();
    create(&vault, &saves, None, false).unwrap();
    let directory_snapshot = history(&vault).unwrap().remove(0);

    let restored = temp.path().join("restored-directory");
    restore(&vault, &directory_snapshot.id, &restored, false).unwrap();
    assert_eq!(fs::read(restored.join("save.bin")).unwrap(), b"before boss");
    assert!(restored.join("nested/empty").is_dir());
    assert!(restore(&vault, &directory_snapshot.id, &restored, false).is_err());
    assert_eq!(fs::read(restored.join("save.bin")).unwrap(), b"before boss");

    let file = saves.join("single.dat");
    fs::write(&file, b"single save").unwrap();
    create(&vault, &file, None, false).unwrap();
    let file_snapshot = history(&vault)
        .unwrap()
        .into_iter()
        .find(|snapshot| snapshot.manifest.source_name == "single.dat")
        .unwrap();
    let restored_file = temp.path().join("restored-file");
    restore(&vault, &file_snapshot.id, &restored_file, false).unwrap();
    assert_eq!(
        fs::read(restored_file.join("single.dat")).unwrap(),
        b"single save"
    );
    clean_root(&vault);
}

#[test]
fn failed_restore_leaves_no_destination_or_staging_data() {
    let (temp, vault, saves) = setup("snapshot-restore-failure");
    fs::write(saves.join("save.bin"), b"save").unwrap();
    create(&vault, &saves, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    let object = object_path(&vault, &hex(&Sha256::digest(b"save"))).join("data");
    fs::write(&object, b"corrupt").unwrap();

    let destination = temp.path().join("failed-restore");
    assert!(restore(&vault, &snapshot.id, &destination, false).is_err());
    assert!(!destination.exists());
    assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".arkive-tmp-")
    }));
    clean_root(&vault);
}

#[test]
fn restore_dry_run_and_vault_targets_are_rejected_without_writes() {
    let (temp, vault, saves) = setup("snapshot-restore-dry-run");
    fs::write(saves.join("save.bin"), b"save").unwrap();
    create(&vault, &saves, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    let destination = temp.path().join("dry-run-restore");

    restore(&vault, &snapshot.id, &destination, true).unwrap();
    assert!(!destination.exists());
    assert!(restore(&vault, &snapshot.id, &vault.join("inside"), false).is_err());
    assert!(restore(&vault, &snapshot.id, temp.path(), false).is_err());
    clean_root(&vault);
}

#[test]
fn dry_run_does_not_write_and_still_detects_corrupt_reused_objects() {
    let (_temp, vault, saves) = setup("snapshot-dry");
    fs::write(saves.join("save.bin"), b"save").unwrap();
    create(&vault, &saves, None, true).unwrap();
    assert_eq!(count(&vault, "snapshots"), 0);
    assert_eq!(count(&vault, "objects"), 0);
    clean_root(&vault);

    create(&vault, &saves, None, false).unwrap();
    let object = object_path(&vault, &hex(&Sha256::digest(b"save"))).join("data");
    fs::write(&object, b"corrupt").unwrap();
    assert!(create(&vault, &saves, None, true).is_err());
    assert_eq!(fs::read(object).unwrap(), b"corrupt");
    assert_eq!(count(&vault, "snapshots"), 1);
    clean_root(&vault);
}

#[test]
fn corrupt_object_aborts_capture_without_overwriting_it_or_publishing_history() {
    let (_temp, vault, saves) = setup("snapshot-corrupt");
    fs::write(saves.join("z-save.bin"), b"old").unwrap();
    create(&vault, &saves, None, false).unwrap();
    let previous = history(&vault).unwrap().remove(0);
    let object = object_path(&vault, &hex(&Sha256::digest(b"old"))).join("data");
    fs::write(&object, b"bad").unwrap();
    // This object will be stored before we encounter the corrupt reused object.
    fs::write(saves.join("a-new.bin"), b"new").unwrap();
    assert!(create(&vault, &saves, None, false).is_err());
    assert_eq!(fs::read(&object).unwrap(), b"bad");
    assert_eq!(count(&vault, "snapshots"), 1);
    assert_eq!(count(&vault, "objects"), 2); // Safe orphan, reusable by a later capture.
    assert!(verify(&vault, &previous.id).is_err());
    assert!(history(&vault).is_ok()); // Listing checks manifests only.
    clean_root(&vault);
    fs::remove_file(&object).unwrap();
    assert!(verify(&vault, &previous.id).is_err());
}

#[test]
fn tampered_manifest_is_rejected_by_listing_and_verification() {
    let (_temp, vault, saves) = setup("snapshot-manifest-tamper");
    create(&vault, &saves, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    let path = vault
        .join(CONTROL)
        .join("snapshots")
        .join(&snapshot.id)
        .join("manifest.json");
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' '); // Still JSON, but no longer the committed bytes.
    fs::write(&path, &bytes).unwrap();
    assert!(history(&vault).is_err());
    assert!(verify(&vault, &snapshot.id).is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn reader_rejects_unsupported_versions_and_unsafe_paths_even_with_matching_hash() {
    let (_temp, vault, saves) = setup("snapshot-invalid-manifests");
    create(&vault, &saves, None, false).unwrap();
    let original = history(&vault).unwrap().remove(0);
    let value = serde_json::to_value(&original.manifest).unwrap();
    let mut variants = Vec::new();
    let mut future = value.clone();
    future["version"] = serde_json::json!(2);
    variants.push(future);
    for path in [
        "../outside",
        "/absolute",
        "C:/save",
        "nested\\save",
        "a//b",
        "missing/child",
    ] {
        let mut bad = value.clone();
        bad["entries"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"type": "directory", "path": path}));
        variants.push(bad);
    }
    let mut duplicate = value.clone();
    duplicate["entries"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"type": "directory", "path": ""}));
    variants.push(duplicate);
    let mut bad_hash = value;
    bad_hash["entries"].as_array_mut().unwrap().push(
        serde_json::json!({"type": "file", "path": "save", "sha256": "../../outside", "size": 1}),
    );
    variants.push(bad_hash);
    for bad in variants {
        let bytes = serde_json::to_vec(&bad).unwrap();
        let id = hex(&Sha256::digest(&bytes));
        let directory = vault.join(CONTROL).join("snapshots").join(&id);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("manifest.json"), &bytes).unwrap();
        assert!(load(&vault, &id).is_err());
    }
    for id in ["../outside", "", "abc", &"A".repeat(64)] {
        assert!(verify(&vault, id).is_err());
    }
}

#[test]
fn source_changes_are_detected_including_added_and_removed_empty_directories() {
    let (_temp, _vault, saves) = setup("snapshot-source-change");
    fs::write(saves.join("save"), b"original").unwrap();
    let expected = scan(&saves).unwrap();
    fs::write(saves.join("save"), b"modified").unwrap();
    assert!(ensure_unchanged(&saves, &expected).is_err());
    fs::write(saves.join("save"), b"original").unwrap();
    fs::create_dir(saves.join("new-empty")).unwrap();
    assert!(ensure_unchanged(&saves, &expected).is_err());
    let with_directory = scan(&saves).unwrap();
    fs::remove_dir(saves.join("new-empty")).unwrap();
    assert!(ensure_unchanged(&saves, &with_directory).is_err());
    fs::remove_file(saves.join("save")).unwrap();
    assert!(ensure_unchanged(&saves, &expected).is_err());
}

#[test]
fn interrupted_object_capture_is_cleaned_and_never_published() {
    let (_temp, vault, saves) = setup("snapshot-changed-capture");
    let source = saves.join("save");
    fs::write(&source, b"before").unwrap();
    let (hash, size) = hash_file(&source, None).unwrap();
    fs::write(&source, b"after").unwrap();
    let result = crate::vault::with_scratch(&vault, |scratch| {
        store_object(&vault, scratch, &source, &hash, size)
    });
    assert!(result.is_err());
    assert_eq!(count(&vault, "objects"), 0);
    assert_eq!(count(&vault, "snapshots"), 0);
    clean_root(&vault);
}

#[test]
fn initialized_vault_nonoverlapping_source_and_available_lock_are_required() {
    let (temp, vault, saves) = setup("snapshot-preconditions");
    for dry in [false, true] {
        assert!(create(&saves, &vault, None, dry).is_err());
        assert!(create(&vault, temp.path(), None, dry).is_err());
        assert!(create(&vault, &vault, None, dry).is_err());
        assert!(create(&vault, &vault.join(CONTROL), None, dry).is_err());
        assert!(create(&vault, &saves.join("missing"), None, dry).is_err());
        assert!(create(&vault, &saves, Some("x".repeat(257)), dry).is_err());
    }
    let lock = VaultLock::acquire(&vault).unwrap();
    assert!(create(&vault, &saves, None, false).is_err());
    assert!(create(&vault, &saves, None, true).is_err());
    lock.release().unwrap();
    assert_eq!(count(&vault, "snapshots"), 0);
    assert_eq!(count(&vault, "objects"), 0);
    clean_root(&vault);
}

#[test]
fn publication_does_not_replace_existing_data_even_an_empty_directory() {
    let temp = TestDir::new("snapshot-no-clobber");
    let staged = temp.path().join("staged");
    let existing = temp.path().join("existing");
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join("data"), b"new").unwrap();
    fs::create_dir(&existing).unwrap();
    assert!(publish(&staged, &existing).is_err());
    fs::write(existing.join("data"), b"keep").unwrap();
    assert!(publish(&staged, &existing).is_err());
    assert_eq!(fs::read(existing.join("data")).unwrap(), b"keep");
    assert_eq!(fs::read(staged.join("data")).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn symlinked_sources_objects_and_manifests_are_rejected() {
    use std::os::unix::fs::symlink;
    let (temp, vault, saves) = setup("snapshot-links");
    let outside = temp.path().join("outside");
    fs::write(&outside, b"private").unwrap();
    let link = saves.join("link");
    for target in [&outside, &temp.path().join("missing"), &saves] {
        symlink(target, &link).unwrap();
        assert!(create(&vault, &saves, None, false).is_err());
        assert!(create(&vault, &link, None, false).is_err());
        fs::remove_file(&link).unwrap();
    }
    let alias = temp.path().join("alias");
    symlink(&saves, &alias).unwrap();
    assert!(create(&vault, &alias.join(""), None, false).is_err());
    fs::write(saves.join("save"), b"private").unwrap();
    create(&vault, &saves, None, false).unwrap();
    let snapshot = history(&vault).unwrap().remove(0);
    let object = object_path(&vault, &hex(&Sha256::digest(b"private")));
    fs::remove_file(object.join("data")).unwrap();
    symlink(&outside, object.join("data")).unwrap();
    assert!(verify(&vault, &snapshot.id).is_err());
    assert!(create(&vault, &saves, None, false).is_err());
    fs::remove_file(object.join("data")).unwrap();
    fs::remove_dir(&object).unwrap();
    symlink(&saves, &object).unwrap();
    assert!(verify(&vault, &snapshot.id).is_err());

    let manifest = vault
        .join(CONTROL)
        .join("snapshots")
        .join(&snapshot.id)
        .join("manifest.json");
    let saved_manifest = temp.path().join("manifest.json");
    fs::rename(&manifest, &saved_manifest).unwrap();
    symlink(&saved_manifest, &manifest).unwrap();
    assert!(history(&vault).is_err());
    assert!(verify(&vault, &snapshot.id).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"private");
    clean_root(&vault);
}

// APFS rejects invalid UTF-8 names before Arkive can inspect them.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_are_rejected_without_snapshot() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let (_temp, vault, saves) = setup("snapshot-non-utf8");
    let path = saves.join(OsString::from_vec(vec![0xff]));
    fs::write(&path, b"bytes").unwrap();
    assert!(create(&vault, &saves, None, false).is_err());
    assert_eq!(count(&vault, "snapshots"), 0);
    clean_root(&vault);
}

#[cfg(unix)]
#[test]
fn special_files_are_rejected_without_snapshot() {
    use std::os::unix::net::UnixListener;
    // Keep the path within macOS's Unix socket path limit.
    let (_temp, vault, saves) = setup("snap-socket");
    let _socket = match UnixListener::bind(saves.join("socket")) {
        Ok(socket) => socket,
        // Some CI sandboxes prohibit Unix-domain sockets entirely. That is a
        // runner capability limit, not a meaningful snapshot test failure.
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("create Unix socket fixture: {error}"),
    };
    assert!(create(&vault, &saves, None, false).is_err());
    assert_eq!(count(&vault, "snapshots"), 0);
    clean_root(&vault);
}
