//! Directory-handle based access to the server's configured files root.
//! Client paths never become ambient filesystem paths.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, RenameFlags};
use sha2::{Digest, Sha256};

use super::error::{invalid, io};
use super::files::{self, Entry, Manifest};
use super::protocol::RemoteEntry;
use crate::error::AppError;

static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn mode(bits: u16) -> Mode {
    Mode::from_bits_truncate(bits)
}
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn file_flags() -> OFlags {
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn components(path: &str, allow_root: bool) -> Result<Vec<&str>, AppError> {
    files::validate_relative(path, allow_root)?;
    Ok(if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    })
}

fn open_dir_from(base: &impl AsFd, path: &str) -> Result<OwnedFd, AppError> {
    let mut current = fs::openat(base, ".", dir_flags(), Mode::empty()).map_err(io)?;
    for part in components(path, true)? {
        current = fs::openat(&current, part, dir_flags(), Mode::empty()).map_err(io)?;
    }
    Ok(current)
}

fn parent_and_name(path: &str) -> Result<(&str, &str), AppError> {
    files::validate_relative(path, false)?;
    Ok(path.rsplit_once('/').unwrap_or(("", path)))
}

fn open_parent(base: &impl AsFd, path: &str, create: bool) -> Result<(OwnedFd, String), AppError> {
    let (parent, name) = parent_and_name(path)?;
    let mut current = fs::openat(base, ".", dir_flags(), Mode::empty()).map_err(io)?;
    for part in components(parent, true)? {
        if create {
            match fs::mkdirat(&current, part, mode(0o700)) {
                Ok(()) => {}
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(error) => return Err(io(error)),
            }
        }
        current = fs::openat(&current, part, dir_flags(), Mode::empty()).map_err(io)?;
    }
    Ok((current, name.into()))
}

fn entry_type(parent: &impl AsFd, name: &str) -> Result<FileType, AppError> {
    let stat = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
    Ok(FileType::from_raw_mode(stat.st_mode))
}

fn names(dir: &impl AsFd) -> Result<Vec<String>, AppError> {
    let mut reader = Dir::read_from(dir).map_err(io)?;
    let mut names = Vec::new();
    while let Some(entry) = reader.read() {
        let entry = entry.map_err(io)?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| invalid("Remote files need UTF-8 names"))?;
        if name == "." || name == ".." || name.starts_with(".arkive-stage-") {
            continue;
        }
        files::validate_relative(name, false)?;
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}

fn hash_file(mut file: File) -> Result<(String, u64), AppError> {
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("Remote file size overflow"))?;
    }
    Ok((hex(&hash.finalize()), total))
}

fn scan_dir(dir: &impl AsFd, prefix: &str, entries: &mut Vec<Entry>) -> Result<(), AppError> {
    for name in names(dir)? {
        if entries.len() >= 100_000 {
            return Err(invalid("Remote file tree has too many entries"));
        }
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        files::validate_relative(&relative, false)?;
        match entry_type(dir, &name)? {
            FileType::Directory => {
                let child =
                    fs::openat(dir, name.as_str(), dir_flags(), Mode::empty()).map_err(io)?;
                entries.push(Entry::Directory {
                    path: relative.clone(),
                });
                scan_dir(&child, &relative, entries)?;
            }
            FileType::RegularFile => {
                let file = File::from(
                    fs::openat(dir, name.as_str(), file_flags(), Mode::empty()).map_err(io)?,
                );
                let (sha256, size) = hash_file(file)?;
                entries.push(Entry::File {
                    path: relative,
                    size,
                    sha256,
                });
            }
            _ => {
                return Err(invalid(
                    "Remote files root contains a symlink or special file",
                ));
            }
        }
    }
    Ok(())
}

fn remove_tree(dir: &impl AsFd) -> Result<(), AppError> {
    for name in names(dir)? {
        match entry_type(dir, &name)? {
            FileType::Directory => {
                let child =
                    fs::openat(dir, name.as_str(), dir_flags(), Mode::empty()).map_err(io)?;
                remove_tree(&child)?;
                fs::unlinkat(dir, name.as_str(), AtFlags::REMOVEDIR).map_err(io)?;
            }
            _ => fs::unlinkat(dir, name.as_str(), AtFlags::empty()).map_err(io)?,
        }
    }
    Ok(())
}

pub(super) struct FilesRoot {
    fd: OwnedFd,
}

impl FilesRoot {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let fd = fs::open(path, dir_flags(), Mode::empty()).map_err(io)?;
        Ok(Self { fd })
    }

    pub fn list(&self, path: &str) -> Result<Vec<RemoteEntry>, AppError> {
        let dir = open_dir_from(&self.fd, path)?;
        let mut result = Vec::new();
        for name in names(&dir)? {
            let stat = fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
            let kind = match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => "directory",
                FileType::RegularFile => "file",
                _ => {
                    return Err(invalid(
                        "Remote files root contains a symlink or special file",
                    ));
                }
            };
            result.push(RemoteEntry {
                name,
                kind: kind.into(),
                size: if kind == "file" {
                    stat.st_size as u64
                } else {
                    0
                },
            });
        }
        Ok(result)
    }

    pub fn begin_upload(
        &self,
        destination: &str,
        manifest: Manifest,
    ) -> Result<UploadStage<'_>, AppError> {
        files::validate_relative(destination, false)?;
        manifest.validate()?;
        manifest.file_count_and_bytes()?;
        let (dest_parent, dest_name) = open_parent(&self.fd, destination, true)?;
        match fs::statat(&dest_parent, dest_name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err(invalid("Remote destination already exists")),
            Err(error) if error == rustix::io::Errno::NOENT => {}
            Err(error) => return Err(io(error)),
        }
        let mut stage = None;
        for _ in 0..1000 {
            let name = format!(
                ".arkive-stage-{}-{}",
                std::process::id(),
                STAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            match fs::mkdirat(&self.fd, name.as_str(), mode(0o700)) {
                Ok(()) => {
                    stage = Some(name);
                    break;
                }
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(error) => return Err(io(error)),
            }
        }
        let stage_name =
            stage.ok_or_else(|| invalid("Could not reserve remote staging directory"))?;
        let stage_fd =
            fs::openat(&self.fd, stage_name.as_str(), dir_flags(), Mode::empty()).map_err(io)?;
        Ok(UploadStage {
            root: self,
            stage_name,
            stage_fd,
            dest_parent,
            dest_name,
            manifest,
            published: false,
        })
    }

    pub fn export(&self, source: &str) -> Result<FileExport<'_>, AppError> {
        let (parent, name) = open_parent(&self.fd, source, false)?;
        let mut entries = Vec::new();
        match entry_type(&parent, &name)? {
            FileType::Directory => {
                let dir =
                    fs::openat(&parent, name.as_str(), dir_flags(), Mode::empty()).map_err(io)?;
                entries.push(Entry::Directory { path: "".into() });
                scan_dir(&dir, "", &mut entries)?;
            }
            FileType::RegularFile => {
                let file = File::from(
                    fs::openat(&parent, name.as_str(), file_flags(), Mode::empty()).map_err(io)?,
                );
                let (sha256, size) = hash_file(file)?;
                entries.push(Entry::File {
                    path: "".into(),
                    size,
                    sha256,
                });
            }
            _ => {
                return Err(invalid(
                    "Remote download refuses symlinks and special files",
                ));
            }
        }
        let manifest = Manifest {
            format: "arkive-remote-files".into(),
            version: 1,
            entries,
        };
        let bytes = manifest.encode()?;
        Ok(FileExport {
            root: self,
            source: source.into(),
            manifest,
            bytes,
        })
    }
}

pub(super) struct UploadStage<'a> {
    root: &'a FilesRoot,
    stage_name: String,
    stage_fd: OwnedFd,
    dest_parent: OwnedFd,
    dest_name: String,
    manifest: Manifest,
    published: bool,
}

impl UploadStage<'_> {
    pub fn receive_and_publish(mut self, input: &mut impl Read) -> Result<u64, AppError> {
        let mut written = 0u64;
        let root_file = matches!(self.manifest.entries[0], Entry::File { .. });
        for entry in &self.manifest.entries {
            match entry {
                Entry::Directory { path } if path.is_empty() => {}
                Entry::Directory { path } => {
                    let (parent, name) = open_parent(&self.stage_fd, path, false)?;
                    fs::mkdirat(&parent, name.as_str(), mode(0o700)).map_err(io)?;
                }
                Entry::File { path, size, sha256 } => {
                    let name = if path.is_empty() { ".payload" } else { path };
                    let (parent, file_name) = open_parent(&self.stage_fd, name, false)?;
                    let fd = fs::openat(
                        &parent,
                        file_name.as_str(),
                        OFlags::WRONLY
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC,
                        mode(0o600),
                    )
                    .map_err(io)?;
                    let mut output = File::from(fd);
                    let mut hash = Sha256::new();
                    let mut left = *size;
                    let mut buffer = [0u8; 64 * 1024];
                    while left > 0 {
                        let chunk = buffer.len().min(left as usize);
                        let count = input.read(&mut buffer[..chunk])?;
                        if count == 0 {
                            return Err(invalid("Remote upload ended early"));
                        }
                        output.write_all(&buffer[..count])?;
                        hash.update(&buffer[..count]);
                        left -= count as u64;
                    }
                    output.sync_all()?;
                    if hex(&hash.finalize()) != *sha256 {
                        return Err(invalid("Remote upload SHA-256 mismatch"));
                    }
                    written = written
                        .checked_add(*size)
                        .ok_or_else(|| invalid("Remote upload size overflow"))?;
                }
            }
        }
        let (from_dir, from_name): (&dyn AsFd, &str) = if root_file {
            (&self.stage_fd, ".payload")
        } else {
            (&self.root.fd, self.stage_name.as_str())
        };
        fs::renameat_with(
            from_dir,
            from_name,
            &self.dest_parent,
            self.dest_name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(io)?;
        self.published = true;
        if root_file {
            fs::unlinkat(&self.root.fd, self.stage_name.as_str(), AtFlags::REMOVEDIR)
                .map_err(io)?;
        }
        Ok(written)
    }
}

impl Drop for UploadStage<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = remove_tree(&self.stage_fd);
            let _ = fs::unlinkat(&self.root.fd, self.stage_name.as_str(), AtFlags::REMOVEDIR);
        }
    }
}

pub(super) struct FileExport<'a> {
    root: &'a FilesRoot,
    source: String,
    manifest: Manifest,
    bytes: Vec<u8>,
}

impl FileExport<'_> {
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn send(&self, output: &mut impl Write) -> Result<u64, AppError> {
        let mut sent = 0u64;
        for entry in &self.manifest.entries {
            let Entry::File { path, size, sha256 } = entry else {
                continue;
            };
            let full = if path.is_empty() {
                self.source.clone()
            } else {
                format!("{}/{path}", self.source)
            };
            let (parent, name) = open_parent(&self.root.fd, &full, false)?;
            let fd = fs::openat(&parent, name.as_str(), file_flags(), Mode::empty()).map_err(io)?;
            let mut file = File::from(fd);
            let mut hash = Sha256::new();
            let mut left = *size;
            let mut buffer = [0u8; 64 * 1024];
            while left > 0 {
                let chunk = buffer.len().min(left as usize);
                let count = file.read(&mut buffer[..chunk])?;
                if count == 0 {
                    return Err(invalid("Remote file changed during download"));
                }
                output.write_all(&buffer[..count])?;
                hash.update(&buffer[..count]);
                left -= count as u64;
            }
            if file.read(&mut buffer[..1])? != 0 || hex(&hash.finalize()) != *sha256 {
                return Err(invalid("Remote file changed during download"));
            }
            sent = sent
                .checked_add(*size)
                .ok_or_else(|| invalid("Remote download size overflow"))?;
        }
        Ok(sent)
    }
}
