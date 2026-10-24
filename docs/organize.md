# Organize files into folders

`organize by-extension` sorts the immediate regular files in one directory into
folders named for their lowercase extensions. For example, `report.PDF` moves
to `pdf/report.PDF`. Files without extensions go to `no-extension/`, and
extensions containing characters other than ASCII letters and digits go to
`other-extension/`.

```bash
arkive --dry-run organize by-extension ./downloads --folder-metadata
arkive organize by-extension ./downloads --folder-metadata
arkive --dry-run organize undo ./downloads/.arkive-organize-RUN.json
arkive organize undo ./downloads/.arkive-organize-RUN.json
```

The command does not recurse or move existing directories or hidden files.
It refuses symbolic links among the visible entries, an Arkive vault root,
and any destination filename that already exists. It checks the complete
plan for collisions before moving files. A run prints the exact undo manifest
path and saves that manifest in the source directory before the first move.
If a run stops partway through, use that manifest to undo files already moved.

`--folder-metadata` is optional. When used, Arkive writes
`.arkive-folder.json` inside each destination folder. It records the folder's
extension rule and the names and SHA-256 hashes of files Arkive placed there.
Existing metadata from earlier runs is updated; unrelated files in the
folder are left alone. This file is an inventory of Arkive-organized files,
not a rule that automatically moves future files.

Undo checks that each file still matches its saved hash and refuses to
overwrite a file at its original location. It removes the matching inventory
entries and folders the run created when they are empty. Keep the undo
manifest until you no longer need to reverse the run. Organizing live game
save directories can change paths expected by games or Arkive profiles.
