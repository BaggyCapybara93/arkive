use std::path::PathBuf;

use crate::batch_module::job::WorkType;
use crate::batch_module::{BatchError, Job};
use crate::file_module::FileManager;
use crate::file_module::manager::{normalize_lock_path, paths_overlap};

/// A read-only view of the work that a batch run will submit.
pub struct BatchPlan {
    jobs: Vec<PlannedJob>,
    overlaps: Vec<(usize, usize)>,
}

struct PlannedJob {
    index: usize,
    action: String,
    source: PathBuf,
    destination: PathBuf,
    timestamped_destination: bool,
    cleanup: Vec<&'static str>,
    cleanup_path: Option<PathBuf>,
    lock_paths: Vec<PathBuf>,
}

impl BatchPlan {
    pub fn build(jobs: &[Job]) -> Result<Self, BatchError> {
        let mut planned = Vec::with_capacity(jobs.len());
        for (offset, job) in jobs.iter().enumerate() {
            let index = offset + 1;
            job.validate()
                .map_err(|source| BatchError::Job { index, source })?;

            let source = normalize_lock_path(job.source.as_ref());
            let destination =
                normalize_lock_path(job.destination.as_deref().unwrap_or(&job.source).as_ref());
            let settings = job.settings.as_deref().copied().unwrap_or_default();
            let timestamped_destination = job.timestamp.unwrap_or(settings.use_timestamp)
                && (matches!(&job.work_type, WorkType::Copy | WorkType::Compress)
                    || matches!(&job.work_type, WorkType::Move)
                        && job.recursive.unwrap_or(settings.recursive));
            let cleanup_path = job
                .cleanup
                .as_ref()
                .and_then(|cleanup| cleanup.path.as_deref())
                .map(|path| normalize_lock_path(path.as_ref()));
            let mut cleanup_actions = Vec::new();
            let mut lock_paths = vec![source.clone(), destination.clone()];
            if let Some(path) = &cleanup_path {
                lock_paths.push(path.clone());
            }
            if let Some(cleanup) = &job.cleanup {
                if cleanup.empty_trash {
                    cleanup_actions.push("empty_trash");
                    let trash = FileManager::trash_path()
                        .map_err(|source| BatchError::Job { index, source })?;
                    lock_paths.push(normalize_lock_path(&trash));
                }
                if cleanup.deduplicate {
                    cleanup_actions.push("deduplicate");
                }
                if cleanup.scan_unused {
                    cleanup_actions.push("scan_unused");
                }
                if cleanup.scan_empty_dirs {
                    cleanup_actions.push("scan_empty_dirs");
                }
            }
            planned.push(PlannedJob {
                index,
                action: format!("{:?}", job.work_type).to_lowercase(),
                source,
                destination,
                timestamped_destination,
                cleanup: cleanup_actions,
                cleanup_path,
                lock_paths,
            });
        }

        let mut overlaps = Vec::new();
        for (left_index, left) in planned.iter().enumerate() {
            for right in planned.iter().skip(left_index + 1) {
                if left.lock_paths.iter().any(|left_path| {
                    right
                        .lock_paths
                        .iter()
                        .any(|right_path| paths_overlap(left_path, right_path))
                }) {
                    overlaps.push((left.index, right.index));
                }
            }
        }

        Ok(Self {
            jobs: planned,
            overlaps,
        })
    }

    pub fn print(&self) {
        println!("Batch plan: {} job(s)", self.jobs.len());
        for job in &self.jobs {
            println!(
                "  {}. {} {} -> {}{}",
                job.index,
                job.action,
                job.source.display(),
                job.destination.display(),
                if job.timestamped_destination {
                    " (timestamped name at run time)"
                } else {
                    ""
                }
            );
            if !job.cleanup.is_empty() {
                println!("     cleanup: {}", job.cleanup.join(", "));
                if let Some(path) = &job.cleanup_path {
                    println!("     cleanup path: {}", path.display());
                }
            }
        }
        if self.overlaps.is_empty() {
            println!("No overlapping job paths detected.");
        } else {
            println!("Overlapping job paths:");
            for (left, right) in &self.overlaps {
                println!("  jobs {left} and {right}");
            }
            println!(
                "Overlapping operations wait on path locks; batch does not guarantee their order."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BatchPlan;
    use crate::batch_module::Job;
    use crate::batch_module::job::{BatchCleanup, WorkType};

    fn job(source: &str, destination: &str) -> Job {
        Job {
            work_type: WorkType::Copy,
            source: source.into(),
            destination: Some(destination.into()),
            recursive: None,
            cleanup: None,
            compression_method: None,
            archive_format: None,
            timestamp: None,
            settings: None,
        }
    }

    #[test]
    fn detects_overlapping_destination_paths() {
        let jobs = [
            job("/tmp/source-a", "/tmp/backup"),
            job("/tmp/source-b", "/tmp/backup/child"),
            job("/tmp/source-c", "/tmp/other"),
        ];
        let plan = BatchPlan::build(&jobs).unwrap();
        assert_eq!(plan.overlaps, vec![(1, 2)]);
    }

    #[test]
    fn includes_cleanup_path_and_timestamp_effects() {
        let mut first = job("/tmp/source-a", "/tmp/backup-a");
        first.timestamp = Some(true);
        first.cleanup = Some(BatchCleanup {
            path: Some("/tmp/shared".into()),
            scan_empty_dirs: true,
            ..BatchCleanup::default()
        });
        let mut second = job("/tmp/source-b", "/tmp/backup-b");
        second.cleanup = Some(BatchCleanup {
            path: Some("/tmp/shared/child".into()),
            scan_unused: true,
            ..BatchCleanup::default()
        });

        let plan = BatchPlan::build(&[first, second]).unwrap();
        assert!(plan.jobs[0].timestamped_destination);
        assert_eq!(plan.jobs[0].cleanup, vec!["scan_empty_dirs"]);
        assert_eq!(plan.overlaps, vec![(1, 2)]);
    }
}
