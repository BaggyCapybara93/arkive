# Arkive

Arkive is a command-line file management utility written in Rust. It supports
copying, moving, renaming, compressing, removing, deduplicating, and cleaning up
files and directories.

> Arkive can permanently remove files. Use `--dry-run` to preview destructive
> operations before running them.

## Features

- Move and copy files or directory trees
- Verify copied files with SHA-256 hashes
- Create gzip or Zstandard-compressed tar archives
- Rename individual paths or bulk-rename matching entries
- Run file operations in batches from JSON
- Move removed files into Arkive's trash directory
- Find and remove duplicate files
- Find unused files and empty directories
- Create portable backups that can be deployed to their original locations
- Initialize versioned filesystem vaults and check local or mounted SMB/NFS storage
- Capture immutable save snapshots, reuse identical files, and verify snapshot integrity

## Installation

Build a release binary with Cargo:

```bash
cargo build --release
```

The binary is written to `target/release/arkive`.

## Command syntax

```text
arkive [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS]
```

Global options must be placed before the command:

- `--no-trash` disables Arkive's trash and makes requested removals permanent
- `-v`, `--verbose` prints additional operation details
- `--dry-run` previews operations without changing the filesystem
- `-h`, `--help` prints help

For help with a specific command, run:

```bash
arkive help <COMMAND>
# or
arkive <COMMAND> --help
```

## Commands

### Filesystem vaults

Vaults are the storage foundation for planned game-save management. Mount an
SMB or NFS share using your operating system, then give Arkive its local path.
Arkive does not mount shares or handle network credentials.

Use an **existing, dedicated empty directory** for initialization:

```bash
# After mounting the share and creating a dedicated directory on it:
arkive vault check /mnt/saves/arkive
arkive --dry-run vault init /mnt/saves/arkive
arkive vault init /mnt/saves/arkive
arkive vault check /mnt/saves/arkive
```

`vault init` stages and publishes a version 1 layout under `.arkive-vault/`:

```text
.arkive-vault/
  vault.json       # Format name, version, and creation time
  objects/         # <file-sha256>/data
  snapshots/       # <manifest-sha256>/manifest.json
  profiles/        # Reserved for future game profiles
```

Repeating initialization validates the existing vault without rewriting it.
Unknown versions, malformed manifests, incomplete layouts, and symlinked vault
roots or layout entries are rejected. Missing target directories are never
created, which helps avoid accidentally recreating an unavailable mount path.
An existing local mountpoint can still be unmounted: confirm the mount yourself.

`vault check` works before or after initialization. It writes and flushes a
temporary 64 KiB binary probe, verifies SHA-256 after writing and file/directory
renames, checks exclusive file creation and deletion, and cleans up its scratch data.
Existing files are preserved. In an initialized vault it also validates the
manifest and required directories. `--dry-run` validates paths and structure
without writing probe data or claiming the filesystem check passed.

Initialization, filesystem checks, and snapshot creation reserve
`.arkive-vault.lock/` in the target directory to exclude
other cooperating Arkive vault operations. An occupied lock causes an error;
Arkive never automatically breaks it. After an interruption, confirm that all
clients have stopped before manually removing a leftover lock. Inspect leftover
`.arkive-tmp-*-vault/` staging directories before removing them; initialization
refuses to proceed in a directory containing unfinished staging data.

A passing check covers this client's filesystem operations. It does **not**
prove mount identity, cross-machine locking/cache consistency, or durability
after a server crash. Vault checks do not hash stored save objects. Game
profiles, restore, synchronization, conflict handling, and encryption
are future milestones; existing file commands do not use the vault lock.

### Save snapshots

Close the game, then capture a save file or directory into an initialized vault:

```bash
arkive vault snapshot /mnt/saves/arkive ./my-game-saves --label "Before boss fight"
arkive vault snapshots /mnt/saves/arkive
arkive vault snapshots /mnt/saves/arkive --json
arkive vault verify-snapshot /mnt/saves/arkive SNAPSHOT_ID
arkive vault restore /mnt/saves/arkive SNAPSHOT_ID /tmp/arkive-restore-test
arkive --dry-run vault snapshot /mnt/saves/arkive ./my-game-saves
arkive --dry-run vault restore /mnt/saves/arkive SNAPSHOT_ID /tmp/arkive-restore-test
```

`SNAPSHOT_ID` is the full 64-character ID printed after creation or in history.
History is newest first; `--json` includes complete manifests and file hashes.
Listing validates manifest structure and hashes. `verify-snapshot` also reads
and verifies every referenced object's SHA-256 and size. These two commands
are read-only and can read published snapshots while a new one is being created.

Each snapshot records its creation time, optional label, source file/directory
name, relative paths, empty directories, file sizes, and SHA-256 hashes. A
single file uses an empty relative path for its root entry. Labels may contain
up to 256 UTF-8 bytes. Identical files share one object across all snapshots;
captures record history without rewriting older snapshots.
The vault remains version 1; snapshot manifests have their
own `arkive-snapshot` format and version 1 schema.

Arkive streams files into temporary storage, flushes and verifies them, then
publishes objects and finally the manifest. It verifies existing objects before
reusing them and refuses to overwrite corrupt data. A failed capture can leave
verified, unreferenced objects that a later capture can reuse; automatic pruning
is not implemented. Keep the whole vault together: manifests depend on objects.

The source is scanned again before publication. Detected content or tree changes
abort the snapshot, but this is **not an atomic filesystem snapshot**: keep the
game and other save writers closed throughout capture. Sources and vaults must
not overlap. Symlinks and special files are rejected; paths must be UTF-8, with
no backslashes or colons in names. Snapshots include all regular files, including
hidden files; Arkive ignore rules and general copy/compression settings do not
apply. Permissions, ownership, timestamps, ACLs, and extended attributes are
not captured. Limits are 100,000 entries, 128 path components below the source,
and a 16 MiB manifest.

`vault restore` verifies every referenced object and restores the snapshot into
a **new, explicit destination directory**. It refuses an existing destination,
including an empty one, and stages the complete restore beside it before an
atomic publish. This makes a new test directory (such as
`/tmp/arkive-restore-test`) the intended first restore target; Arkive never
automatically replaces a live save. A single-file snapshot is restored as a
file named after its original source inside that destination directory.

Dry runs scan and hash the source and check any reusable objects, but do not
write objects, manifests, restores, or locks. Snapshot contents and labels are
currently unencrypted.

### Move

```text
arkive move [--recursive] <SRC> <DEST>
```

Move a file or directory. Directories require `--recursive`; recursive moves
copy the directory tree and then remove the source.

```bash
arkive move file1.txt backup/
arkive move --recursive myproject/ backup/myproject/
arkive move file1.txt backup/file1.txt --metadata
arkive --dry-run move --recursive myproject/ backup/myproject/
```

### Copy

```text
arkive copy [--recursive] <SRC> <DEST>
```

Copy a file or directory. Directories require `--recursive`. File copies are
verified by comparing the source and destination SHA-256 hashes.

```bash
arkive copy file1.txt backup/
arkive copy --recursive myproject/ backup/myproject/
arkive copy --recursive myproject/ backup/myproject/ --metadata
```

Timestamped destination names are controlled by the `use_timestamp` config
setting; there is currently no `--timestamp` option on this command.

### Compress

```text
arkive compress [--method <METHOD>] <SRC> <DEST>
```

Create a tar archive compressed with gzip or Zstandard. Accepted methods are
`gzip`/`gz` and `zstd`/`zst`. If `--method` is omitted, Arkive uses the method
from its config.

```bash
arkive compress data/ backup/data.tar.gz
arkive compress --method zstd data/ backup/data.tar.zst
arkive compress data/ backup/data.tar.gz --metadata
```

Timestamped destination names are controlled by the `use_timestamp` config
setting; there is currently no `--timestamp` option on this command.
Gzip destinations must end in `.tar.gz` or `.tgz`; Zstandard destinations must
end in `.tar.zst` or `.tzst`.

### Deploy a backup

Pass `--metadata` to `copy`, `move`, or `compress` to create a portable
`<backup>.arkive.json` sidecar. Keep this file beside the backup when moving it
to another machine. Because the sidecar is untrusted input, deploy requires an
explicit destination by default:

```bash
arkive compress test test.tar.gz --metadata
rm -rf test
arkive deploy test.tar.gz --destination ./test
```

To restore to the path recorded in the sidecar, opt in with
`--use-recorded-destination`. Arkive refuses to overwrite an existing
destination unless `--force` is supplied, including for partial moves.

```bash
arkive deploy test.tar.gz --destination /srv/restored/test
arkive deploy test.tar.gz --use-recorded-destination --force
```

Deployment copies regular and directory backups, so the backup remains intact.
Gzip and Zstandard archives are extracted according to their saved metadata.

## Ignore rules

Recursive `copy`, `move`, and `compress` operations automatically load ignore
rules in this order:

1. The global `$XDG_CONFIG_HOME/arkive/ignore` file, or
   `~/.config/arkive/ignore` when `XDG_CONFIG_HOME` is unset
2. `.arkiveignore` in the source directory
3. Files passed with `--ignore-file`
4. Patterns passed with `--exclude`
5. Patterns passed with `--include`

Later rules take precedence. Rules use familiar gitignore-style `*`, `**`,
`?`, directory patterns, comments, anchored paths, and `!` negation:

```gitignore
# .arkiveignore
target/
*.log
!logs/important.log
**/node_modules/
```

Arkive also supports size predicates. Units are binary (for example, `1MB` is
1,048,576 bytes):

```gitignore
# Exclude every file larger than 500 MB
:size > 500MB

# Exclude large videos only
videos/** :size > 2GB
```

Rules can be adjusted for a single operation:

```bash
arkive compress project project.tar.gz --exclude '*.tmp'
arkive copy project backup --recursive --include 'logs/important.log'
arkive compress project project.tar.gz --exclude-larger-than 2GB
arkive copy project backup --recursive --ignore-file team.ignore
arkive copy project backup --recursive --no-global-ignore --no-local-ignore
```

Use `ignore check` to explain a decision:

```bash
arkive ignore check project/target/debug/app --root project
```

When a recursive move excludes anything, Arkive removes only successfully
copied items. Excluded files and directories remain at the source, along with
the local `.arkiveignore`. Deploying that partial move later merges the backed
up content into the remaining source tree instead of deleting the excluded
items. Portable deployment metadata records the applied rules for auditing.

### Rename

Rename one path:

```text
arkive rename <NAME> <NEW_NAME>
```

```bash
arkive rename file1.txt file2.txt
arkive rename myproject/ myproject-v2/
```

Bulk-rename entries in a directory by glob pattern or extension:

```text
arkive rename [--recursive] <DIRECTORY> <TEMPLATE> --pattern <PATTERN>
arkive rename [--recursive] <DIRECTORY> <TEMPLATE>
```

Templates support:

- `{name}`: original name without its final extension
- `{ext}`: final extension without the dot
- `{original}`: complete original file name

If a template produces no dot, Arkive preserves the original extension.
`--pattern` supports `*` and `?` globs. `--pattern` and `--extension` cannot be
used together.

```bash
# notes.txt -> archived-notes.txt
arkive rename . 'archived-{name}' --pattern '*.txt'

# report.log -> report-backup.log
arkive rename logs/ '{name}-backup.{ext}' --extension log

arkive rename --recursive src/ '{name}.old.{ext}' --pattern '*.tmp'
```

### Remove matching files

```text
arkive remove <PATH> --pattern <PATTERN> [--trash]
```

Recursively find files below `PATH` whose names match a glob pattern, then
remove them. Removal is permanent unless `--trash` is supplied and trash is
enabled. Use quotes around patterns so the shell does not expand them first.

```bash
arkive --dry-run --verbose remove . --pattern '*.log'
arkive remove downloads/ --pattern '*.tmp' --trash
```

### Batch operations

```text
arkive batch <FILE>
```

Run operations from a JSON file. The file can contain an `operations` object as
shown below, or a top-level array of operations. Supported `work_type` values
are `move`, `copy`, `compress`, and `rename`.

```json
{
  "operations": [
    {
      "work_type": "move",
      "source": "file1.txt",
      "destination": "backup/"
    },
    {
      "work_type": "copy",
      "source": "myproject/",
      "destination": "backup/myproject/",
      "recursive": true,
      "timestamp": false
    },
    {
      "work_type": "compress",
      "source": "data/",
      "destination": "backup/data.tar.zst",
      "compression_method": "zstd"
    }
  ]
}
```

Each operation accepts `source` and may accept `destination`, `recursive`,
`timestamp`, `compression_method`, and `cleanup`. Batch operations may run in
parallel, so avoid operations that modify overlapping paths.

```bash
arkive batch batch.json
```

### Trash management

Files sent to trash are stored in `~/arkive_trash` when `HOME` is available,
or in `./arkive_trash` otherwise. Name collisions receive a numeric suffix.

```bash
arkive list-trash
arkive --dry-run --verbose empty-trash
arkive empty-trash
```

`empty-trash` permanently removes every non-symlink entry in Arkive's trash.

### Deduplicate

```text
arkive deduplicate [--trash] <PATH>
```

Recursively find files with identical hashes and remove duplicate copies. By
default duplicates are deleted permanently; pass `--trash` to retain them in
Arkive's trash.

```bash
arkive --dry-run --verbose deduplicate photos/
arkive deduplicate --trash photos/
```

### Cleanup

```text
arkive cleanup [OPTIONS] [PATH]
```

`PATH` defaults to the current directory. Options can be combined:

- `--empty-trash` empties Arkive's trash
- `--deduplicate` removes duplicate files
- `--scan-unused` finds files not accessed in at least 30 days
- `--scan-empty-dirs` finds and removes empty directories

```bash
arkive cleanup --empty-trash
arkive cleanup --deduplicate --scan-empty-dirs
arkive cleanup --scan-unused /path/to/directory
arkive --dry-run --verbose cleanup --deduplicate --scan-empty-dirs .
```

## Configuration

On first run, Arkive creates `config.json` beside the executable. Supported
settings include:

```json
{
  "enable_trash": true,
  "verbose": false,
  "dry_run": false,
  "recursive": false,
  "enable_metadata": false,
  "compression_method": "gzip",
  "use_timestamp": false,
  "created_at": 1787616000,
  "updated_at": 1787616000
}
```

The timestamp fields are Unix timestamps and are created automatically. Global
boolean flags can enable or disable selected config behavior for a run, but
there is currently no CLI command for editing the config.

## Development

```bash
cargo test --all-targets
cargo fmt --all -- --check
cargo clippy --all-targets
```

## License

Arkive is licensed under the Zero-Clause BSD (0BSD) license. See [LICENSE](LICENSE).

## Contributing

Contributions are welcome.
