pub mod copy;
pub mod dedup;
pub mod error;
pub mod manager;
pub mod ops;
pub mod trash;

pub use error::FileManagerError;
pub use manager::FileManager;
