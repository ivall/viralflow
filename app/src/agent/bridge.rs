//! Tools run on the UI thread, where the editor lives; the chat and the MCP
//! server call them from their own threads through here. The UI thread only
//! ever does the quick part; waiting and heavy work happen on the caller's
//! thread (`Reply::Later`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::Value;

use super::host::Host;
use super::{Origin, Reply, ToolOutput};

thread_local! {
    static HOST: RefCell<Option<Rc<Host>>> = const { RefCell::new(None) };
}

/// Makes `host` answer tool calls (on the UI thread, once at start).
pub fn install(host: Rc<Host>) {
    HOST.with(|slot| *slot.borrow_mut() = Some(host));
}

/// Lets go of the host before the app shuts down: the editor it holds must
/// close with the window, not while the thread's storage is torn down.
pub fn uninstall() {
    let host = HOST.with(|slot| slot.borrow_mut().take());
    drop(host);
}

/// Runs `work` with the host on the UI thread and returns its result. Call it
/// from any other thread (on the UI thread it would wait for itself).
pub fn on_ui<T: Send + 'static>(work: impl FnOnce(&Rc<Host>) -> T + Send + 'static) -> Option<T> {
    let (sender, receiver) = channel();
    slint::invoke_from_event_loop(move || {
        let host = HOST.with(|slot| slot.borrow().clone());
        let _ = sender.send(host.map(|host| work(&host)));
    })
    .ok()?;
    receiver.recv().ok().flatten()
}

/// Calls a tool and waits up to `budget` for long work before answering.
pub fn call(name: &str, args: Value, origin: Origin, budget: Duration) -> ToolOutput {
    let tool = name.to_string();
    match on_ui(move |host| host.run(&tool, &args, &origin)) {
        Some(Reply::Now(output)) => output,
        Some(Reply::Later(finish)) => finish(budget),
        None => ToolOutput::error("ViralFlow is closing or not ready."),
    }
}
