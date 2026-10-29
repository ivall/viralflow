//! Long tool calls (transcription, captions, beats, export) run as jobs: the
//! tool waits for the result for a while and otherwise answers with the job's
//! id, which `job_status` follows. Jobs live in one list any thread can read.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vf_core::engine::math::uid;

use super::{round, ToolOutput};

/// Finished jobs kept for `job_status`.
const KEEP: usize = 40;

#[derive(Debug, Clone)]
pub enum State {
    Running { fraction: f64, stage: String },
    Done(Value),
    Failed(String),
}

pub struct Job {
    pub id: String,
    pub kind: &'static str,
    started: Instant,
    state: Mutex<State>,
    changed: Condvar,
}

static JOBS: Mutex<Vec<Arc<Job>>> = Mutex::new(Vec::new());

pub fn start(kind: &'static str, stage: &str) -> Arc<Job> {
    let job = Arc::new(Job {
        id: uid(kind),
        kind,
        started: Instant::now(),
        state: Mutex::new(State::Running { fraction: 0.0, stage: stage.into() }),
        changed: Condvar::new(),
    });
    let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    if jobs.len() >= KEEP {
        if let Some(index) = jobs.iter().position(|j| !j.running()) {
            jobs.remove(index);
        }
    }
    jobs.push(job.clone());
    job
}

pub fn find(id: &str) -> Option<Arc<Job>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|j| j.id == id).cloned()
}

/// A running job of this kind (one export at a time, one beat analysis…).
pub fn running(kind: &str) -> Option<Arc<Job>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|j| j.kind == kind && j.running()).cloned()
}

impl Job {
    pub fn state(&self) -> State {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn running(&self) -> bool {
        matches!(self.state(), State::Running { .. })
    }

    pub fn progress(&self, fraction: f64, stage: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Running { .. }) {
            *state = State::Running { fraction: fraction.clamp(0.0, 1.0), stage: stage.into() };
        }
    }

    pub fn finish(&self, result: Result<Value, String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Running { .. }) {
            *state = match result {
                Ok(value) => State::Done(value),
                Err(error) => State::Failed(error),
            };
        }
        self.changed.notify_all();
    }

    /// Waits up to `budget` for the job to end.
    pub fn wait(&self, budget: Duration) -> State {
        let deadline = Instant::now() + budget;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while matches!(*state, State::Running { .. }) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            state = self.changed.wait_timeout(state, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        state.clone()
    }

    /// The tool's answer after waiting: the result, the error, or where the job is.
    pub fn output(&self, budget: Duration) -> ToolOutput {
        match self.wait(budget) {
            State::Done(value) => ToolOutput::ok(value),
            State::Failed(error) => ToolOutput::error(error),
            State::Running { fraction, stage } => ToolOutput::ok(json!({
                "summary": format!("Still working: {stage}"),
                "job_id": self.id,
                "status": "running",
                "stage": stage,
                "progress": round(fraction),
                "seconds": self.started.elapsed().as_secs(),
                "next": "call job_status with this job_id (wait_seconds up to 120) for the result",
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_answers_when_done_or_says_it_is_running() {
        let job = start("test", "Starting");
        assert!(running("test").is_some());
        let output = job.output(Duration::from_millis(10));
        assert_eq!(output.value["status"], "running");
        let worker = job.clone();
        std::thread::spawn(move || {
            worker.progress(0.5, "Halfway");
            worker.finish(Ok(json!({ "summary": "Done" })));
        });
        let output = job.output(Duration::from_secs(5));
        assert_eq!(output.value["summary"], "Done");
        assert!(find(&job.id).is_some());
        assert!(!job.running());
        // A finished job stays finished.
        job.finish(Err("late".into()));
        assert!(matches!(job.state(), State::Done(_)));
    }
}
