use crate::batch_module::Job;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BatchFile {
    #[serde(rename = "operations")]
    pub operations: Vec<Job>,
}
