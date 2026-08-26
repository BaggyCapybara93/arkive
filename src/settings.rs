use crate::file_module::compress::CompressionMethod;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub enable_trash: bool,
    pub verbose: bool,
    pub recursive: bool,
    pub dry_run: bool,
    pub enable_metadata: bool,
    pub compression_method: CompressionMethod,
    pub use_timestamp: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enable_trash: true,
            verbose: false,
            recursive: false,
            dry_run: false,
            enable_metadata: false, //This wont be enabled through the cli just through the config for now
            compression_method: CompressionMethod::Gzip,
            use_timestamp: false,
        }
    }
}
