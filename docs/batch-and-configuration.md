# Batch files and configuration

## Batch operations

```text
arkive batch <FILE>
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
      "destination": "backup/data.tar.zst",
      "compression_method": "zstd"
    }
  ]
}
```

Operations may use `source`, `destination`, `recursive`, `timestamp`,
`compression_method`, and `cleanup`. Batch jobs may run in parallel, so avoid
operations that modify overlapping paths.

```bash
arkive batch batch.json
```

## Configuration

On first run, Arkive creates `config.json` beside the executable. The current
configuration includes settings such as:

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
CLI flags can override selected behavior for one run. There is currently no
CLI command for editing the configuration, and portable backup metadata is
separate from this runtime configuration.
