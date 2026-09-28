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

#[test]
fn profile_round_trip_and_removal_preserve_saves_and_snapshots() {
    let (_temp, vault, saves) = setup("profiles-roundtrip");
    fs::write(saves.join("save.dat"), b"progress").unwrap();

    add(&vault, "my-game", &saves, false).unwrap();
    let root = vault.canonicalize().unwrap();
    let stored = all(&root).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "my-game");
    assert_eq!(stored[0].source, saves.canonicalize().unwrap());
    assert_eq!(
        source(&root, "my-game").unwrap(),
        saves.canonicalize().unwrap()
    );

    crate::vault::snapshots::create(&vault, &saves, None, false).unwrap();
    remove(&vault, "my-game", false).unwrap();
    assert!(all(&root).unwrap().is_empty());
    assert_eq!(
        fs::read_dir(root.join(CONTROL).join("snapshots"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(fs::read(saves.join("save.dat")).unwrap(), b"progress");
}

#[test]
fn profile_add_is_atomic_and_dry_run_does_not_write() {
    let (_temp, vault, saves) = setup("profiles-dry-run");
    assert!(add(&vault, "my-game", &saves, true).is_ok());
    assert!(all(&vault.canonicalize().unwrap()).unwrap().is_empty());

    add(&vault, "my-game", &saves, false).unwrap();
    assert!(add(&vault, "my-game", &saves, false).is_err());
    assert!(add(&vault, "bad/name", &saves, false).is_err());
    assert_eq!(all(&vault.canonicalize().unwrap()).unwrap().len(), 1);
}

#[test]
fn profiles_reject_vault_and_unavailable_sources() {
    let (_temp, vault, saves) = setup("profiles-preconditions");
    assert!(add(&vault, "inside", &vault, false).is_err());
    assert!(add(&vault, "missing", &saves.join("missing"), false).is_err());
    assert!(source(&vault.canonicalize().unwrap(), "missing").is_err());
}

#[cfg(unix)]
#[test]
fn profiles_reject_symlink_sources() {
    use std::os::unix::fs::symlink;

    let (_temp, vault, saves) = setup("profiles-links");
    let target = saves.join("real");
    let link = saves.join("link");
    fs::write(&target, b"save").unwrap();
    symlink(&target, &link).unwrap();
    assert!(add(&vault, "linked", &link, false).is_err());
}

#[test]
fn shared_profile_edit_cannot_redirect_snapshot_source() {
    let (_temp, vault, saves) = setup("profiles-redirection");
    let private = vault.parent().unwrap().join("private-save");
    fs::write(saves.join("save.dat"), b"approved").unwrap();
    fs::write(&private, b"private").unwrap();
    add(&vault, "game", &saves, false).unwrap();

    let root = vault.canonicalize().unwrap();
    let profile_file = profile_path(&root, "game").unwrap();
    let original = fs::read(&profile_file).unwrap();
    let mut profile: Profile = serde_json::from_slice(&original).unwrap();
    profile.source = private.canonicalize().unwrap();
    fs::write(&profile_file, serde_json::to_vec(&profile).unwrap()).unwrap();
    assert!(
        crate::vault::snapshots::create_selected(&vault, None, Some("game"), None, false, false)
            .is_err()
    );
    assert_eq!(
        fs::read_dir(root.join(CONTROL).join("objects"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(root.join(CONTROL).join("snapshots"))
            .unwrap()
            .count(),
        0
    );

    fs::write(&profile_file, original).unwrap();
    crate::vault::snapshots::create_selected(&vault, None, Some("game"), None, false, false)
        .unwrap();
    assert_eq!(
        fs::read_dir(root.join(CONTROL).join("snapshots"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn profile_from_shared_vault_needs_explicit_local_approval() {
    let (_temp, vault, saves) = setup("profiles-local-approval");
    fs::write(saves.join("save.dat"), b"approved").unwrap();
    let root = vault.canonicalize().unwrap();
    let profile = Profile {
        format: FORMAT.into(),
        version: VERSION,
        name: "game".into(),
        source: saves.canonicalize().unwrap(),
        created_at: Utc::now(),
    };
    fs::write(
        profile_path(&root, "game").unwrap(),
        serde_json::to_vec(&profile).unwrap(),
    )
    .unwrap();
    assert!(
        crate::vault::snapshots::create_selected(&vault, None, Some("game"), None, false, false)
            .is_err()
    );
    add(&vault, "game", &saves, false).unwrap();
    crate::vault::snapshots::create_selected(&vault, None, Some("game"), None, false, false)
        .unwrap();
}

#[test]
fn unavailable_local_binding_does_not_publish_profile() {
    let (_temp, vault, saves) = setup("profiles-binding-failure");
    let home = vault.parent().unwrap().join("test-home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".arkive-profile-bindings"), b"blocked").unwrap();
    assert!(add(&vault, "game", &saves, false).is_err());
    assert!(all(&vault.canonicalize().unwrap()).unwrap().is_empty());
}

#[test]
fn stale_binding_can_be_replaced_after_remote_profile_removal() {
    let (_temp, vault, saves) = setup("profiles-stale-binding");
    let replacement = vault.parent().unwrap().join("replacement");
    fs::create_dir(&replacement).unwrap();
    add(&vault, "game", &saves, false).unwrap();
    let root = vault.canonicalize().unwrap();
    fs::remove_file(profile_path(&root, "game").unwrap()).unwrap();

    add(&vault, "game", &replacement, false).unwrap();
    assert_eq!(
        source(&root, "game").unwrap(),
        replacement.canonicalize().unwrap()
    );
}

#[test]
fn unavailable_binding_does_not_remove_shared_profile() {
    let (_temp, vault, saves) = setup("profiles-remove-binding-failure");
    add(&vault, "game", &saves, false).unwrap();
    let home = vault.parent().unwrap().join("test-home");
    fs::rename(&home, vault.parent().unwrap().join("old-test-home")).unwrap();
    fs::write(&home, b"blocked").unwrap();

    assert!(remove(&vault, "game", false).is_err());
    assert!(
        profile_path(&vault.canonicalize().unwrap(), "game")
            .unwrap()
            .exists()
    );
}
