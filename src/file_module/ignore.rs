use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::file_module::error::FileManagerError;

#[derive(Clone, Debug, Default)]
pub struct IgnoreOptions {
    pub no_global: bool,
    pub no_local: bool,
    pub files: Vec<PathBuf>,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub exclude_larger_than: Option<String>,
}

#[derive(Debug, Default)]
pub struct IgnoreStats {
    pub entries: u64,
    pub bytes: u64,
}

impl IgnoreStats {
    pub fn record(&mut self, path: &Path) -> Result<(), FileManagerError> {
        let metadata = fs::symlink_metadata(path)?;
        self.entries += 1;
        if metadata.is_file() {
            self.bytes += metadata.len();
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                self.record(&entry?.path())?;
            }
        }
        Ok(())
    }
}

enum Rule {
    Pattern { regex: Regex, exclude: bool },
    Size { regex: Option<Regex>, bytes: u64 },
}

pub struct IgnoreMatcher {
    root: PathBuf,
    rules: Vec<Rule>,
    descriptions: Vec<String>,
}

impl IgnoreMatcher {
    pub fn build(root: &Path, options: &IgnoreOptions) -> Result<Self, FileManagerError> {
        let root = if root.is_dir() {
            fs::canonicalize(root)?
        } else {
            fs::canonicalize(root.parent().unwrap_or_else(|| Path::new(".")))?
        };
        let mut matcher = Self {
            root,
            rules: Vec::new(),
            descriptions: Vec::new(),
        };

        if !options.no_global
            && let Some(path) = global_ignore_path()
        {
            matcher.add_file(&path, false)?;
        }
        if !options.no_local {
            matcher.add_file(&matcher.root.join(".arkiveignore"), false)?;
        }
        for path in &options.files {
            matcher.add_file(path, true)?;
        }
        for pattern in &options.excludes {
            matcher.add_line(pattern, "--exclude")?;
        }
        if let Some(size) = &options.exclude_larger_than {
            let bytes = parse_size(size)?;
            matcher.rules.push(Rule::Size { regex: None, bytes });
            matcher
                .descriptions
                .push(format!("--exclude-larger-than {size}"));
        }
        // Includes are intentionally last so command-line includes have the
        // highest precedence over global, local, and explicit exclusions.
        for pattern in &options.includes {
            matcher.add_line(&format!("!{pattern}"), "--include")?;
        }
        Ok(matcher)
    }

    pub fn is_excluded(&self, path: &Path, is_dir: bool, size: u64) -> bool {
        self.decision(path, is_dir, size).0
    }

    pub fn decision(&self, path: &Path, is_dir: bool, size: u64) -> (bool, Option<&str>) {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        let value = relative.to_string_lossy().replace('\\', "/");
        let mut excluded = false;
        let mut reason = None;
        for (rule, description) in self.rules.iter().zip(&self.descriptions) {
            match rule {
                Rule::Pattern {
                    regex,
                    exclude: value_to_set,
                } if regex.is_match(&value) => {
                    excluded = *value_to_set;
                    reason = Some(description.as_str());
                }
                Rule::Size { regex, bytes }
                    if !is_dir
                        && size > *bytes
                        && regex.as_ref().is_none_or(|regex| regex.is_match(&value)) =>
                {
                    excluded = true;
                    reason = Some(description.as_str());
                }
                _ => {}
            }
        }
        (excluded, reason)
    }

    pub fn descriptions(&self) -> &[String] {
        &self.descriptions
    }

    /// Remove only items that would have been included in a filtered move.
    /// Excluded directories and files remain in place.
    pub fn remove_included_sources(&self, directory: &Path) -> Result<(), FileManagerError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let metadata = entry.metadata()?;
            if self.is_excluded(&path, file_type.is_dir(), metadata.len()) {
                continue;
            }
            if path == self.root.join(".arkiveignore") {
                continue;
            }
            if file_type.is_dir() {
                self.remove_included_sources(&path)?;
                if fs::read_dir(&path)?.next().is_none() {
                    fs::remove_dir(&path)?;
                }
            } else {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn add_file(&mut self, path: &Path, required: bool) -> Result<(), FileManagerError> {
        if !path.exists() {
            if required {
                return Err(FileManagerError::InvalidInput(format!(
                    "Ignore file does not exist: {:?}",
                    path
                )));
            }
            return Ok(());
        }
        let contents = fs::read_to_string(path)?;
        for (index, line) in contents.lines().enumerate() {
            self.add_line(line, &format!("{}:{}", path.display(), index + 1))?;
        }
        Ok(())
    }

    fn add_line(&mut self, raw: &str, source: &str) -> Result<(), FileManagerError> {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            return Ok(());
        }

        if let Some((scope, size)) = parse_size_rule(line) {
            let regex = scope.map(glob_regex).transpose()?;
            let bytes = parse_size(size)?;
            self.rules.push(Rule::Size { regex, bytes });
            self.descriptions.push(format!("{source}: {line}"));
            return Ok(());
        }

        let (exclude, pattern) = match line.strip_prefix('!') {
            Some(pattern) => (false, pattern),
            None => (true, line),
        };
        if pattern.is_empty() {
            return Err(FileManagerError::InvalidInput(format!(
                "Empty ignore pattern in {source}"
            )));
        }
        self.rules.push(Rule::Pattern {
            regex: glob_regex(pattern)?,
            exclude,
        });
        self.descriptions.push(format!("{source}: {line}"));
        Ok(())
    }
}

fn global_ignore_path() -> Option<PathBuf> {
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config).join("arkive/ignore"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/arkive/ignore"))
}

fn parse_size_rule(line: &str) -> Option<(Option<&str>, &str)> {
    let (scope, expression) = if let Some(expression) = line.strip_prefix(":size") {
        (None, expression)
    } else {
        let (scope, expression) = line.rsplit_once(" :size")?;
        (Some(scope.trim()), expression)
    };
    let size = expression.trim().strip_prefix('>')?.trim();
    Some((scope, size))
}

fn parse_size(value: &str) -> Result<u64, FileManagerError> {
    let value = value.trim().to_ascii_uppercase();
    let split = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let number: f64 = value[..split]
        .parse()
        .map_err(|_| FileManagerError::InvalidInput(format!("Invalid size: {value}")))?;
    let multiplier = match value[split..].trim() {
        "" | "B" => 1f64,
        "KB" | "KIB" => 1024f64,
        "MB" | "MIB" => 1024f64.powi(2),
        "GB" | "GIB" => 1024f64.powi(3),
        "TB" | "TIB" => 1024f64.powi(4),
        unit => {
            return Err(FileManagerError::InvalidInput(format!(
                "Unsupported size unit: {unit}"
            )));
        }
    };
    Ok((number * multiplier) as u64)
}

fn glob_regex(pattern: &str) -> Result<Regex, FileManagerError> {
    let directory = pattern.ends_with('/');
    let mut pattern = pattern.trim_end_matches('/');
    let anchored = pattern.starts_with('/');
    pattern = pattern.trim_start_matches('/');
    let has_slash = pattern.contains('/');
    let mut output = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    output.push_str("(?:.*/)?");
                } else {
                    output.push_str(".*");
                }
            }
            '*' => output.push_str("[^/]*"),
            '?' => output.push_str("[^/]"),
            other => output.push_str(&regex::escape(&other.to_string())),
        }
    }
    let prefix = if anchored || has_slash {
        "^"
    } else {
        "(?:^|/)"
    };
    let suffix = if directory { "(?:/.*)?$" } else { "$" };
    Regex::new(&format!("{prefix}{output}{suffix}"))
        .map_err(|error| FileManagerError::InvalidInput(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{glob_regex, parse_size};

    #[test]
    fn gitignore_style_globs_match_expected_paths() {
        assert!(glob_regex("*.log").unwrap().is_match("logs/app.log"));
        assert!(glob_regex("/target/").unwrap().is_match("target/debug/app"));
        assert!(
            !glob_regex("/target/")
                .unwrap()
                .is_match("nested/target/app")
        );
        assert!(glob_regex("**/cache/").unwrap().is_match("a/b/cache/item"));
    }

    #[test]
    fn human_sizes_use_binary_units() {
        assert_eq!(parse_size("1.5MB").unwrap(), 1_572_864);
    }
}
