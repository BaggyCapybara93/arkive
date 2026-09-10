use thiserror::Error;

/// Errors related to configuration management
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to determine config path
    #[error("Failed to determine config path: {0}")]
    Path(std::io::Error),

    /// Failed to read config file
    #[error("Failed to read config file: {0}")]
    Read(std::io::Error),

    /// Failed to parse config file as JSON
    #[error("Failed to parse config file: {0}")]
    Parse(#[from] serde_json::Error),

    /// Failed to write config file
    #[error("Failed to write config file: {0}")]
    Write(std::io::Error),
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::Read(err)
    }
}
