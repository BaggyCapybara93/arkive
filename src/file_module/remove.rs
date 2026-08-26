use std::fs;
use std::path::{Path, PathBuf};

use crate::file_module::FileManager;
use crate::file_module::error::FileManagerError;

/// Options for file removal
#[derive(Debug)]
pub struct RemoveOptions {
    pub trash: bool,
    pub dry_run: bool,
    pub verbose: bool,
}

impl<'a> FileManager<'a> {
    /// Remove files based on name pattern or extension
    pub fn remove_files(
        &self,
        pattern: &str,
        extension: Option<&str>,
        options: RemoveOptions,
    ) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();
        let trash_path = if options.trash && self.settings.enable_trash {
            Some(Self::trash_path()?)
        } else {
            None
        };
        let _guard = if let Some(trash) = trash_path.as_deref() {
            Self::acquire_paths([src, trash])
        } else {
            Self::acquire_paths([src])
        };

        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Remove files requires a directory".into(),
            ));
        }

        let files_to_remove = Self::find_files_to_remove(src, pattern, extension)?;
        let progress = if !files_to_remove.is_empty() {
            FileManager::maybe_create_progress_bar(
                files_to_remove.len().max(1) as u64,
                "Removing matching files",
            )
        } else {
            None
        };

        if files_to_remove.is_empty() {
            if options.verbose {
                println!("No files matching the pattern '{}' found", pattern);
            }
            return Ok(());
        }

        if options.dry_run {
            if options.verbose {
                println!("Dry run mode - no files will be removed:");
            }
            for file in &files_to_remove {
                if options.verbose {
                    println!("  Would remove: {:?}", file);
                }
            }
            return Ok(());
        }

        // Remove files
        for file_path in &files_to_remove {
            let metadata_keys = self.metadata_keys_for_path(file_path)?;

            if options.verbose {
                println!("Removing: {:?}", file_path);
            }

            if options.trash && self.settings.enable_trash {
                let target_path = Self::unique_trash_path(file_path)?;
                fs::rename(file_path, &target_path)?;
            } else {
                // Permanently delete
                fs::remove_file(file_path)?;
            }

            self.remove_metadata_by_keys(&metadata_keys)?;

            if let Some(bar) = &progress {
                bar.inc(1);
            }
        }

        if let Some(bar) = progress {
            bar.finish_with_message(format!("Removed {} file(s)", files_to_remove.len()));
        }

        if options.verbose {
            println!("Removed {} file(s)", files_to_remove.len());
        }

        Ok(())
    }

    /// Find files matching the pattern or extension
    fn find_files_to_remove(
        dir: &Path,
        pattern: &str,
        extension: Option<&str>,
    ) -> Result<Vec<PathBuf>, FileManagerError> {
        let mut files = Vec::new();

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                let file_name = path
                    .file_name()
                    .ok_or_else(|| FileManagerError::InvalidInput("File has no name".into()))?;
                let file_name_str = file_name.to_string_lossy();

                let should_remove = if let Some(ext) = extension {
                    file_name_str.ends_with(ext)
                } else {
                    Self::matches_pattern(&file_name_str, pattern)
                };

                if should_remove {
                    files.push(path);
                }
            } else if path.is_dir() {
                // Recursively search subdirectories
                let sub_files = Self::find_files_to_remove(&path, pattern, extension)?;
                files.extend(sub_files);
            }
        }

        Ok(files)
    }

    fn matches_pattern(file_name: &str, pattern: &str) -> bool {
        Self::match_glob(file_name, pattern)
    }

    fn match_glob(file_name: &str, pattern: &str) -> bool {
        let regex_pattern = Self::glob_to_regex(pattern);
        match regex::Regex::new(&regex_pattern) {
            Ok(regex) => regex.is_match(file_name),
            Err(_) => false,
        }
    }

    fn glob_to_regex(pattern: &str) -> String {
        let mut regex = String::new();

        for c in pattern.chars() {
            match c {
                '*' => regex.push_str(".*"),
                '?' => regex.push_str("."),
                '[' => regex.push_str(r"\["),
                ']' => regex.push_str(r"\]"),
                '(' => regex.push_str(r"\("),
                ')' => regex.push_str(r"\)"),
                '{' => regex.push_str(r"\{"),
                '}' => regex.push_str(r"\}"),
                '$' => regex.push_str(r"\$"),
                '^' => regex.push_str(r"\^"),
                '+' => regex.push_str(r"\+"),
                '.' => regex.push_str(r"\."),
                '\\' => regex.push_str(r"\\\\"),
                _ => regex.push(c),
            }
        }

        regex
    }
}

#[cfg(test)]
mod tests {
    use super::{FileManager, RemoveOptions};
    use crate::settings::Settings;
    use crate::test::TestDir;

    #[test]
    fn remove_files_only_deletes_recursive_pattern_matches() {
        let temp = TestDir::new("remove-pattern");
        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(temp.path().join("root.log"), b"remove").unwrap();
        std::fs::write(nested.join("nested.log"), b"remove").unwrap();
        std::fs::write(nested.join("keep.txt"), b"keep").unwrap();

        let settings = Settings::default();
        FileManager::new(temp.path(), "", &settings)
            .remove_files(
                "*.log",
                None,
                RemoveOptions {
                    trash: false,
                    dry_run: false,
                    verbose: false,
                },
            )
            .unwrap();

        assert!(!temp.path().join("root.log").exists());
        assert!(!nested.join("nested.log").exists());
        assert_eq!(std::fs::read(nested.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn dry_run_remove_leaves_matching_files_untouched() {
        let temp = TestDir::new("remove-dry-run");
        let matching = temp.path().join("keep.log");
        std::fs::write(&matching, b"keep").unwrap();

        let settings = Settings::default();
        FileManager::new(temp.path(), "", &settings)
            .remove_files(
                "*.log",
                None,
                RemoveOptions {
                    trash: false,
                    dry_run: true,
                    verbose: false,
                },
            )
            .unwrap();

        assert_eq!(std::fs::read(matching).unwrap(), b"keep");
    }
}
