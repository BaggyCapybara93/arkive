# Arkive

A simple file management utility written in Rust. Primarily made for myself to learn Rust.

## Features

- **Move files and directories**: Relocate files with optional recursive directory support
- **Copy files and directories**: Duplicate files with integrity verification using SHA-256 hashing
- **Delete files and directories**: Remove files with optional recursive directory deletion and trash support
- **Compress files and directories**: Create tar.gz archives
- **Batch operations**: Execute multiple file operations from a JSON configuration file
- **Trash management**: Soft-delete files to arkive_trash directory with list and empty commands
- **Deduplication**: Scan directories for duplicate files (same hash) and remove them
- **Integrity checking**: Automatic hash verification after copy operations to ensure data integrity

## Global Options

All commands support the following global options:

```bash
arkive [OPTIONS] <COMMAND>
```

**Options:**
- `--no-trash`: Disable trash, permanently delete files
- `--verbose, -v`: Enable verbose output
- `--dry-run`: Preview operations without executing

## Usage

### Help
```bash
arkive help
```

### Move Files
```bash
arkive move <source> <destination> [OPTIONS]
```

Move a file or directory to a new location.

**Options:**
- `--recursive`: Move directories recursively (required for directories)

**Examples:**
```bash
# Move a single file
arkive move file1.txt backup/

# Move a directory recursively
arkive move myproject/ backup/myproject/ --recursive
```

### Copy Files
```bash
arkive copy <source> <destination> [OPTIONS]
```

Copy a file or directory to a new location with integrity verification.

**Options:**
- `--recursive`: Copy directories recursively (required for directories)

**Examples:**
```bash
# Copy a single file
arkive copy file1.txt backup/

# Copy a directory recursively
arkive copy myproject/ backup/myproject/ --recursive
```

### Compress Files
```bash
arkive compress <source> <destination>
```

Compress a file or directory into a tar.gz archive.

**Examples:**
```bash
# Compress a single file
arkive compress file1.txt backup/file1.txt.gz

# Compress a directory
arkive compress myproject/ backup/myproject.tar.gz

### Rename Files
```bash
arkive rename <source> <destination>
```

Rename a file or directory to a new name.

**Examples:**
```bash
# Rename a single file
arkive rename file1.txt file2.txt

# Rename a file in a directory
arkive rename file1.txt backup/file1.txt

# Rename a directory
arkive rename myproject/ myproject_v2/
```

### Batch Operations
```bash
arkive batch <batch_file>
```

Execute a batch of file operations from a JSON configuration file.

**Example batch file (batch.json):**
```json
{
  "operations": [
    {
      "work_type": "move",
      "source": "file1.txt",
      "destination": "backup/",
      "recursive": false
    },
    {
      "work_type": "copy",
      "source": "myproject/",
      "destination": "backup/myproject/",
      "recursive": true
    },
    {
      "work_type": "compress",
      "source": "data/",
      "destination": "backup/data.tar.gz"
    }
  ]
}
```

**Usage:**
```bash
arkive batch batch.json
```

### Trash Management

#### Empty Trash
```bash
arkive empty-trash [OPTIONS]
```

Empty the arkive trash directory, permanently deleting all files.

**Example:**
```bash
arkive empty-trash --verbose
```

#### List Trash
```bash
arkive list-trash [OPTIONS]
```

List all files in the arkive trash directory.

**Example:**
```bash
arkive list-trash --verbose
```

### Deduplication
```bash
arkive deduplicate <path> [OPTIONS]
```

Scan a directory for duplicate files (same hash) and remove them.

**Options:**
- `--trash`: Keep deleted files in arkive trash instead of permanent deletion

**Examples:**
```bash
# Permanently delete duplicates
arkive deduplicate /path/to/folder

# Move duplicates to trash
arkive deduplicate /path/to/folder --trash
```

### Cleanup
```bash
arkive cleanup [OPTIONS] [PATH]
```

Clean up the workspace with multiple options. This command can perform several cleanup operations in one go.

**Options:**
- `--empty-trash`: Empty the arkive trash directory
- `--deduplicate`: Scan for and remove duplicate files
- `--scan-unused`: Scan for unused files (files not accessed in 30+ days)
- `--scan-empty-dirs`: Scan for and remove empty directories
- `PATH`: Optional path to scan (defaults to current directory)

**Examples:**
```bash
# Empty trash only
arkive cleanup --empty-trash

# Remove duplicates only
arkive cleanup --deduplicate

# Scan for unused files
arkive cleanup --scan-unused

# Remove empty directories
arkive cleanup --scan-empty-dirs

# Combine multiple operations
arkive cleanup --empty-trash --deduplicate --scan-empty-dirs

# Clean a specific directory with multiple operations
arkive cleanup --path /path/to/directory --deduplicate --scan-unused

# Preview cleanup operations without executing
arkive cleanup --dry-run --empty-trash --deduplicate
```

## Branch Overview

This project uses a traditional branching model to maintain stability while allowing active development.

### Branches

**`master`**
- Primary development branch
- Contains the latest features and improvements
- May contain breaking changes during active development

**`stable`**
- Production-ready, tested build
- Should not be modified except for bug fixes and verified updates

### Release Process

1. Complete development and testing on `master`
2. Create a release tag (e.g., `v1.0.0`) from `master`
3. Merge release tag into `stable`
4. Update `stable` branch to the tagged version
5. Announce release with changelog

### Bug Fixes

- Critical bug fixes can be cherry-picked from `master` to `stable`
- Non-critical fixes should be developed on `master` and merged normally

## License

This project is licensed under the MIT License

## Contributing

Contributions are welcomed!
