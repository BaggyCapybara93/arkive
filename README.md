# Arkive

A simple file management utility written in Rust. Primarily made for myself to learn Rust.

## Features

- **Move files and directories**: Relocate files with optional recursive directory support
- **Copy files and directories**: Duplicate files with integrity verification using SHA-256 hashing
- **Delete files and directories**: Remove files with optional recursive directory deletion
- **Integrity checking**: Automatic hash verification after copy operations to ensure data integrity

## Usage

### Help
```bash
arkive help
```

### Move Files
```bash
arkive move <source> <destination>
```

Move a file or directory to a new location.

**Recursive mode for directories:**
```bash
arkive move <source> <destination> --recursive
```

### Copy Files
```bash
arkive copy <source> <destination>
```

**Recursive mode for directories:**
```bash
arkive copy <source> <destination> --recursive
```

## License

This project is licensed under the MIT License

## Contributing

Contributions are welcomed!
