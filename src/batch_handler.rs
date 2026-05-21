use serde::Deserialize;
use serde_json;
use crate::file_manager::FileManager;
use std::fs;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkType {
    Move,
    Copy,
    Compress,
}

#[derive(Debug, Deserialize)]
pub struct Job {
    pub work_type: WorkType,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
}

impl Job {
    pub fn execute(&self) -> std::io::Result<()> {
        let recursive = self.recursive.unwrap_or(false);
        let dest = self.destination.clone().unwrap_or_else(|| self.source.clone());
        let fm = FileManager::new(self.source.clone(), dest);

        match self.work_type {
            WorkType::Move => {
                if recursive {
                    fm.copy_path(true)?;
                    fm.delete_path(true)?;
                } else {
                    fm.move_path()?;
                }
            }
            WorkType::Copy => fm.copy_path(recursive)?,
            WorkType::Compress => fm.compress_path()?,
        }
        Ok(())
    }
}

pub struct BatchHandler {
    pub commands: Vec<Job>,
}

impl BatchHandler {
    pub fn new(commands: Vec<Job>) -> Self{
        BatchHandler { commands }
    }

    pub fn run(&self) {
        for job in &self.commands {
            println!("Running job: {:?} -> {:?}", job.work_type, job.destination);
            if let Err(e) = job.execute() {
                eprintln!("Error executing job ({:?} from {}): {}", 
                job.work_type, job.source, e);
            }
        }
    }

    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let data = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read batch file {}: {}", path, e))?;

        let commands: Vec<Job> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse batch file {}: {}", path, e))?;

        Ok(Self::new(commands))
    }
}