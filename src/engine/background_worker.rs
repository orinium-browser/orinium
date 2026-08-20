use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// A generic background worker that accepts commands of type `C` and returns
/// results of type `R` via a command/response channel pair.
///
/// One or more worker threads share a single command channel via
/// `Arc<Mutex<Receiver<C>>>` (competing-consumer pattern). An idle worker
/// automatically picks up the next available command.
///
/// # Type parameters
///
/// * `C` – command type sent from the UI thread to the worker(s)
/// * `R` – result type sent back from the worker(s) to the UI thread
pub struct BackgroundWorker<C, R> {
    cmd_tx: Sender<C>,
    result_rx: Receiver<R>,
}

impl<C, R> std::fmt::Debug for BackgroundWorker<C, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundWorker").finish()
    }
}

impl<C: Send + 'static, R: Send + 'static> BackgroundWorker<C, R> {
    /// Spawns `worker_count` threads, each running `process(cmd) -> result` in
    /// a loop until the command channel disconnects.
    pub fn new<F>(worker_count: usize, process: F) -> Self
    where
        F: Fn(C) -> R + Send + Sync + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel::<C>();
        let (result_tx, result_rx) = mpsc::channel::<R>();
        let cmd_rx = Arc::new(Mutex::new(cmd_rx));

        let process = Arc::new(process);
        for _ in 0..worker_count {
            let cmd_rx = Arc::clone(&cmd_rx);
            let result_tx = result_tx.clone();
            let process = Arc::clone(&process);
            thread::spawn(move || {
                loop {
                    let cmd = match cmd_rx.lock().unwrap().recv() {
                        Ok(cmd) => cmd,
                        Err(_) => break,
                    };
                    let result = process(cmd);
                    let _ = result_tx.send(result);
                }
            });
        }

        Self { cmd_tx, result_rx }
    }

    /// Sends a command to the worker pool. The command is delivered to whichever
    /// worker thread acquires the lock first.
    pub fn send(&self, cmd: C) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Returns a completed result, or `None` if none is ready yet.
    /// This never blocks the calling thread.
    pub fn try_receive(&self) -> Option<R> {
        self.result_rx.try_recv().ok()
    }
}
