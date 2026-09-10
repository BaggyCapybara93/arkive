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

Vaults currently store snapshots and verified content-addressed objects. Game
profiles, synchronization, conflict handling, and encryption are planned but
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

# Capture and verify a save snapshot.
arkive vault snapshot /mnt/saves/arkive ./saves --label "Before boss fight"
arkive vault snapshots /mnt/saves/arkive
arkive vault verify-snapshot /mnt/saves/arkive SNAPSHOT_ID
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
