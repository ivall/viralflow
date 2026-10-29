//! The chat in the editor's AI panel: a conversation with the model chosen in
//! Settings (OpenRouter) that edits the open project with the tools. Each
//! project keeps its conversation (saved next to it), and the changes of the
//! latest request can be undone at once.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, VecModel};
use vf_ai::openrouter::{self, Client};
use vf_core::model::Project;

use super::{bridge, prompt, specs, Origin, Picture, ToolOutput};
use crate::app::Context;
use crate::editor::EditorController;
use crate::{ChatEntry, ChatState};

/// Model calls in one request before it stops (a runaway loop costs money).
const MAX_STEPS: usize = 40;
/// How long a tool may work before the model hears "still running".
const TOOL_BUDGET: Duration = Duration::from_secs(900);
const FILE: &str = "assistant.json";

const SUGGESTIONS: &[&str] = &[
    "Add captions where the spoken word lights up",
    "Cut the silences, filler words and retakes",
    "Add a bold hook title for the first 2 seconds",
    "Make it more dynamic: zooms, punches and b-roll from my media",
    "Find the beats of the music and sync effects to them",
    "Check the video frame by frame and fix anything that looks off",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    kind: String,
    text: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    turn: i32,
    #[serde(skip)]
    working: bool,
}

impl Entry {
    fn new(kind: &str, text: &str, turn: i32) -> Self {
        Entry { kind: kind.into(), text: text.into(), turn, ..Default::default() }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Conversation {
    /// What the model saw, in OpenAI format (no system message, no pictures).
    messages: Vec<Value>,
    entries: Vec<Entry>,
    turns: i32,
    /// USD spent in this conversation.
    cost: f64,
    /// The document before each request of this session, and the requests that changed it.
    #[serde(skip)]
    before: HashMap<i32, Project>,
    #[serde(skip)]
    changed: HashSet<i32>,
}

/// How a request ended: its messages and why it stopped early.
type Ending = Option<(Vec<Value>, Option<String>)>;

enum Event {
    Text(String),
    ToolStarted(String),
    ToolFinished {
        summary: String,
        failed: bool,
    },
    Status(String),
    Cost(f64),
    /// The request is over; its messages (pictures left out) and why it stopped early.
    Finished {
        messages: Vec<Value>,
        error: Option<String>,
    },
}

pub struct Chat {
    context: Rc<Context>,
    editor: Rc<EditorController>,
    /// The project whose conversation shows.
    project: RefCell<Option<String>>,
    conversations: RefCell<HashMap<String, Conversation>>,
    rows: Rc<VecModel<ChatEntry>>,
    /// Stop flag of the running request.
    running: RefCell<Option<Arc<AtomicBool>>>,
    scroll: Cell<i32>,
}

impl Chat {
    pub fn new(context: Rc<Context>, editor: Rc<EditorController>) -> Rc<Self> {
        let chat = Rc::new(Chat {
            context: context.clone(),
            editor: editor.clone(),
            project: RefCell::default(),
            conversations: RefCell::default(),
            rows: Rc::new(VecModel::default()),
            running: RefCell::default(),
            scroll: Cell::new(0),
        });
        if let Some(window) = context.window() {
            let state = window.global::<ChatState>();
            state.set_entries(ModelRc::from(chat.rows.clone()));
            let suggestions: Vec<slint::SharedString> = SUGGESTIONS.iter().map(|s| (*s).into()).collect();
            state.set_suggestions(ModelRc::new(VecModel::from(suggestions)));
            macro_rules! on {
                ($method:ident, |$me:ident $(, $arg:ident)*| $body:expr) => {{
                    let weak = Rc::downgrade(&chat);
                    state.$method(move |$($arg),*| {
                        if let Some($me) = weak.upgrade() {
                            $body
                        }
                    });
                }};
            }
            on!(on_send, |me, text| me.send(&text));
            on!(on_stop, |me| {
                if let Some(stop) = me.running.borrow().as_ref() {
                    stop.store(true, Ordering::Release);
                }
                if let Some(window) = me.context.window() {
                    window.global::<ChatState>().set_status("Stopping after this step…".into());
                }
            });
            on!(on_reset, |me| me.reset());
            on!(on_undo_turn, |me, turn| me.undo_turn(turn));
            on!(on_open_settings, |me| me.editor.open_settings());
        }
        {
            let weak = Rc::downgrade(&chat);
            *editor.on_opened.borrow_mut() = Some(Box::new(move |id| {
                if let Some(chat) = weak.upgrade() {
                    chat.show(&id);
                }
            }));
            let weak = Rc::downgrade(&chat);
            *editor.on_panel.borrow_mut() = Some(Box::new(move |panel| {
                if let (Some(chat), "ai") = (weak.upgrade(), panel.as_str()) {
                    chat.sync();
                }
            }));
        }
        chat
    }

    fn file(&self, project: &str) -> Option<PathBuf> {
        crate::paths::safe_id(project).ok().map(|id| self.context.paths.projects.join(id).join(FILE))
    }

    /// Shows (and loads) the conversation of a project.
    pub fn show(&self, project: &str) {
        if !self.conversations.borrow().contains_key(project) {
            let saved = self.file(project).and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Conversation>(&bytes).ok());
            self.conversations.borrow_mut().insert(project.to_string(), saved.unwrap_or_default());
        }
        *self.project.borrow_mut() = Some(project.to_string());
        self.sync();
        self.jump_to_end();
    }

    /// Shows the newest message even when the user had scrolled up.
    fn jump_to_end(&self) {
        if let Some(window) = self.context.window() {
            let state = window.global::<ChatState>();
            state.set_jump_request(state.get_jump_request() + 1);
        }
    }

    fn save(&self, project: &str) {
        let conversations = self.conversations.borrow();
        let (Some(conversation), Some(path)) = (conversations.get(project), self.file(project)) else { return };
        // A project deleted meanwhile stays deleted.
        if !path.parent().is_some_and(|dir| dir.join("project.json").is_file()) {
            return;
        }
        match serde_json::to_vec(conversation) {
            Ok(bytes) => {
                if let Err(error) = crate::paths::write_atomic(&path, &bytes) {
                    log::warn!("chat could not be saved: {error}");
                }
            }
            Err(error) => log::warn!("chat could not be saved: {error}"),
        }
    }

    fn with_conversation<T>(&self, change: impl FnOnce(&mut Conversation) -> T) -> Option<T> {
        let project = self.project.borrow().clone()?;
        self.conversations.borrow_mut().get_mut(&project).map(change)
    }

    pub fn sync(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<ChatState>();
        let settings = self.context.settings.get();
        state.set_ready(self.context.keys.status().has_key);
        state.set_model(settings.openrouter_model.clone().into());
        let busy = self.running.borrow().is_some();
        state.set_busy(busy);
        let (rows, cost) = self
            .with_conversation(|c| {
                // Only the latest request that changed the project can be undone.
                let undoable = c.changed.iter().max().copied().filter(|turn| *turn == c.turns && c.before.contains_key(turn));
                let last_of = |index: usize| c.entries.get(index + 1).is_none_or(|next| next.turn != c.entries[index].turn);
                let rows: Vec<ChatEntry> = c
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, e)| ChatEntry {
                        kind: e.kind.clone().into(),
                        text: e.text.clone().into(),
                        detail: e.detail.clone().into(),
                        working: e.working,
                        failed: e.failed,
                        turn: e.turn,
                        can_undo: !busy && Some(e.turn) == undoable && last_of(index),
                    })
                    .collect();
                (rows, c.cost)
            })
            .unwrap_or_default();
        crate::models::sync_rows(&self.rows, rows);
        state.set_cost(if cost > 0.0 { format!("This chat cost about ${cost:.3}").into() } else { "Enter sends · Shift+Enter: new line".into() });
        self.scroll.set(self.scroll.get() + 1);
        state.set_scroll_request(self.scroll.get());
    }

    fn push(&self, entry: Entry) {
        self.with_conversation(|c| c.entries.push(entry));
        self.sync();
    }

    fn send(self: &Rc<Self>, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() || self.running.borrow().is_some() {
            return;
        }
        let Some(project) = self.editor.project_id() else { return };
        if self.project.borrow().as_deref() != Some(project.as_str()) {
            self.show(&project);
        }
        if let Some(window) = self.context.window() {
            window.global::<ChatState>().set_draft("".into());
        }
        let turn = self.with_conversation(|c| c.turns + 1).unwrap_or(1);
        let key = self.context.keys.key();
        if key.is_empty() {
            self.push(Entry::new("user", &text, turn));
            self.push(Entry::new("error", "Add your OpenRouter API key in Settings → AI first.", turn));
            return;
        }
        let model = self.context.settings.get().openrouter_model;
        let client = match Client::new(&key, &model) {
            Ok(client) => client,
            Err(error) => return self.push(Entry::new("error", &error.to_string(), turn)),
        };
        let (Some(state), Some(document)) = (self.editor.agent_context(), self.editor.agent_document()) else { return };
        let history = self
            .with_conversation(|c| {
                c.turns = turn;
                c.before.insert(turn, document);
                c.entries.push(Entry::new("user", &text, turn));
                c.messages.clone()
            })
            .unwrap_or_default();
        let mut messages = vec![json!({ "role": "system", "content": prompt::chat() })];
        messages.extend(history);
        let first = messages.len();
        messages.push(json!({ "role": "user", "content": format!("{text}\n\n---\nThe editor right now (JSON): {state}") }));
        let stop = Arc::new(AtomicBool::new(false));
        *self.running.borrow_mut() = Some(stop.clone());
        if let Some(window) = self.context.window() {
            window.global::<ChatState>().set_status("Thinking…".into());
        }
        self.sync();
        self.jump_to_end();
        let (weak, weak_done) = (Rc::downgrade(self), Rc::downgrade(self));
        let finished: Rc<RefCell<Ending>> = Rc::default();
        let finished_done = finished.clone();
        let request = text.clone();
        crate::tasks::stream(
            "ai-chat",
            move |sender| converse(client, messages, first, project, stop, &sender),
            move |event| {
                if let Some(me) = weak.upgrade() {
                    me.event(event, turn, &finished);
                }
            },
            move || {
                if let Some(me) = weak_done.upgrade() {
                    let (messages, error) = finished_done.borrow_mut().take().unwrap_or((vec![], Some("The request ended unexpectedly.".into())));
                    me.finish(turn, &request, messages, error);
                }
            },
        );
    }

    fn event(&self, event: Event, turn: i32, finished: &RefCell<Ending>) {
        match event {
            Event::Text(text) => self.push(Entry::new("assistant", &text, turn)),
            Event::ToolStarted(title) => self.push(Entry { working: true, ..Entry::new("tool", &title, turn) }),
            Event::ToolFinished { summary, failed } => {
                self.with_conversation(|c| {
                    if let Some(entry) = c.entries.iter_mut().rev().find(|e| e.kind == "tool" && e.working) {
                        entry.working = false;
                        entry.failed = failed;
                        entry.detail = summary;
                    }
                });
                self.sync();
            }
            Event::Status(status) => {
                if let Some(window) = self.context.window() {
                    window.global::<ChatState>().set_status(status.into());
                }
            }
            Event::Cost(cost) => {
                self.with_conversation(|c| c.cost += cost);
            }
            Event::Finished { messages, error } => *finished.borrow_mut() = Some((messages, error)),
        }
    }

    fn finish(&self, turn: i32, request: &str, mut messages: Vec<Value>, error: Option<String>) {
        *self.running.borrow_mut() = None;
        // The kept request leaves out the editor state that came with it.
        if let Some(first) = messages.first_mut() {
            *first = json!({ "role": "user", "content": request });
        }
        let document = self.editor.agent_document();
        let project = self.project.borrow().clone();
        self.with_conversation(|c| {
            c.messages.extend(messages);
            // Keep the conversation from growing without end (whole requests only).
            while c.messages.len() > 160 {
                let next = c.messages.iter().skip(1).position(|m| m["role"] == "user" && m["content"].is_string()).map(|i| i + 1);
                match next {
                    Some(index) => {
                        c.messages.drain(..index);
                    }
                    None => break,
                }
            }
            for entry in c.entries.iter_mut().filter(|e| e.working) {
                entry.working = false;
                entry.failed = true;
                entry.detail = "Stopped".into();
            }
            if let (Some(before), Some(now)) = (c.before.get(&turn), document.as_ref()) {
                if before.tracks != now.tracks
                    || before.assets != now.assets
                    || before.name != now.name
                    || before.width != now.width
                    || before.export != now.export
                {
                    c.changed.insert(turn);
                }
            }
            if let Some(error) = error {
                let kind = if error == "Stopped." { "note" } else { "error" };
                c.entries.push(Entry::new(kind, &error, turn));
            }
        });
        if let Some(project) = project {
            self.save(&project);
        }
        self.sync();
    }

    fn reset(&self) {
        if self.running.borrow().is_some() {
            return;
        }
        let Some(project) = self.project.borrow().clone() else { return };
        self.conversations.borrow_mut().insert(project.clone(), Conversation::default());
        if let Some(path) = self.file(&project) {
            let _ = std::fs::remove_file(path);
        }
        self.sync();
    }

    fn undo_turn(&self, turn: i32) {
        if self.running.borrow().is_some() {
            return;
        }
        let Some(before) = self.with_conversation(|c| c.before.get(&turn).cloned()).flatten() else { return };
        if self.editor.agent_restore(before) {
            self.with_conversation(|c| {
                c.changed.remove(&turn);
                c.entries.push(Entry::new("note", "Undone. Ctrl+Z brings the changes back.", turn));
                // The model hears of it, so it doesn't build on changes that are gone.
                c.messages.push(json!({ "role": "user", "content": "(I undid all the changes of your last request in the editor.)" }));
                c.messages.push(json!({ "role": "assistant", "content": "Understood: those changes are gone." }));
            });
            if let Some(project) = self.project.borrow().clone() {
                self.save(&project);
            }
        }
        self.sync();
    }
}

/// Whether the model takes pictures and can call tools (from the public catalogue, read once).
fn abilities(model: &str) -> (bool, bool) {
    static CATALOGUE: Mutex<Option<Vec<openrouter::ModelSummary>>> = Mutex::new(None);
    let mut catalogue = CATALOGUE.lock().unwrap_or_else(|e| e.into_inner());
    if catalogue.is_none() {
        // Asked again next time when it can't be read now.
        *catalogue = openrouter::list_models(false).ok();
    }
    let found = catalogue.as_ref().and_then(|list| list.iter().find(|m| m.id == model || m.id == model.split(':').next().unwrap_or(model)));
    match found {
        Some(m) => (m.vision, m.tools),
        None => (false, true),
    }
}

/// Markdown the panel can't show, as plain text.
fn plain(text: &str) -> String {
    text.lines()
        .map(|line| {
            let line = line.trim_end();
            let line = line.trim_start_matches('#').trim_start_matches(' ');
            let line = if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) { format!("• {rest}") } else { line.to_string() };
            line.replace("**", "").replace("__", "").replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Pictures stay out of the kept conversation (they would be sent again with every request).
fn without_pictures(message: Value) -> Value {
    match message.get("content") {
        Some(Value::Array(parts)) if parts.iter().any(|p| p["type"] == "image_url") => {
            let count = parts.iter().filter(|p| p["type"] == "image_url").count();
            let note = if count == 1 { "(a picture was shown here)".to_string() } else { format!("({count} pictures were shown here)") };
            json!({ "role": message["role"], "content": note })
        }
        _ => message,
    }
}

/// The conversation loop of one request, on its own thread.
fn converse(client: Client, mut messages: Vec<Value>, first: usize, project: String, stop: Arc<AtomicBool>, events: &Sender<Event>) {
    let send = |event: Event| {
        let _ = events.send(event);
    };
    let (vision, tools_ok) = abilities(client.model());
    let finish = |messages: &[Value], error: Option<String>| {
        let _ = events.send(Event::Finished { messages: messages[first..].iter().cloned().map(without_pictures).collect(), error });
    };
    if !tools_ok {
        return finish(
            &messages,
            Some(format!(
                "The model {} can't use tools, so it can't edit. Choose another model in Settings → AI (recent GPT, Claude, Gemini and Qwen models work).",
                client.model()
            )),
        );
    }
    let tools = specs::functions(true);
    for _ in 0..MAX_STEPS {
        if stop.load(Ordering::Acquire) {
            return finish(&messages, Some("Stopped.".into()));
        }
        send(Event::Status("Thinking…".into()));
        let reply = match client.chat_tools(&messages, &tools) {
            Ok(reply) => reply,
            Err(error) => return finish(&messages, Some(error.to_string())),
        };
        if let Some(cost) = reply.cost {
            send(Event::Cost(cost));
        }
        messages.push(reply.message.clone());
        if !reply.text.is_empty() {
            send(Event::Text(plain(&reply.text)));
        }
        if reply.tool_calls.is_empty() {
            let error = (reply.finish == "length").then(|| "The answer was cut off because it got too long.".to_string());
            return finish(&messages, error);
        }
        let mut pictures: Vec<Picture> = Vec::new();
        for call in reply.tool_calls {
            // Every call needs an answer, even when stopping.
            let output = if stop.load(Ordering::Acquire) {
                ToolOutput::error("Stopped by the user.")
            } else {
                let title = specs::title(&call.name);
                send(Event::ToolStarted(title.clone()));
                send(Event::Status(format!("{title}…")));
                let output = match serde_json::from_str::<Value>(if call.arguments.trim().is_empty() { "{}" } else { &call.arguments }) {
                    Ok(arguments) => bridge::call(&call.name, arguments, Origin::Chat(project.clone()), TOOL_BUDGET),
                    Err(error) => ToolOutput::error(format!("The arguments were not valid JSON ({error}); send them again as one JSON object.")),
                };
                send(Event::ToolFinished { summary: output.summary(), failed: output.error });
                output
            };
            messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": output.text() }));
            pictures.extend(output.pictures);
        }
        if !pictures.is_empty() {
            if vision {
                let captions: Vec<String> = pictures.iter().map(|p| p.caption.clone()).collect();
                let mut content = vec![json!({ "type": "text", "text": format!("Pictures from the tools, in order: {}.", captions.join("; ")) })];
                for picture in &pictures {
                    let data = base64::engine::general_purpose::STANDARD.encode(&picture.jpeg);
                    content.push(json!({ "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{data}") } }));
                }
                messages.push(json!({ "role": "user", "content": content }));
            } else {
                messages.push(json!({
                    "role": "user",
                    "content": "(The pictures can't be shown: the chosen model doesn't take images. Work from get_project; if seeing matters, tell the user to pick a model with vision in Settings → AI.)"
                }));
            }
        }
    }
    finish(&messages, Some(format!("Stopped after {MAX_STEPS} steps; ask again to continue.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_plain_text() {
        assert_eq!(plain("## Done\n- **Added** captions\n* cut `3` silences"), "Done\n• Added captions\n• cut 3 silences");
    }

    #[test]
    fn pictures_leave_the_kept_conversation() {
        let message = json!({ "role": "user", "content": [{ "type": "text", "text": "x" }, { "type": "image_url", "image_url": { "url": "data:" } }] });
        assert_eq!(without_pictures(message)["content"], "(a picture was shown here)");
        let text = json!({ "role": "user", "content": "hi" });
        assert_eq!(without_pictures(text.clone()), text);
    }

    #[test]
    fn conversations_round_trip_without_session_state() {
        let mut conversation = Conversation { turns: 2, cost: 0.01, ..Default::default() };
        conversation.entries.push(Entry { working: true, ..Entry::new("tool", "Add texts", 2) });
        conversation.changed.insert(2);
        let saved: Conversation = serde_json::from_slice(&serde_json::to_vec(&conversation).unwrap()).unwrap();
        assert_eq!(saved.turns, 2);
        assert!(!saved.entries[0].working && saved.changed.is_empty());
    }
}
