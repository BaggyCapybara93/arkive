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
      "type": "move",
      "src": "file1.txt",
      "dest": "backup/",
      "recursive": false
    },
    {
      "type": "copy",
      "src": "myproject/",
      "dest": "backup/myproject/",
      "recursive": true
    },
    {
      "type": "compress",
      "src": "data/",
      "dest": "backup/data.tar.gz"
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

## License

This project is licensed under the MIT License

## Contributing

Contributions are welcomed!
