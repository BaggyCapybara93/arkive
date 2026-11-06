use crate::file_module::FileManagerError;

#[derive(thiserror::Error, Debug)]
pub enum BatchError {
    #[error("Failed to read batch file: {0}")]
    Read(#[from] std::io::Error),

    #[error("Failed to parse batch file: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Batch job {index} failed: {source}")]
    Job {
        index: usize,
        #[source]
        source: FileManagerError,
    },

    #[error("Thread pool error: {0}")]
    ThreadPool(String),
}
