# Arkive

Arkive is a Rust command-line utility for safe, inspectable file operations
and local-first game-save storage.

> Arkive includes destructive commands. Use `--dry-run` first when you are
> unsure what a command will change.

## What it does

- Copy and move files or directory trees with transactional installation
- Create gzip or Zstandard tar archives
- Create portable backup metadata and deploy backups safely
- Rename, remove, deduplicate, and clean up files
- Run file operations from JSON batch files
- Initialize filesystem-backed vaults and check local or OS-mounted SMB/NFS paths
- Capture immutable, deduplicated save snapshots and restore them safely
- Compare current saves with a selected or newest snapshot
- Save named game profiles for repeatable snapshot and status commands
- Audit vault health and safely reclaim unreferenced snapshot objects
- Prune old snapshots while retaining protected history

Vaults currently store profiles, snapshots, and verified content-addressed
objects. Synchronization, conflict handling, and encryption are planned but
not implemented yet.

## Install

Build a release binary with Cargo:

```bash
cargo build --release
```

The binary is written to `target/release/arkive`.

## Quick start

```bash
# Preview a destructive operation.
arkive --dry-run --verbose remove ./saves --pattern '*.tmp'

# Copy a directory and verify its files.
arkive copy ./saves ./backup --recursive

# Create a compressed backup.
arkive compress ./saves ./saves.tar.zst --method zstd

# Initialize and check a dedicated vault directory.
arkive vault init /mnt/saves/arkive
arkive vault check /mnt/saves/arkive
arkive vault health /mnt/saves/arkive
arkive --dry-run vault gc /mnt/saves/arkive
arkive --dry-run vault prune /mnt/saves/arkive --keep-last 20

# Capture and verify a save snapshot.
arkive vault snapshot /mnt/saves/arkive ./saves --label "Before boss fight"
arkive vault snapshots /mnt/saves/arkive
arkive vault verify-snapshot /mnt/saves/arkive SNAPSHOT_ID
arkive vault status /mnt/saves/arkive ./saves
arkive vault profile add /mnt/saves/arkive my-game ./saves
arkive vault snapshot /mnt/saves/arkive --profile my-game --label "Before boss fight"
arkive vault status /mnt/saves/arkive --profile my-game
```

For a mounted SMB or NFS share, mount it with the operating system first and
pass Arkive the resulting local path. Arkive does not mount shares or manage
network credentials.

## Command guides

See the focused documentation for details and safety notes:

- [Vaults and save snapshots](docs/vault.md)
- [Copy, move, compress, and deploy](docs/file-operations.md)
- [Ignore rules, rename, remove, trash, and cleanup](docs/ignore-and-cleanup.md)
- [Batch files and configuration](docs/batch-and-configuration.md)

The complete command syntax is also available from the binary:

```bash
arkive --help
arkive help <COMMAND>
arkive <COMMAND> --help
```

Global options must appear before the command:

```text
arkive [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS]
```

- `--dry-run` previews filesystem changes without writing them
- `--no-trash` disables Arkive's trash for removal operations
- `-v`, `--verbose` prints additional details

## Development

```bash
cargo test --all-targets
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo build --release
```

## License

Arkive is licensed under the [Zero-Clause BSD (0BSD) license](LICENSE).

Contributions are welcome.
