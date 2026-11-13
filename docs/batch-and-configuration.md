# Batch files and configuration

## Batch operations

```text
arkive batch <FILE>
arkive batch <FILE> --plan
```

Batch files may contain an object with an `operations` array or a top-level
array. Supported `work_type` values are `move`, `copy`, `compress`, and
`rename`.

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
      "destination": "backup/data.tar.xz",
      "compression_method": "xz"
    },
    {
      "work_type": "compress",
      "source": "portable-saves/",
      "destination": "backup/portable-saves.zip",
      "archive_format": "zip"
    }
  ]
}
```

Operations may use `source`, `destination`, `recursive`, `timestamp`,
`compression_method`, `archive_format`, and `cleanup`. `archive_format` is
`tar` by default; use `zip` for a Deflate-compressed ZIP archive.
`compression_method` applies only to tar jobs.

`cleanup` is an optional object that runs only the named cleanup actions after
its job succeeds. A bare `true` is intentionally not accepted: emptying the
trash, deduplicating, and removing empty directories must be explicit. `path`
is required for `deduplicate`, `scan_unused`, and `scan_empty_dirs`; it is not
needed for `empty_trash` alone.

```json
{
  "work_type": "copy",
  "source": "screenshots/",
  "destination": "backup/screenshots/",
  "recursive": true,
  "cleanup": {
    "path": "backup/screenshots/",
    "scan_empty_dirs": true
  }
}
```

Batch jobs may run in parallel, so avoid operations that modify overlapping
paths. If a job fails, other queued jobs may still finish. The command exits
with an error identifying the lowest-numbered failed job and its cause.

Before submitting jobs, Arkive validates the batch and prints a preflight plan
with resolved source paths, destination base paths, and cleanup actions. A
timestamped destination's exact name is chosen at run time. The plan flags
overlapping paths because those jobs wait on path locks and their execution
order is not guaranteed.
Use `--plan` to inspect this information without running jobs or creating a
configuration file. The plan does not scan file contents or guarantee that
paths will remain unchanged before execution.

```bash
arkive batch batch.json
arkive batch batch.json --plan
```

## Configuration

On first non-configuration command, Arkive creates `config.json` beside the
executable. Inspect and update it through the CLI:

```bash
arkive config path
arkive config show
arkive config get compression-method
arkive config set compression-method zstd
arkive config set enable-trash false
arkive --dry-run config set enable-trash false
```

Boolean values must be `true` or `false`. The supported keys are
`enable-trash`, `verbose`, `dry-run`, `recursive`, `enable-metadata`,
`compression-method`, and `use-timestamp`.
`--dry-run` previews a `config set` change without writing it.

The configuration includes settings such as:

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

The timestamp fields are Unix timestamps and are managed automatically. Global
CLI flags can override selected behavior for one run. Portable backup metadata
is separate from this runtime configuration.
