use crate::file_module::compress::CompressionMethod;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for long-term storage of settings.
/// This is separate from Settings which holds mostly short-term CLI settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Enable trash by default when deleting files
    #[serde(default = "default_true")]
    pub enable_trash: bool,

    /// Enable verbose output by default
    #[serde(default)]
    pub verbose: bool,

    /// Enable dry-run mode by default
    #[serde(default)]
    pub dry_run: bool,

    /// Enable recursive operations by default
    #[serde(default)]
    pub recursive: bool,

    /// Enable metadata tracking when moving or copying files
    #[serde(default)]
    pub enable_metadata: bool,

    /// Default compression method for archive operations
    #[serde(default = "default_gzip")]
    pub compression_method: CompressionMethod,

    /// Enable timestamp prefix for copy and compression operations
    #[serde(default)]
    pub use_timestamp: bool,

    /// Timestamp when config was created (ISO 8601 format)
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,

    /// Timestamp when config was last updated (ISO 8601 format)
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

fn default_gzip() -> CompressionMethod {
    CompressionMethod::Gzip
}

/// Default to true for boolean fields that should be enabled by default
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enable_trash: true,
            verbose: false,
            dry_run: false,
            recursive: false,
            enable_metadata: false,
            compression_method: CompressionMethod::Gzip,
            use_timestamp: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}
