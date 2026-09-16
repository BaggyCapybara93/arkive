use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::metadata_module::error::MetadataError;

///This manages the core metadata folder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalIndex {
    #[serde(default = "metadata_format_version")]
    pub version: u8,
    #[serde(default)]
    pub map: HashMap<PathBuf, PathBuf>,
}

fn metadata_format_version() -> u8 {
    1
}

impl Default for GlobalIndex {
    fn default() -> Self {
        Self {
            version: metadata_format_version(),
            map: HashMap::new(),
        }
    }
}
pub struct CoreMetadataManager {
    index_path: PathBuf,
    shards_dir: PathBuf,
}

impl CoreMetadataManager {
    pub fn new() -> Result<Self, MetadataError> {
        let exe_dir = std::env::current_exe()
            .map_err(|e| MetadataError::PathError(format!("Failed to get executable path: {}", e)))?
            .parent()
            .ok_or_else(|| {
                MetadataError::PathError("Executable has no parent directory".to_string())
            })?
            .to_path_buf();

        Ok(Self::from_core_dir(exe_dir.join("core")))
    }

    pub(crate) fn from_core_dir(core_dir: PathBuf) -> Self {
        let index_path = core_dir.join("index.json");
        let shards_dir = core_dir.join("shards");

        Self {
            index_path,
            shards_dir,
        }
    }

    pub fn canonicalize(&self, path: &Path) -> Result<PathBuf, MetadataError> {
        fs::canonicalize(path)
            .map_err(|source| MetadataError::io("canonicalize metadata path", path, source))
    }

    //Index loading/saving
    pub fn load_index(&self) -> Result<GlobalIndex, MetadataError> {
        let content = match fs::read_to_string(&self.index_path) {
            Ok(content) => content,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GlobalIndex::default());
            }
            Err(source) => {
                return Err(MetadataError::io(
                    "read metadata index",
                    &self.index_path,
                    source,
                ));
            }
        };
        let index =
            serde_json::from_str(&content).map_err(|source| MetadataError::CorruptIndex {
                path: self.index_path.clone(),
                source,
            })?;
        Ok(index)
    }

    pub fn save_index(&self, index: &GlobalIndex) -> Result<(), MetadataError> {
        if let Some(parent) = self.index_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| MetadataError::io("create metadata directory", parent, source))?;
        }
        let content = serde_json::to_vec(index).map_err(|source| {
            MetadataError::json("serialize metadata index", &self.index_path, source)
        })?;
        Self::atomic_write_file(&self.index_path, &content)?;
        Ok(())
    }

    fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), MetadataError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| MetadataError::io("create metadata directory", parent, source))?;
        }

        let temp_path = Self::temp_path(path)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|source| {
                MetadataError::io("create temporary metadata file", &temp_path, source)
            })?;

        if let Err(source) = output.write_all(contents) {
            return Err(Self::cleanup_after_failure(
                "write temporary metadata file",
                path,
                &temp_path,
                MetadataError::io("write temporary metadata file", &temp_path, source),
            ));
        }
        if let Err(source) = output.flush() {
            return Err(Self::cleanup_after_failure(
                "flush temporary metadata file",
                path,
                &temp_path,
                MetadataError::io("flush temporary metadata file", &temp_path, source),
            ));
        }
        if let Err(source) = output.sync_all() {
            return Err(Self::cleanup_after_failure(
                "sync temporary metadata file",
                path,
                &temp_path,
                MetadataError::io("sync temporary metadata file", &temp_path, source),
            ));
        }
        drop(output);

        match fs::rename(&temp_path, path) {
            Ok(()) => Ok(()),
            Err(source) => Err(Self::cleanup_after_failure(
                "publish metadata file",
                path,
                &temp_path,
                MetadataError::io("publish metadata file", path, source),
            )),
        }
    }

    fn cleanup_after_failure(
        operation: &'static str,
        path: &Path,
        temp_path: &Path,
        original: MetadataError,
    ) -> MetadataError {
        match fs::remove_file(temp_path) {
            Ok(()) => original,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => original,
            Err(source) => MetadataError::transaction_rollback(
                operation,
                path,
                original,
                MetadataError::io("remove temporary metadata file", temp_path, source),
            ),
        }
    }

    fn temp_path(path: &Path) -> Result<PathBuf, MetadataError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "metadata.tmp".to_string());
        let temp_name = format!(".{file_name}.tmp.{timestamp}");
        Ok(path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(temp_name))
    }

    //Shard Resolution
    pub fn resolve_shard(&self, canonical_path: &Path) -> PathBuf {
        let parent = canonical_path.parent().unwrap_or(canonical_path);
        let digest = Sha256::digest(parent.to_string_lossy().as_bytes());
        let shard_key = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        self.shards_dir.join(format!("dir_{shard_key}.json"))
    }

    pub fn lookup_shard(&self, canonical_path: &Path) -> Result<Option<PathBuf>, MetadataError> {
        let index = self.load_index()?;
        Ok(index.map.get(canonical_path).cloned())
    }

    //Index Updating
    pub fn update_index(
        &self,
        canonical_path: PathBuf,
        shard_path: PathBuf,
    ) -> Result<(), MetadataError> {
        let mut index = self.load_index()?;
        index.map.insert(canonical_path, shard_path);
        self.save_index(&index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_module::error::MetadataError;
    use crate::test::TestDir;

    #[test]
    fn missing_index_is_treated_as_an_empty_index() {
        let temp = TestDir::new("metadata-missing-index");
        let manager = CoreMetadataManager::from_core_dir(temp.path().to_path_buf());

        let index = manager.load_index().unwrap();
        assert_eq!(index.version, 1);
        assert!(index.map.is_empty());
    }

    #[test]
    fn invalid_index_reports_corruption_and_its_path() {
        let temp = TestDir::new("metadata-invalid-index");
        let index_path = temp.path().join("index.json");
        fs::write(&index_path, b"not json").unwrap();
        let manager = CoreMetadataManager::from_core_dir(temp.path().to_path_buf());

        let error = manager.load_index().unwrap_err();
        match error {
            MetadataError::CorruptIndex { path, .. } => assert_eq!(path, index_path),
            other => panic!("expected corrupt index error, got {other:?}"),
        }
    }

    #[test]
    fn unreadable_index_reports_an_io_error_instead_of_corruption() {
        let temp = TestDir::new("metadata-index-io-error");
        let index_path = temp.path().join("index.json");
        fs::create_dir(&index_path).unwrap();
        let manager = CoreMetadataManager::from_core_dir(temp.path().to_path_buf());

        let error = manager.load_index().unwrap_err();
        match error {
            MetadataError::Io {
                operation,
                path,
                source,
            } => {
                assert_eq!(operation, "read metadata index");
                assert_eq!(path, index_path);
                assert_eq!(source.kind(), std::io::ErrorKind::IsADirectory);
            }
            other => panic!("expected metadata index io error, got {other:?}"),
        }
    }
}
