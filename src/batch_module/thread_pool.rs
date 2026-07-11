use std::sync::Arc;
use std::{
    sync::{mpsc, Mutex},
    thread,
};
use crate::batch_module::{Job, BatchError};

pub enum ThreadMessage {
    Job(Job),
    Shutdown,
}

pub enum ThreadResult {
    Ok,
    Err(String),
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
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<ThreadMessage>();
        let (result_sender, result_receiver) = mpsc::channel::<ThreadResult>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let receiver = Arc::clone(&receiver);
            let result_sender = result_sender.clone();

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
                        Ok(ThreadMessage::Job(job)) => {
                            match job.execute() {
                                Ok(_) => {
                                    let _ = result_sender.send(ThreadResult::Ok);
                                }
                                Err(e) => {
                                    let _ = result_sender.send(ThreadResult::Err(format!("Worker {id} failed to execute job: {e}")));
                                }
                            }
                        }

                        Ok (ThreadMessage::Shutdown) => break,
                        Err(_) => break,
                    }
                }
            });

            workers.push(Worker { handle });
        }

        ThreadPool { workers, sender, result_receiver }
    }

    pub fn join(self) -> Result<(), BatchError> {
        for _ in &self.workers {
            let _ = self.sender.send(ThreadMessage::Shutdown);
        }

        for worker in self.workers {
            worker.handle.join()
                .map_err(|_| BatchError::ThreadPool("Worker thread panicked".into()))?;
        }

        while let Ok(result) = self.result_receiver.recv() {
            if let ThreadResult::Err(e) = result {
                return Err(BatchError::Worker(e));
            }
        }

        Ok(())
    }

    pub fn add_job(&self, job: Job) -> Result<(), BatchError> {
        self.sender.send(ThreadMessage::Job(job))
                .map_err(|e| BatchError::ThreadPool(e.to_string()))
    }

}