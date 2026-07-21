use std::path::PathBuf;
use std::env;

use crate::config_module::{Config, ConfigError};

/// Manager for loading and saving configuration
pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    /// Create a new ConfigManager
    /// Config is stored in the same directory as the executable
    pub fn new() -> Result<Self, ConfigError> {
        let exe_dir = env::current_exe()
            .map_err(ConfigError::PathError)?
            .parent()
            .ok_or_else(|| ConfigError::PathError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Executable has no parent directory",
            )))?
            .to_path_buf();
        
        let config_path = exe_dir.join("config.json");
        
        Ok(ConfigManager { config_path })
    }
    
    /// Load configuration from file
    pub fn load(&self) -> Result<Config, ConfigError> {
        if self.config_path.exists() {
            let content = std::fs::read_to_string(&self.config_path)
                .map_err(ConfigError::ReadError)?;
            
            let config = serde_json::from_str(&content)
                .map_err(ConfigError::ParseError)?;
            
            return Ok(config);
        }
        
        // Return default config if file doesn't exist
        Ok(Config::default())
    }
    
    /// Save configuration to file
    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        // Update the updated_at timestamp
        let mut config_to_save = config.clone();
        config_to_save.updated_at = chrono::Utc::now();
        
        let content = serde_json::to_string_pretty(&config_to_save)
            .map_err(|e| ConfigError::WriteError(std::io::Error::new(
                std::io::ErrorKind::Other,
                e,
            )))?;
        
        std::fs::write(&self.config_path, content)
            .map_err(ConfigError::WriteError)?;
        
        Ok(())
    }
    
    /// Create a default config file if it doesn't exist
    pub fn create_default_config(&self) -> Result<(), ConfigError> {
        if !self.config_path.exists() {
            let config = Config::default();
            self.save(&config)?;
        }
        Ok(())
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            let temp_dir = std::env::temp_dir();
            let config_path = temp_dir.join("arkive_config.json");
            ConfigManager { config_path }
        })
    }
}
