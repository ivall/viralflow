//! Work on a background thread whose result comes back on the UI thread.

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// Runs `work` on its own thread and then `done` on the UI thread with the result.
pub fn background<T: Send + 'static>(name: &str, work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (sender, receiver) = channel();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = sender.send(work());
    });
    if let Err(error) = spawned {
        log::error!("cannot start {name}: {error}");
        return;
    }
    poll(receiver, done);
}

/// Checks for the result every 50 ms; each check schedules the next, so nothing
/// keeps itself alive once the result is in.
fn poll<T: 'static>(receiver: Receiver<T>, done: impl FnOnce(T) + 'static) {
    match receiver.try_recv() {
        Ok(value) => done(value),
        Err(TryRecvError::Empty) => slint::Timer::single_shot(Duration::from_millis(50), move || poll(receiver, done)),
        Err(TryRecvError::Disconnected) => log::warn!("a background task ended without a result"),
    }
}

/// Runs `work` on its own thread; every message it sends reaches `on` on the
/// UI thread, and `finished` runs once the thread is done.
pub fn stream<T: Send + 'static>(
    name: &str,
    work: impl FnOnce(std::sync::mpsc::Sender<T>) + Send + 'static,
    on: impl FnMut(T) + 'static,
    finished: impl FnOnce() + 'static,
) {
    let (sender, receiver) = channel();
    if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(move || work(sender)) {
        log::error!("cannot start {name}: {error}");
        finished();
        return;
    }
    drain(receiver, on, finished);
}

fn drain<T: 'static>(receiver: Receiver<T>, mut on: impl FnMut(T) + 'static, finished: impl FnOnce() + 'static) {
    loop {
        match receiver.try_recv() {
            Ok(value) => on(value),
            Err(TryRecvError::Empty) => {
                slint::Timer::single_shot(Duration::from_millis(100), move || drain(receiver, on, finished));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                finished();
                return;
            }
        }
    }
}
