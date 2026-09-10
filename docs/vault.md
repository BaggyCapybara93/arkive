# Vaults and save snapshots

Arkive vaults are versioned directories intended for local-first game-save
storage. An SMB or NFS share should be mounted by the operating system first;
Arkive treats the mounted path as an ordinary filesystem directory and does
not handle network credentials.

## Initialize a vault

Use an existing, dedicated empty directory. Arkive does not create a missing
path during initialization, which helps avoid writing to local storage when a
mount is unavailable.

```bash
arkive vault check /mnt/saves/arkive
arkive --dry-run vault init /mnt/saves/arkive
arkive vault init /mnt/saves/arkive
arkive vault check /mnt/saves/arkive
```

Version 1 stores this layout:

```text
.arkive-vault/
  vault.json
  objects/       # <file-sha256>/data
  snapshots/     # <manifest-sha256>/manifest.json
  profiles/      # reserved for future game profiles
```

Initialization is idempotent: repeating it validates the existing vault
without rewriting it. Unknown versions, malformed manifests, incomplete
layouts, and symlinked roots or layout entries are rejected.

## Check storage

`vault check` exercises read, write, flush, hash verification, rename,
exclusive file creation, deletion, and cleanup using temporary probe data. In
an initialized vault it also checks the manifest and required directories.

`--dry-run` validates paths and structure without writing probe data. A passing
check covers this client's filesystem operations; it does not prove mount
identity, cross-machine cache consistency, crash durability, or server health.
It also does not hash every stored save object.

## Vault locking and recovery

Initialization, checks, snapshots, and restores reserve
`.arkive-vault.lock/` in the vault root. An occupied lock causes an error;
Arkive never breaks it automatically. After an interruption, confirm that all
clients have stopped before removing a leftover lock manually. Inspect any
`.arkive-tmp-*-vault/` staging directories before removing them.

These locks coordinate cooperating Arkive processes through the filesystem.
They are not a general distributed lock service and cannot guarantee behavior
against clients that ignore the lock.

## Capture snapshots

Close the game and other save writers before capturing. Snapshots include
regular files, empty directories, file sizes, SHA-256 hashes, the source name,
creation time, and an optional label.

```bash
arkive vault snapshot /mnt/saves/arkive ./my-game-saves --label "Before boss fight"
arkive vault snapshots /mnt/saves/arkive
arkive vault snapshots /mnt/saves/arkive --json
arkive --dry-run vault snapshot /mnt/saves/arkive ./my-game-saves
```

Snapshots are immutable and content-addressed. Identical files are stored once
and reused by later snapshots. A failed capture can leave verified,
unreferenced objects for later reuse; automatic pruning is not implemented.
Keep the vault together because manifests depend on their objects.

The source is scanned again before publication. Changes detected during the
capture abort it, but this is not an atomic filesystem snapshot. Sources and
vaults must not overlap. Symlinks and special files are rejected, as are
unsupported path names. Arkive does not capture permissions, ownership,
timestamps, ACLs, or extended attributes. Snapshots currently contain
unencrypted contents and labels.

## Verify and list snapshots

`SNAPSHOT_ID` is the full 64-character identifier printed after creation or
shown in history.

```bash
arkive vault snapshots /mnt/saves/arkive
arkive vault snapshots /mnt/saves/arkive --json
arkive vault verify-snapshot /mnt/saves/arkive SNAPSHOT_ID
```

Listing checks manifest structure and hashes. `verify-snapshot` additionally
reads every referenced object and verifies its SHA-256 and size. Both commands
are read-only.

## Compare a live source

`vault diff` scans and hashes a current save file or directory, then reports
added, modified, and removed files or directories relative to a selected
snapshot. `vault status` performs the same comparison against the newest
snapshot. Neither command changes the vault or source and neither reads every
stored object.

```bash
arkive vault diff /mnt/saves/arkive SNAPSHOT_ID ./my-game-saves
arkive vault diff /mnt/saves/arkive SNAPSHOT_ID ./my-game-saves --json
arkive vault status /mnt/saves/arkive ./my-game-saves
```

An unchanged source reports that it matches the snapshot. If the vault has no
snapshots, `status` reports that no comparison is available. Sources and vaults
must not overlap, and the same symlink, special-file, and path-name restrictions
used by snapshot capture apply.

## Restore snapshots

Restore always targets a new, explicit destination directory. Existing
destinations, including empty directories, are refused; Arkive never silently
replaces a live save directory.

```bash
arkive vault restore \
  /mnt/saves/arkive SNAPSHOT_ID /tmp/arkive-restore-test

arkive --dry-run vault restore \
  /mnt/saves/arkive SNAPSHOT_ID /tmp/arkive-restore-test
```

Arkive verifies the referenced objects, stages the complete restore beside the
destination, and publishes it atomically. A single-file snapshot is restored
inside the destination directory using its original file name.

## Planned vault work

The implemented foundation is intentionally smaller than a full game-save
cloud. Planned follow-up work includes game profiles, encrypted manifests and
objects, account namespaces, synchronization, and conflict preservation. Native
SMB/NFS support and peer-to-peer transport are not prerequisites: OS-mounted
paths remain the first remote-storage target.
