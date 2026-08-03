use std::fs;
use std::path::{Path, PathBuf};

use crate::file_module::error::FileManagerError;
use crate::file_module::FileManager;

impl<'a> FileManager<'a> {
    /// Rename a file or directory to the destination.
    pub fn rename_path(&self) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if self.settings.dry_run {
            if self.settings.verbose {
                println!("[DRY-RUN] Would rename {:?} to {:?}", src, dst);
            }
            return Ok(());
        }

        let dest_path = Self::canonical_destination_file(src, dst)?;
        std::fs::rename(src, &dest_path)?;

        if self.settings.enable_metadata {
            let manager = self.metadata_manager_for_destination(&dest_path)?;

            if dest_path.is_file() {
                self.save_metadata_for_file(&dest_path, &manager)?;
            } else if dest_path.is_dir() {
                self.save_metadata_for_directory(src, &dest_path, &manager)?;
            }

            let source_paths = if self.settings.enable_metadata && src.is_dir() {
                Some(self.collect_file_paths(src)?)
            } else {
                None
            };

            if let Some(paths) = source_paths {
                for old_path in paths {
                    self.remove_metadata_for_file(&old_path)?;
                }
            } else if src.is_file() {
                self.remove_metadata_for_file(src)?;
            }
        }

        Ok(())
    }

    pub fn rename_matching_items(
        &self,
        pattern: Option<&str>,
        extension: Option<&str>,
        recursive: bool,
        template: &str,
    ) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
        let root = self.file_path.as_path();

        if !root.exists() {
            return Err(FileManagerError::InvalidDirectory(format!("Path does not exist: {:?}", root)));
        }

        if !root.is_dir() {
            return Err(FileManagerError::InvalidDirectory(format!("Path is not a directory: {:?}", root)));
        }

        let mut matched_paths = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if Self::matches_rename_target(&file_name, pattern, extension)? {
                matched_paths.push(path);
            }
        }

        matched_paths.sort();

        for path in matched_paths {
            let file_name = path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| FileManagerError::InvalidInput(format!("Invalid file name: {:?}", path)))?;

            let new_name = Self::build_renamed_name(file_name, template)?;
            let new_path = path.parent().unwrap_or(root).join(&new_name);

            if path == new_path {
                continue;
            }

            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would rename {:?} to {:?}", path, new_path);
                }
                continue;
            }

            if new_path.exists() {
                return Err(FileManagerError::InvalidInput(format!("Target already exists: {:?}", new_path)));
            }

            fs::rename(&path, &new_path)?;

            if self.settings.verbose {
                println!("Renamed {:?} to {:?}", path, new_path);
            }
        }

        if recursive {
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let child = FileManager::new(&path, PathBuf::new(), self.settings);
                    child.rename_matching_items(pattern, extension, true, template)?;
                }
            }
        }

        Ok(())
    }

    fn matches_rename_target(file_name: &str, pattern: Option<&str>, extension: Option<&str>) -> Result<bool, FileManagerError> {
        if let Some(pattern_value) = pattern {
            return Ok(Self::matches_rename_pattern(file_name, pattern_value));
        }

        if let Some(extension_value) = extension {
            let normalized = if extension_value.starts_with('.') {
                extension_value.to_string()
            } else {
                format!(".{extension_value}")
            };
            return Ok(file_name.ends_with(&normalized));
        }

        Ok(false)
    }

    fn matches_rename_pattern(file_name: &str, pattern: &str) -> bool {
        let regex_pattern = Self::rename_glob_to_regex(pattern);
        regex::Regex::new(&regex_pattern).map(|regex| regex.is_match(file_name)).unwrap_or(false)
    }

    fn rename_glob_to_regex(pattern: &str) -> String {
        let mut regex = String::from("^");
        let mut chars = pattern.chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                '*' => regex.push_str(".*"),
                '?' => regex.push('.'),
                '.' => regex.push_str("\\."),
                '+' => regex.push_str("\\+"),
                '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\' => {
                    regex.push('\\');
                    regex.push(ch);
                }
                _ => regex.push(ch),
            }
        }

        regex.push('$');
        regex
    }

    fn build_renamed_name(file_name: &str, template: &str) -> Result<String, FileManagerError> {
        let path = Path::new(file_name);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_string();
        let mut result = template.to_string();

        result = result.replace("{name}", &stem);
        result = result.replace("{ext}", &extension);
        result = result.replace("{original}", file_name);

        if !extension.is_empty() && !result.contains('.') {
            result.push('.');
            result.push_str(&extension);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::FileManager;

    #[test]
    fn pattern_matching_supports_globs() {
        assert!(FileManager::matches_rename_pattern("notes.txt", "*.txt"));
        assert!(FileManager::matches_rename_pattern("archive.tar.gz", "*.gz"));
        assert!(!FileManager::matches_rename_pattern("notes.txt", "*.md"));
    }

    #[test]
    fn rename_templates_preserve_extension_when_missing() {
        let renamed = FileManager::build_renamed_name("notes.txt", "prefix-{name}").unwrap();
        assert_eq!(renamed, "prefix-notes.txt");
    }
}