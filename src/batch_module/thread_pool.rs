use crate::batch_module::{BatchError, Job};
use crate::file_module::FileManagerError;
use indicatif::ProgressBar;
use std::sync::Arc;
use std::{
    sync::{Mutex, mpsc},
    thread,
};

pub enum ThreadMessage {
    Job { index: usize, job: Job },
    Shutdown,
}

pub enum ThreadResult {
    Ok,
    Err {
        index: usize,
        source: FileManagerError,
    },
}

pub struct Worker {
    pub handle: thread::JoinHandle<()>,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<ThreadMessage>,
    result_receiver: mpsc::Receiver<ThreadResult>,
}

impl ThreadPool {
    /// `progress` is the shared batch-level bar (or None if there are no jobs).
    /// It's captured once here and cloned into every worker thread, so each
    /// worker can call `.inc(1)` on the SAME underlying bar the instant its
    /// own job finishes.
    pub fn new(size: usize, progress: Option<ProgressBar>) -> Self {
        let (sender, receiver) = mpsc::channel::<ThreadMessage>();
        let (result_sender, result_receiver) = mpsc::channel::<ThreadResult>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let receiver = Arc::clone(&receiver);
            let result_sender = result_sender.clone();
            // ProgressBar is internally Arc-backed, so this clone is cheap and
            // all workers end up incrementing the same visible bar.
            let progress = progress.clone();

            let handle = thread::spawn(move || {
                loop {
                    let message = match receiver.lock() {
                        Ok(guard) => guard.recv(),
                        Err(e) => {
                            eprintln!("Worker {id} failed to lock receiver: {e}");
                            break;
                        }
                    };

                    match message {
                        Ok(ThreadMessage::Job { index, job }) => {
                            match job.execute(progress.as_ref()) {
                                Ok(_) => {
                                    let _ = result_sender.send(ThreadResult::Ok);
                                }
                                Err(source) => {
                                    let _ = result_sender.send(ThreadResult::Err { index, source });
                                }
                            }
                        }

                        Ok(ThreadMessage::Shutdown) => break,
                        Err(_) => break,
                    }
                }
            });

            workers.push(Worker { handle });
        }

        ThreadPool {
            workers,
            sender,
            result_receiver,
        }
    }

    pub fn join(self) -> Result<(), BatchError> {
        for _ in &self.workers {
            let _ = self.sender.send(ThreadMessage::Shutdown);
        }

        let mut worker_panicked = false;
        for worker in self.workers {
            worker_panicked |= worker.handle.join().is_err();
        }

        let mut first_failure: Option<(usize, FileManagerError)> = None;
        while let Ok(result) = self.result_receiver.recv() {
            if let ThreadResult::Err { index, source } = result {
                if first_failure
                    .as_ref()
                    .is_none_or(|(first_index, _)| index < *first_index)
                {
                    first_failure = Some((index, source));
                }
            }
        }

        if worker_panicked {
            Err(BatchError::ThreadPool("Worker thread panicked".into()))
        } else if let Some((index, source)) = first_failure {
            Err(BatchError::Job { index, source })
        } else {
            Ok(())
        }
    }

    pub fn add_job(&self, index: usize, job: Job) -> Result<(), BatchError> {
        self.sender
            .send(ThreadMessage::Job { index, job })
            .map_err(|e| BatchError::ThreadPool(e.to_string()))
    }
}
