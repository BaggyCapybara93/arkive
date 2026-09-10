# Ignore rules and cleanup commands

## Ignore rules

Recursive `copy`, `move`, and `compress` operations load rules in this order:

1. Global `$XDG_CONFIG_HOME/arkive/ignore`, or `~/.config/arkive/ignore`
2. `.arkiveignore` in the source directory
3. Files supplied with `--ignore-file`
4. `--exclude` patterns
5. `--include` patterns

Later rules take precedence. Patterns support familiar gitignore-style `*`,
`**`, `?`, directory patterns, comments, anchored paths, and `!` negation.

```gitignore
target/
*.log
!logs/important.log
**/node_modules/
```

Size predicates use binary units:

```gitignore
:size > 500MB
videos/** :size > 2GB
```

Override rules for one operation with options such as:

```bash
arkive copy project backup --recursive --ignore-file team.ignore
arkive compress project project.tar.gz --exclude '*.tmp'
arkive copy project backup --recursive --include 'logs/important.log'
arkive copy project backup --recursive --exclude-larger-than 2GB
arkive copy project backup --recursive --no-global-ignore --no-local-ignore
```

Explain an ignore decision with:

```bash
arkive ignore check project/target/debug/app --root project
```

Excluded files remain at the source during a recursive move. The local
`.arkiveignore` also remains. Portable deployment metadata records the rules
that were applied.

## Rename

Rename one path:

```bash
arkive rename file1.txt file2.txt
arkive rename myproject/ myproject-v2/
```

Bulk rename matching entries with a template:

```bash
arkive rename . 'archived-{name}' --pattern '*.txt'
arkive rename logs/ '{name}-backup.{ext}' --extension log
arkive rename --recursive src/ '{name}.old.{ext}' --pattern '*.tmp'
```

Templates support `{name}` (without the final extension), `{ext}` (without
the dot), and `{original}` (the complete original name). If the template has
no dot, the original extension is preserved. `--pattern` and `--extension`
cannot be used together.

## Remove matching files

```text
arkive remove <PATH> --pattern <PATTERN> [--trash]
```

Find matching files below `PATH` and remove them. Quote patterns so the shell
does not expand them first. Removal is permanent unless `--trash` is supplied
and trash is enabled.

```bash
arkive --dry-run --verbose remove . --pattern '*.log'
arkive remove downloads/ --pattern '*.tmp' --trash
```

## Trash

Trash items are stored in `~/arkive_trash` when `HOME` is available, or
`./arkive_trash` otherwise. Name collisions receive a numeric suffix.

```bash
arkive list-trash
arkive --dry-run --verbose empty-trash
arkive empty-trash
```

`empty-trash` permanently removes every non-symlink entry in Arkive's trash.

## Deduplicate

```text
arkive deduplicate [--trash] <PATH>
```

Find files with identical hashes and remove duplicate copies. By default
duplicates are deleted permanently; use `--trash` to retain them.

```bash
arkive --dry-run --verbose deduplicate photos/
arkive deduplicate --trash photos/
```

## Cleanup

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
