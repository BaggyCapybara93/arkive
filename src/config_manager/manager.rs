use std::path::PathBuf;
use std::env;

use crate::config_manager::Config;

/// Manager for loading and saving configuration
pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    /// Create a new ConfigManager
    /// Config is stored in the same directory as the executable
    pub fn new() -> Self {
        let exe_dir = env::current_exe()
            .expect("Could not determine executable directory")
            .parent()
            .expect("Executable has no parent")
            .to_path_buf();
        
        let config_path = exe_dir.join("config.json");
        
        ConfigManager { config_path }
    }
    
    /// Load configuration from file
    pub fn load(&self) -> Config {
        if self.config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.config_path) {
                if let Ok(config) = serde_json::from_str(&content) {
                    return config;
                }
            }
        }
        
        // Return default config if file doesn't exist or is invalid
        Config::default()
    }
    
    /// Save configuration to file
    pub fn save(&self, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        // Update the updated_at timestamp
        let mut config_to_save = config.clone();
        config_to_save.updated_at = chrono::Utc::now();
        
        let content = serde_json::to_string_pretty(&config_to_save)?;
        std::fs::write(&self.config_path, content)?;
        
        Ok(())
    }
    
    /// Create a default config file if it doesn't exist
    pub fn create_default_config(&self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.config_path.exists() {
            let config = Config::default();
            self.save(&config)?;
        }
        Ok(())
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}
