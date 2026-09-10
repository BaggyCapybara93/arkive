# File operations and backups

Arkive's general file commands use explicit destinations and transactional
staging where practical. Use `--dry-run` before destructive operations.

## Move

```text
arkive move [--recursive] <SRC> <DEST>
```

Move a file or directory. Directories require `--recursive`; recursive moves
copy the tree first and then remove the source. If ignore rules exclude part
of a recursive move, only successfully copied items are removed from the
source.

```bash
arkive move file1.txt backup/
arkive move --recursive myproject/ backup/myproject/
arkive --dry-run move --recursive myproject/ backup/myproject/
```

## Copy

```text
arkive copy [--recursive] <SRC> <DEST>
```

Copy a file or directory. Directories require `--recursive`. Regular file
copies are verified by comparing source and destination SHA-256 hashes.

```bash
arkive copy file1.txt backup/
arkive copy --recursive myproject/ backup/myproject/
arkive copy --recursive myproject/ backup/myproject/ --metadata
```

## Compress

```text
arkive compress [--method <METHOD>] <SRC> <DEST>
```

Create a tar archive compressed with gzip or Zstandard. Accepted methods are
`gzip`/`gz` and `zstd`/`zst`. The destination extension must match the selected
method: `.tar.gz`/`.tgz` for gzip or `.tar.zst`/`.tzst` for Zstandard.

```bash
arkive compress data/ backup/data.tar.gz
arkive compress --method zstd data/ backup/data.tar.zst
arkive compress data/ backup/data.tar.zst --metadata
```

Timestamped destination names are controlled by the `use_timestamp`
configuration setting. There is no separate `--timestamp` option.

## Portable backup metadata and deploy

Pass `--metadata` to `copy`, `move`, or `compress` to write a portable
`<backup>.arkive.json` sidecar. Keep the sidecar beside the backup when moving
it to another machine.

```bash
arkive compress saves saves.tar.zst --method zstd --metadata
arkive deploy saves.tar.zst --destination ./restored-saves
```

Deploy requires an explicit destination by default because the sidecar is
untrusted input. To use the recorded original path, opt in explicitly:

```bash
arkive deploy saves.tar.zst --use-recorded-destination
```

Existing destinations are refused unless `--force` is supplied. Deployment
leaves the backup intact. Compressed backups are extracted according to their
recorded compression metadata.
