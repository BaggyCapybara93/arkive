use serde::Deserialize;
use serde_json;
use crate::file_manager::FileManager;
use std::fs;

#[derive(Debug, Deserialize)]
pub struct Job {
    pub work_type: String,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
}

impl Job {
    pub fn new(work_type: String, source: String, destination: Option<String>, recursive: Option<bool>) -> Self {
        Job { work_type, source, destination, recursive }
    }

    pub fn execute(&self) -> std::io::Result<()> {
        let recursive = self.recursive.unwrap_or(false);
        let dest = self.destination.clone().unwrap_or_else(|| self.source.clone());
        let fm = FileManager::new(self.source.clone(), dest);

        match self.work_type.as_str() {
            "move" => {
                if recursive {
                    fm.copy_path(true)?;
                    fm.delete_path(true)?;
                } else {
                    fm.move_path()?;
                }
            }
            "copy" => fm.copy_path(recursive)?,
            "compress" => fm.compress_path()?,
            _ => eprintln!("Unknown action: {}", self.work_type),
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
            println!("Running job: {} -> {:?}", job.work_type, job.destination);
            if let Err(e) = job.execute() {
                eprintln!("Error executing job ({} from {}): {}", 
    job.work_type, job.source, e);
            }
        }
    }

    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path).with_context(|| format!("Failed to read batch file: {}", path))?;
        let commands: Vec<Job> = serde_json::from_str(&data).with_context(|| format!("Failed to parse batch file: {}", path))?;
        Ok(Self::new(commands))
    }
}