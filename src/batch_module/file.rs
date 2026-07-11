use serde::Deserialize;
use crate::batch_module::{Job};

#[derive(Debug, Deserialize)]
pub struct BatchFile {
    #[serde(rename = "operations")]
    pub operations: Vec<Job>,
}