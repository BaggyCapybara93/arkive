use super::confirmed;
use super::files;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::{Action, Message, protocol, serve_stream};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::test::TestDir;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::vault;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fs;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::net::UnixStream;

#[test]
fn confirmation_requires_explicit_yes() {
    assert!(confirmed("y\n"));
    assert!(confirmed("YES"));
    assert!(!confirmed(""));
    assert!(!confirmed("yes please"));
}

#[test]
fn remote_paths_are_bounded() {
    for path in [
        "../secret",
        "/etc/passwd",
        "a/../b",
        "a//b",
        "a/./b",
        "a\\b",
        ".arkive-stage-x",
    ] {
        assert!(files::validate_relative(path, false).is_err(), "{path}");
    }
    assert!(files::validate_relative("safe/nested.txt", false).is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn configured_files_root_cannot_expose_vault_storage() {
    let temp = TestDir::new("remote-files-root");
    let vault_path = temp.path().join("vault");
    fs::create_dir(&vault_path).unwrap();
    vault::init(&vault_path, false).unwrap();
    assert!(files::prepare_root(&vault_path, Some(&vault_path)).is_err());
    assert!(files::prepare_root(&vault_path, Some(temp.path())).is_err());
    assert!(files::prepare_root(&vault_path, Some(&vault_path.join(".arkive-vault"))).is_err());
    let safe = temp.path().join("safe");
    fs::create_dir(&safe).unwrap();
    assert_eq!(
        files::prepare_root(&vault_path, Some(&safe)).unwrap(),
        fs::canonicalize(safe).unwrap()
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn file_area_round_trip_and_no_clobber() {
    let temp = TestDir::new("remote-files-roundtrip");
    let vault_path = temp.path().join("vault");
    fs::create_dir(&vault_path).unwrap();
    vault::init(&vault_path, false).unwrap();
    let root_path = files::prepare_root(&vault_path, None).unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("empty")).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/data.txt"), b"remote data").unwrap();
    let export = files::LocalExport::scan(&source).unwrap();
    let vault_id = vault::remote_identity(&vault_path).unwrap();
    let token = "a".repeat(64);

    let (mut client, mut server) = UnixStream::pair().unwrap();
    let server_path = vault_path.clone();
    let server_root = root_path.clone();
    let server_id = vault_id.clone();
    let server_token = token.clone();
    let task = std::thread::spawn(move || {
        serve_stream(
            &mut server,
            &server_path,
            &server_id,
            &server_token,
            &server_root,
        )
    });
    super::hello(&mut client, token.clone(), Action::Copy).unwrap();
    protocol::write(
        &mut client,
        &Message::Copy {
            destination: "backups/save".into(),
            manifest_size: export.bytes.len(),
        },
    )
    .unwrap();
    use std::io::Write;
    client.write_all(&export.bytes).unwrap();
    assert!(matches!(
        protocol::read(&mut client).unwrap(),
        Message::TransferReady
    ));
    assert_eq!(export.send(&mut client).unwrap(), 11);
    assert!(matches!(
        protocol::read(&mut client).unwrap(),
        Message::FileComplete { bytes: 11 }
    ));
    task.join().unwrap().unwrap();
    assert_eq!(
        fs::read(root_path.join("backups/save/nested/data.txt")).unwrap(),
        b"remote data"
    );
    assert!(root_path.join("backups/save/empty").is_dir());

    let root = super::server_files::FilesRoot::open(&root_path).unwrap();
    let listing = root.list("backups").unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "save");
    assert!(
        root.begin_upload("backups/save", export.manifest.clone())
            .is_err()
    );

    let download = root.export("backups/save").unwrap();
    let destination = temp.path().join("downloaded");
    let mut bytes = Vec::new();
    assert_eq!(download.send(&mut bytes).unwrap(), 11);
    let manifest = files::Manifest::decode(download.manifest_bytes()).unwrap();
    let staged = files::receive_to(&mut bytes.as_slice(), &destination, &manifest).unwrap();
    assert!(!destination.exists());
    assert_eq!(staged.bytes(), 11);
    staged.publish().unwrap();
    assert_eq!(
        fs::read(destination.join("nested/data.txt")).unwrap(),
        b"remote data"
    );
    assert!(destination.join("empty").is_dir());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn server_rejects_escape_and_cleans_failed_upload() {
    use std::os::unix::fs::symlink;
    let temp = TestDir::new("remote-escape");
    let root_path = temp.path().join("files");
    fs::create_dir(&root_path).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, root_path.join("link")).unwrap();
    let root = super::server_files::FilesRoot::open(&root_path).unwrap();
    assert!(root.list("link").is_err());
    assert!(root.export("link").is_err());
    let source = temp.path().join("source.txt");
    fs::write(&source, b"good").unwrap();
    let export = files::LocalExport::scan(&source).unwrap();
    assert!(
        root.begin_upload("link/escape", export.manifest.clone())
            .is_err()
    );
    assert!(!outside.join("escape").exists());
    let stage = root.begin_upload("safe.txt", export.manifest).unwrap();
    assert!(stage.receive_and_publish(&mut b"evil".as_slice()).is_err());
    assert!(!root_path.join("safe.txt").exists());
    assert_eq!(fs::read_dir(&root_path).unwrap().count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn bad_token_is_rejected_before_request() {
    let temp = TestDir::new("remote-auth");
    let vault_path = temp.path().join("vault");
    fs::create_dir(&vault_path).unwrap();
    vault::init(&vault_path, false).unwrap();
    let root = files::prepare_root(&vault_path, None).unwrap();
    let id = vault::remote_identity(&vault_path).unwrap();
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let task = std::thread::spawn(move || {
        serve_stream(&mut server, &vault_path, &id, &"a".repeat(64), &root)
    });
    assert!(super::hello(&mut client, "b".repeat(64), Action::Check).is_err());
    assert!(task.join().unwrap().is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn snapshot_push_imports_verifiable_history() {
    let temp = TestDir::new("remote-snapshot-push");
    let source_vault = temp.path().join("source-vault");
    let target_vault = temp.path().join("target-vault");
    fs::create_dir(&source_vault).unwrap();
    fs::create_dir(&target_vault).unwrap();
    vault::init(&source_vault, false).unwrap();
    vault::init(&target_vault, false).unwrap();
    let save = temp.path().join("save.txt");
    fs::write(&save, b"snapshot payload").unwrap();
    vault::snapshots::create(&source_vault, &save, None, false).unwrap();
    let snapshot_id = fs::read_dir(source_vault.join(".arkive-vault/snapshots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let export = vault::snapshots::remote_export(&source_vault, &snapshot_id).unwrap();
    let id = vault::remote_identity(&target_vault).unwrap();
    let root = files::prepare_root(&target_vault, None).unwrap();
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let token = "a".repeat(64);
    let server_token = token.clone();
    let task = std::thread::spawn(move || {
        serve_stream(&mut server, &target_vault, &id, &server_token, &root)
    });
    let (_, count, bytes) = super::push_stream(&mut client, &export, &snapshot_id, token).unwrap();
    export.release().unwrap();
    task.join().unwrap().unwrap();
    assert_eq!(count, 1);
    assert_eq!(bytes, 16);
    let restored = temp.path().join("restored.txt");
    let target_vault = temp.path().join("target-vault");
    vault::snapshots::verify(&target_vault, &snapshot_id).unwrap();
    vault::snapshots::restore(&target_vault, &snapshot_id, &restored, false).unwrap();
    assert_eq!(
        fs::read(restored.join("save.txt")).unwrap(),
        b"snapshot payload"
    );
}
