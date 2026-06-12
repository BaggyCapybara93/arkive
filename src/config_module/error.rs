use thiserror::Error;

/// Errors related to configuration management
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to determine config path
    #[error("Failed to determine config path: {0}")]
    PathError(std::io::Error),
    
    /// Failed to read config file
    #[error("Failed to read config file: {0}")]
    ReadError(std::io::Error),
    
    /// Failed to parse config file as JSON
    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] serde_json::Error),
    
    /// Failed to write config file
    #[error("Failed to write config file: {0}")]
    WriteError(std::io::Error),
    
    /// Config file not found
    #[error("Config file not found at: {0}")]
    NotFound(String),
    
    /// Failed to create default config
    #[error("Failed to create default config: {0}")]
    CreateError(std::io::Error),
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::ReadError(err)
    }
}
