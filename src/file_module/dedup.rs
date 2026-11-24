use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_module::error::FileManagerError;
use crate::file_module::manager::FileManager;
use crate::file_validation::hash::hash_file;

#[derive(Debug, PartialEq, Eq)]
struct Removal {
    path: PathBuf,
    retained: PathBuf,
    size: u64,
}

fn collect_files(
    dir: &Path,
    groups: &mut BTreeMap<u64, Vec<PathBuf>>,
) -> Result<(), FileManagerError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&entry.path(), groups)?;
        } else if file_type.is_file() {
            groups
                .entry(entry.metadata()?.len())
                .or_default()
                .push(entry.path());
        }
    }
    Ok(())
}

// Sort paths before hashing so the first matching path is always the retained copy.
fn build_plan(
    dir: &Path,
    mut hash: impl FnMut(&Path) -> Result<String, FileManagerError>,
) -> Result<Vec<Removal>, FileManagerError> {
    let mut groups = BTreeMap::new();
    collect_files(dir, &mut groups)?;
    let candidates: u64 = groups
        .values()
        .filter(|files| files.len() > 1)
        .map(|files| files.len() as u64)
        .sum();
    let progress =
        FileManager::maybe_create_progress_bar(candidates.max(1), "Hashing duplicate candidates");
    let mut plan = Vec::new();
    for (size, mut files) in groups {
        if files.len() < 2 {
            continue;
        }
        files.sort();
        let mut seen: HashMap<String, PathBuf> = HashMap::new();
        for path in files {
            let digest = hash(&path)?;
            if let Some(retained) = seen.get(&digest) {
                plan.push(Removal {
                    path,
                    retained: retained.clone(),
                    size,
                });
            } else {
                seen.insert(digest, path);
            }
            if let Some(bar) = &progress {
                bar.inc(1);
            }
        }
    }
    if let Some(bar) = progress {
        bar.finish_with_message("Duplicate scan complete");
    }
    plan.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(plan)
}

impl<'a> FileManager<'a> {
    /// Plan and remove duplicate files recursively, retaining the smallest path in each group.
    pub fn folder_deduplication(&self, to_trash: bool) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();
        crate::file_validation::handlers::valid_directory(src)?;
        let plan = build_plan(src, |path| {
            let path = path
                .to_str()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid UTF-8 file path".into()))?;
            Ok(hash_file(path)?)
        })?;
        let bytes: u128 = plan.iter().map(|entry| u128::from(entry.size)).sum();
        let trash = to_trash && self.settings.enable_trash;
        println!("Duplicate removal plan ({} file(s)):", plan.len());
        for entry in &plan {
            println!(
                "  Remove {:?} ({} bytes); retain {:?}",
                entry.path, entry.size, entry.retained
            );
        }
        println!("Estimated logical space savings: {} bytes", bytes);
        if trash && !plan.is_empty() {
            println!("Files moved to trash still occupy space until trash is emptied.");
        }
        if !self.settings.dry_run {
            for entry in plan {
                self.delete_path(entry.path, false, to_trash)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FileManager;
    use crate::settings::Settings;
    use crate::test::TestDir;

    #[test]
    fn plan_hashes_only_shared_sizes_and_selects_paths_deterministically() {
        use super::build_plan;
        let temp = TestDir::new("dedup-plan");
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        for name in ["z", "nested/b", "a"] {
            std::fs::write(temp.path().join(name), b"same").unwrap();
        }
        std::fs::write(temp.path().join("different"), b"diff").unwrap();
        std::fs::write(temp.path().join("unique"), b"unique-size").unwrap();
        let mut hashed = Vec::new();
        let plan = build_plan(temp.path(), |path| {
            hashed.push(path.to_path_buf());
            Ok(crate::file_validation::hash::hash_file(
                path.to_str().unwrap(),
            )?)
        })
        .unwrap();
        assert_eq!(hashed.len(), 4);
        assert!(!hashed.contains(&temp.path().join("unique")));
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].path, temp.path().join("nested/b"));
        assert_eq!(plan[1].path, temp.path().join("z"));
        assert!(
            plan.iter()
                .all(|item| item.retained == temp.path().join("a") && item.size == 4)
        );
        assert_eq!(plan.iter().map(|item| item.size).sum::<u64>(), 8);
    }

    #[test]
    fn dry_run_preserves_files_and_real_run_keeps_smallest_path() {
        let temp = TestDir::new("dedup-execute");
        let first = temp.path().join("a");
        let last = temp.path().join("z");
        std::fs::write(&last, b"same").unwrap();
        std::fs::write(&first, b"same").unwrap();
        let mut settings = Settings::default();
        settings.dry_run = true;
        FileManager::new(temp.path(), "", &settings)
            .folder_deduplication(false)
            .unwrap();
        assert!(first.exists() && last.exists());
        settings.dry_run = false;
        FileManager::new(temp.path(), "", &settings)
            .folder_deduplication(false)
            .unwrap();
        assert_eq!(std::fs::read(first).unwrap(), b"same");
        assert!(!last.exists());
    }

    #[cfg(unix)]
    #[test]
    fn deduplication_skips_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("dedup-symlink");
        let source = temp.path().join("source");
        let external = temp.path().join("external");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::fs::write(source.join("original.txt"), b"same").unwrap();
        std::fs::write(external.join("outside.txt"), b"same").unwrap();
        symlink(&external, source.join("linked-external")).unwrap();

        let settings = Settings::default();
        FileManager::new(&source, "", &settings)
            .folder_deduplication(false)
            .unwrap();

        assert_eq!(
            std::fs::read(external.join("outside.txt")).unwrap(),
            b"same"
        );
    }
}
