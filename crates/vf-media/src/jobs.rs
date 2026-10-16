//! One pool of workers for background media work (filmstrips, waveforms,
//! library thumbnails), taking the most important job first. Importing twenty
//! files queues twenty jobs instead of starting twenty decoders, and the
//! pool stays small, leaving the other cores to the preview's own decoders.

use std::collections::BinaryHeap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// What a job is for, most urgent last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Anything that may wait.
    Background,
    /// Thumbnails in the media library.
    Library,
    /// Filmstrips and waveforms on the timeline.
    Timeline,
}

type Task = Box<dyn FnOnce() + Send>;

struct Job {
    priority: Priority,
    /// Order of arrival: older first within a priority.
    order: u64,
    task: Task,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        (self.priority, self.order) == (other.priority, other.order)
    }
}

impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority).then(other.order.cmp(&self.order))
    }
}

#[derive(Default)]
struct Queue {
    jobs: BinaryHeap<Job>,
    order: u64,
}

pub struct Pool {
    queue: Arc<(Mutex<Queue>, Condvar)>,
}

impl Pool {
    pub fn new(workers: usize, name: &str) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        for index in 0..workers.max(1) {
            let queue = queue.clone();
            std::thread::Builder::new()
                .name(format!("{name}-{index}"))
                .spawn(move || loop {
                    let task = {
                        let (lock, ready) = &*queue;
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(job) = state.jobs.pop() {
                                break job.task;
                            }
                            state = ready.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    // A panicking job must not take the worker with it.
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)).is_err() {
                        log::error!("a background media job panicked");
                    }
                })
                .expect("media worker thread");
        }
        Pool { queue }
    }

    pub fn spawn(&self, priority: Priority, task: impl FnOnce() + Send + 'static) {
        let (lock, ready) = &*self.queue;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.order += 1;
        let order = state.order;
        state.jobs.push(Job { priority, order, task: Box::new(task) });
        ready.notify_one();
    }

    /// Jobs waiting for a worker.
    pub fn pending(&self) -> usize {
        self.queue.0.lock().map(|state| state.jobs.len()).unwrap_or(0)
    }
}

/// The app's pool: a quarter of the cores, two to four workers.
pub fn shared() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Pool::new((cores / 4).clamp(2, 4), "media-job")
    })
}

/// Runs `task` on the shared pool.
pub fn spawn(priority: Priority, task: impl FnOnce() + Send + 'static) {
    shared().spawn(priority, task);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn urgent_jobs_run_first_and_ties_in_arrival_order() {
        let pool = Pool::new(1, "test-job");
        let (gate_tx, gate_rx) = channel::<()>();
        let (done_tx, done_rx) = channel::<&'static str>();
        // Keep the only worker busy while the queue fills.
        pool.spawn(Priority::Background, move || {
            let _ = gate_rx.recv();
        });
        while pool.pending() > 0 {
            std::thread::yield_now();
        }
        for (priority, name) in
            [(Priority::Background, "background"), (Priority::Library, "library 1"), (Priority::Timeline, "timeline"), (Priority::Library, "library 2")]
        {
            let done = done_tx.clone();
            pool.spawn(priority, move || done.send(name).unwrap());
        }
        gate_tx.send(()).unwrap();
        let order: Vec<_> = (0..4).map(|_| done_rx.recv().unwrap()).collect();
        assert_eq!(order, ["timeline", "library 1", "library 2", "background"]);
    }

    #[test]
    fn a_panicking_job_leaves_the_worker_running() {
        let pool = Pool::new(1, "test-panic");
        let (done_tx, done_rx) = channel();
        pool.spawn(Priority::Timeline, || panic!("broken file"));
        pool.spawn(Priority::Timeline, move || done_tx.send(()).unwrap());
        done_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the next job runs");
    }
}
