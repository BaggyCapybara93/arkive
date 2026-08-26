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
- `--last-used-directory <PATH>` is currently reserved and has no effect
- `-h`, `--help` prints help

For help with a specific command, run:

```bash
arkive help <COMMAND>
# or
arkive <COMMAND> --help
```

## Commands

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
to another machine. Deploy restores the backup to its recorded original path:

```bash
arkive compress test test.tar.gz --metadata
rm -rf test
arkive deploy test.tar.gz
```

Use `--destination` when the original absolute path is not suitable on the
current machine. Arkive refuses to overwrite an existing destination unless
`--force` is supplied.

```bash
arkive deploy test.tar.gz --destination /srv/restored/test
arkive deploy test.tar.gz --force
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

Although `--extension` appears in command help, the current CLI definition
requires `--pattern` and makes the two options conflict. Extension-only removal
is therefore not currently usable; use a pattern such as `*.log` instead.

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

Arkive is licensed under the MIT License. See [LICENSE](LICENSE).

## Contributing

Contributions are welcome.
