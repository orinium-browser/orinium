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
        Self::new_with_init(worker_count, || (), move |(), cmd| process(cmd))
    }

    /// Like [`BackgroundWorker::new`], but each worker thread first runs
    /// `init()` once and passes its value to every `process(ctx, cmd)` call.
    ///
    /// Use this when workers need expensive per-thread state (e.g. an async
    /// runtime) that must not be rebuilt per command. The context never
    /// leaves the thread that created it, so `T` may be `!Send`.
    pub fn new_with_init<T, F, G>(worker_count: usize, init: F, process: G) -> Self
    where
        T: 'static,
        F: Fn() -> T + Send + Sync + 'static,
        G: Fn(&T, C) -> R + Send + Sync + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel::<C>();
        let (result_tx, result_rx) = mpsc::channel::<R>();
        let cmd_rx = Arc::new(Mutex::new(cmd_rx));

        let init = Arc::new(init);
        let process = Arc::new(process);
        for _ in 0..worker_count {
            let cmd_rx = Arc::clone(&cmd_rx);
            let result_tx = result_tx.clone();
            let init = Arc::clone(&init);
            let process = Arc::clone(&process);
            thread::spawn(move || {
                let ctx = init();
                loop {
                    let cmd = match cmd_rx.lock().unwrap().recv() {
                        Ok(cmd) => cmd,
                        Err(_) => {
                            log::debug!("BackgroundWorker: worker exiting, command channel closed");
                            break;
                        }
                    };
                    let result = process(&ctx, cmd);
                    let _ = result_tx.send(result);
                }
            });
        }

        Self { cmd_tx, result_rx }
    }

    /// Sends a command to the worker pool. The command is delivered to whichever
    /// worker thread acquires the lock first.
    ///
    /// Logs an error when every worker has already exited: in that state the
    /// command cannot be delivered and is dropped, so callers should treat the
    /// request as lost.
    pub fn send(&self, cmd: C) {
        if self.cmd_tx.send(cmd).is_err() {
            log::error!("BackgroundWorker: command dropped, all worker threads have exited");
        }
    }

    /// Returns a completed result, or `None` if none is ready yet.
    /// This never blocks the calling thread.
    pub fn try_receive(&self) -> Option<R> {
        self.result_rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn wait_for_results<C, R>(worker: &BackgroundWorker<C, R>, count: usize) -> Vec<R>
    where
        C: Send + 'static,
        R: Send + 'static,
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut results = Vec::new();
        while results.len() < count {
            if let Some(result) = worker.try_receive() {
                results.push(result);
                continue;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker results did not arrive before the timeout"
            );
            thread::sleep(Duration::from_millis(1));
        }
        results
    }

    #[test]
    fn every_command_is_processed_exactly_once() {
        const COMMANDS: usize = 32;
        let worker = BackgroundWorker::new(4, |n: usize| n * 2);
        for n in 0..COMMANDS {
            worker.send(n);
        }

        let mut results = wait_for_results(&worker, COMMANDS);
        results.sort_unstable();
        let doubled: Vec<_> = (0..COMMANDS).map(|n| n * 2).collect();
        assert_eq!(results, doubled);
    }

    #[test]
    fn init_runs_once_per_worker_and_context_is_shared_by_that_thread() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static INIT_CALLS: AtomicUsize = AtomicUsize::new(0);
        const WORKERS: usize = 3;
        const COMMANDS: usize = 12;

        let worker = BackgroundWorker::<(), usize>::new_with_init(
            WORKERS,
            || INIT_CALLS.fetch_add(1, Ordering::SeqCst),
            |ctx, (): ()| *ctx,
        );
        for _ in 0..COMMANDS {
            worker.send(());
        }

        let results = wait_for_results(&worker, COMMANDS);
        // Every command observes one of the per-thread contexts.
        assert!(results.iter().all(|ctx| *ctx < WORKERS));
        assert_eq!(
            INIT_CALLS.load(Ordering::SeqCst),
            WORKERS,
            "each worker must initialize its context exactly once"
        );
    }

    #[test]
    fn non_send_context_is_accepted() {
        #[allow(dead_code)]
        struct NotSend(std::rc::Rc<()>);
        let worker = BackgroundWorker::<usize, usize>::new_with_init(
            2,
            || NotSend(std::rc::Rc::new(())),
            |_ctx, n: usize| n + 1,
        );
        for n in 0..4 {
            worker.send(n);
        }
        let mut results = wait_for_results(&worker, 4);
        results.sort_unstable();
        assert_eq!(results, vec![1, 2, 3, 4]);
    }
}
