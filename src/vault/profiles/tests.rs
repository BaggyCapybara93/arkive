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
