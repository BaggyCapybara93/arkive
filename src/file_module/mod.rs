pub mod cleanup;
pub mod compress;
pub mod copy;
pub mod metadata;
pub mod dedup;
pub mod error;
pub mod manager;
pub mod rename;
pub mod ops;
pub mod remove;
pub mod trash;

pub use error::FileManagerError;
pub use manager::FileManager;
