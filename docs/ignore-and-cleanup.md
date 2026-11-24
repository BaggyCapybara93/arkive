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

Renaming fails if the resolved target already exists. When the destination is
a directory, the target is the file or folder with the source's name inside it.
The source and existing target are preserved.

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
`./arkive_trash` otherwise. New items have unique IDs, their original absolute
paths, and UTC deletion times. Paths must be valid UTF-8 for metadata storage.
When trash is on another filesystem, Arkive copies and verifies regular files or
directories before removing the originals. Cross-filesystem moves reject symlinks
and special files; copied items may have new timestamps.

```bash
arkive trash list
arkive trash restore <ID>
arkive trash restore <ID> --destination /path/to/restored-file
arkive --dry-run trash restore <ID>
arkive trash empty
```

The destination is the full path for the restored item. Its parent must already
exist, and Arkive will not overwrite an existing file, directory, or symlink.
Items trashed before metadata was introduced appear with `legacy:` IDs and
require `--destination` because their original paths are unknown. `list-trash`
and `empty-trash` remain available as aliases for listing and emptying trash.
Emptying permanently removes the trash contents.

## Deduplicate

```text
arkive deduplicate [--trash] <PATH>
```

Files are grouped by size before hashing, and the lexicographically smallest path
in each matching group is retained. The complete removal plan and estimated
logical byte savings are printed before removal. Use `--dry-run` to preview the
plan without changing files. By default duplicates are deleted permanently; use
`--trash` to retain them. Files in trash still occupy space until it is emptied;
actual disk savings can also differ from logical sizes (for example, with hard links).

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
