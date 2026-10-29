//! MCP (Model Context Protocol), so AI apps can edit in ViralFlow.
//!
//! - The running app serves MCP over HTTP at `http://127.0.0.1:<port>/mcp`
//!   (JSON responses, no event streams). Only this computer can connect and
//!   every request needs the token from the app's data folder. Tools run in
//!   the editor, so the user watches the changes happen.
//! - `viralflow --mcp` is a stdio server for apps that start a command
//!   (Claude Desktop, Cursor, Claude Code…). It answers the handshake itself
//!   and passes tool calls to the running app, starting the app when needed.

use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::{bridge, prompt, specs, Origin, ToolOutput};

/// Newest first; an app asking for another version gets the newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const PORT: u16 = 47615;
/// How long the app waits for a long tool before answering "still running"
/// (AI apps give up on a request after about a minute).
const MCP_BUDGET: Duration = Duration::from_secs(50);
const CONNECTION_FILE: &str = "mcp.json";
const TOKEN_FILE: &str = "mcp-token";

// ------------------------------------------------------------ protocol

/// Answers one JSON-RPC message (or a batch); `None` for notifications.
pub fn handle(message: &Value, call: &dyn Fn(&str, Value) -> ToolOutput) -> Option<Value> {
    if let Some(batch) = message.as_array() {
        let replies: Vec<Value> = batch.iter().filter_map(|m| handle(m, call)).collect();
        return (!replies.is_empty()).then_some(Value::Array(replies));
    }
    let id = message.get("id").cloned().filter(|id| !id.is_null())?;
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" | "logging/setLevel" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": specs::mcp_tools() })),
        "tools/call" => tools_call(&params, call),
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "viralflow", "title": "ViralFlow", "version": env!("CARGO_PKG_VERSION") },
        "instructions": prompt::mcp(),
    })
}

fn tools_call(params: &Value, call: &dyn Fn(&str, Value) -> ToolOutput) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "The tool name is missing.".to_string()))?;
    if !specs::exists(name) {
        return Err((-32602, format!("Unknown tool: {name}")));
    }
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    Ok(result_of(&call(name, arguments)))
}

/// A tool's output as MCP content: the JSON as text, then the pictures.
pub fn result_of(output: &ToolOutput) -> Value {
    let mut content = vec![json!({ "type": "text", "text": output.text() })];
    for picture in &output.pictures {
        content.push(json!({ "type": "text", "text": picture.caption }));
        content.push(json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&picture.jpeg), "mimeType": "image/jpeg" }));
    }
    json!({ "content": content, "isError": output.error })
}

fn is_tool_call(message: &Value) -> bool {
    match message {
        Value::Array(batch) => batch.iter().any(is_tool_call),
        _ => message.get("method").and_then(Value::as_str) == Some("tools/call"),
    }
}

// ------------------------------------------------------------ connection details

/// How AI apps reach the running app (written while the server runs).
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub url: String,
    pub token: String,
    pub pid: u32,
}

fn read_connection(data: &Path) -> Option<Connection> {
    let value: Value = serde_json::from_slice(&std::fs::read(data.join(CONNECTION_FILE)).ok()?).ok()?;
    Some(Connection {
        url: value.get("url")?.as_str()?.to_string(),
        token: value.get("token")?.as_str()?.to_string(),
        pid: value.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
    })
}

fn random_hex() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // RandomState is keyed from the operating system's random source.
    (0..2u64)
        .map(|round| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(round);
            hasher.write_u128(nanos);
            hasher.write_u32(std::process::id());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// The token AI apps send (made once and kept, so HTTP setups keep working).
fn token(data: &Path) -> Result<String, String> {
    let path = data.join(TOKEN_FILE);
    if let Ok(token) = std::fs::read_to_string(&path) {
        if token.trim().len() >= 24 {
            return Ok(token.trim().to_string());
        }
    }
    let token = format!("vf_{}", random_hex());
    crate::secrets::write_private(&path, &token).map_err(|e| e.to_string())?;
    Ok(token)
}

/// Same answer in the same time whatever the guess (the token is a secret).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ------------------------------------------------------------ the server in the app

struct Running {
    server: Arc<tiny_http::Server>,
    url: String,
    token: String,
}

/// The app's MCP endpoint, started and stopped from Settings.
pub struct Service {
    data: PathBuf,
    running: Mutex<Option<Running>>,
}

impl Service {
    pub fn new(data: &Path) -> Self {
        Service { data: data.to_path_buf(), running: Mutex::new(None) }
    }

    /// Starts listening (if not already); returns the URL.
    pub fn start(&self) -> Result<String, String> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = running.as_ref() {
            return Ok(r.url.clone());
        }
        let token = token(&self.data)?;
        // A second copy of the app (or another program) may hold the usual port.
        let (server, port) = (PORT..PORT + 10)
            .find_map(|port| tiny_http::Server::http(("127.0.0.1", port)).ok().map(|server| (server, port)))
            .ok_or("No local port is free for the MCP server.")?;
        let server = Arc::new(server);
        let url = format!("http://127.0.0.1:{port}/mcp");
        let listener = server.clone();
        let secret = token.clone();
        std::thread::Builder::new()
            .name("mcp-server".into())
            .spawn(move || {
                for request in listener.incoming_requests() {
                    let secret = secret.clone();
                    let _ = std::thread::Builder::new().name("mcp-request".into()).spawn(move || respond(request, &secret, port));
                }
            })
            .map_err(|e| e.to_string())?;
        let details = json!({ "url": url, "port": port, "token": token, "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") });
        crate::secrets::write_private(&self.data.join(CONNECTION_FILE), &details.to_string()).map_err(|e| e.to_string())?;
        log::info!("MCP server listening on {url}");
        *running = Some(Running { server, url: url.clone(), token });
        Ok(url)
    }

    pub fn stop(&self) {
        let Some(running) = self.running.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        running.server.unblock();
        // Only our own details: another copy of the app may have written its own since.
        if read_connection(&self.data).is_some_and(|c| c.pid == std::process::id()) {
            let _ = std::fs::remove_file(self.data.join(CONNECTION_FILE));
        }
        log::info!("MCP server stopped");
    }

    /// URL and token while the server runs.
    pub fn details(&self) -> Option<(String, String)> {
        self.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| (r.url.clone(), r.token.clone()))
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

fn header(request: &tiny_http::Request, name: &'static str) -> Option<String> {
    request.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string())
}

fn send(request: tiny_http::Request, status: u16, body: Option<Value>) {
    let mut response = match body {
        Some(body) => tiny_http::Response::from_string(body.to_string()),
        None => tiny_http::Response::from_string(String::new()),
    }
    .with_status_code(status);
    if let Ok(content) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        response.add_header(content);
    }
    if status == 405 {
        if let Ok(allow) = tiny_http::Header::from_bytes("Allow", "POST") {
            response.add_header(allow);
        }
    }
    let _ = request.respond(response);
}

fn rpc_error(code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": code, "message": message } })
}

fn respond(mut request: tiny_http::Request, token: &str, port: u16) {
    // Web pages must not reach the editor: no foreign Host (DNS rebinding) or Origin.
    let local = |value: &str| {
        let value = value.trim().to_lowercase();
        let value = value.strip_prefix("http://").unwrap_or(&value);
        ["127.0.0.1", "localhost", "[::1]"].iter().any(|host| value == *host || value == format!("{host}:{port}"))
    };
    if header(&request, "Host").is_some_and(|h| !local(&h)) || header(&request, "Origin").is_some_and(|o| !local(&o)) {
        return send(request, 403, Some(rpc_error(-32000, "Only apps on this computer can connect.")));
    }
    let path = request.url().split('?').next().unwrap_or_default().trim_end_matches('/').to_string();
    if path != "/mcp" && !path.is_empty() {
        return send(request, 404, Some(rpc_error(-32000, "The MCP endpoint is /mcp.")));
    }
    let authorized = header(&request, "Authorization").and_then(|v| v.strip_prefix("Bearer ").map(|t| same(t.trim(), token))).unwrap_or(false);
    if !authorized {
        return send(request, 401, Some(rpc_error(-32001, "Missing or wrong token: see Settings → AI in ViralFlow.")));
    }
    if *request.method() != tiny_http::Method::Post {
        return send(request, 405, None);
    }
    let mut body = String::new();
    if request.as_reader().take(16 * 1024 * 1024).read_to_string(&mut body).is_err() {
        return send(request, 400, Some(rpc_error(-32700, "The request could not be read.")));
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        return send(request, 400, Some(rpc_error(-32700, "The request is not JSON.")));
    };
    match handle(&message, &|name, arguments| bridge::call(name, arguments, Origin::Mcp, MCP_BUDGET)) {
        Some(reply) => send(request, 200, Some(reply)),
        None => send(request, 202, None),
    }
}

// ------------------------------------------------------------ `viralflow --mcp`

/// The stdio server: one JSON-RPC message per line in, one answer per line out.
pub fn run_stdio() -> i32 {
    let data = match crate::paths::data_dir() {
        Ok(data) => data,
        Err(error) => {
            eprintln!("viralflow: {error}");
            return 1;
        }
    };
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).http_status_as_error(false).build().into();
    let mut link: Option<Connection> = None;
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(_) => Some(rpc_error(-32700, "The message is not JSON.")),
            Ok(message) if is_tool_call(&message) => forward(&data, &agent, &mut link, &line, &message),
            Ok(message) => handle(&message, &|_, _| ToolOutput::error("ViralFlow is not reachable.")),
        };
        if let Some(reply) = reply {
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
    0
}

/// Posts `body` to the app; `Ok(None)` when it answered with no content.
fn post(agent: &ureq::Agent, link: &Connection, body: &str) -> Result<Option<Value>, String> {
    let mut response = agent
        .post(&link.url)
        .header("Authorization", &format!("Bearer {}", link.token))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .send(body)
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    match status {
        200 => serde_json::from_str(&text).map(Some).map_err(|e| e.to_string()),
        202 | 204 => Ok(None),
        _ => Err(format!("ViralFlow answered {status}: {text}")),
    }
}

fn reachable(agent: &ureq::Agent, link: &Connection) -> bool {
    post(agent, link, &json!({ "jsonrpc": "2.0", "id": "ping", "method": "ping" }).to_string()).is_ok_and(|reply| reply.is_some())
}

fn mcp_disabled(data: &Path) -> bool {
    std::fs::read(data.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| settings.get("mcpEnabled").and_then(Value::as_bool))
        == Some(false)
}

/// Starts the app (detached: it must outlive this process and not share its pipes).
fn launch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map(|_| ()).map_err(|e| format!("ViralFlow could not be started: {e}"))
}

/// The running app's details, starting the app when it doesn't answer.
fn connect(data: &Path, agent: &ureq::Agent) -> Result<Connection, String> {
    if let Some(link) = read_connection(data).filter(|link| reachable(agent, link)) {
        return Ok(link);
    }
    if mcp_disabled(data) {
        return Err("AI apps are turned off in ViralFlow (Settings → AI → Let AI apps edit in ViralFlow).".into());
    }
    let stale = read_connection(data);
    eprintln!("viralflow: starting ViralFlow");
    launch()?;
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(link) = read_connection(data).filter(|link| Some(link) != stale.as_ref() && reachable(agent, link)) {
            return Ok(link);
        }
    }
    Err("ViralFlow did not start in time. Open it, then try again.".into())
}

fn forward(data: &Path, agent: &ureq::Agent, link: &mut Option<Connection>, line: &str, message: &Value) -> Option<Value> {
    let mut problem = String::new();
    for _ in 0..2 {
        let connection = match link.clone() {
            Some(connection) => connection,
            None => match connect(data, agent) {
                Ok(connection) => {
                    *link = Some(connection.clone());
                    connection
                }
                Err(error) => {
                    problem = error;
                    break;
                }
            },
        };
        match post(agent, &connection, line) {
            Ok(reply) => return reply,
            // The app was closed or restarted: find it again.
            Err(error) => {
                problem = error;
                *link = None;
            }
        }
    }
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    // A tool error the model can read (not a protocol error).
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result_of(&ToolOutput::error(problem)) }))
}

// ------------------------------------------------------------ setup snippets

/// How to add ViralFlow to AI apps, for Settings.
pub struct Setup {
    pub desktop: String,
    pub claude_code: String,
    pub http: String,
}

pub fn setup(details: Option<(String, String)>) -> Setup {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "viralflow".into());
    let desktop = serde_json::to_string_pretty(&json!({ "mcpServers": { "viralflow": { "command": exe, "args": ["--mcp"] } } })).unwrap_or_default();
    let claude_code = format!("claude mcp add viralflow -- \"{exe}\" --mcp");
    let http = match details {
        Some((url, token)) => format!("claude mcp add --transport http viralflow {url} --header \"Authorization: Bearer {token}\""),
        None => String::new(),
    };
    Setup { desktop, claude_code, http }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_tools(_: &str, _: Value) -> ToolOutput {
        ToolOutput::error("no tools in this test")
    }

    #[test]
    fn handshake_and_tool_list() {
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }), &no_tools).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(reply["result"]["serverInfo"]["name"], "viralflow");
        assert!(reply["result"]["instructions"].as_str().unwrap().contains("ViralFlow"));
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }), &no_tools).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), &no_tools).is_none());
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": "x", "method": "tools/list" }), &no_tools).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "remove_time_ranges" && t["inputSchema"]["type"] == "object"));
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }), &no_tools).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn tool_calls_carry_text_and_pictures() {
        let call = |name: &str, arguments: Value| {
            assert_eq!(name, "view_frames");
            assert_eq!(arguments["times"][0], 1.5);
            ToolOutput {
                value: json!({ "summary": "ok" }),
                pictures: vec![super::super::Picture { jpeg: vec![1, 2, 3], caption: "frame".into() }],
                error: false,
            }
        };
        let reply =
            handle(&json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "view_frames", "arguments": { "times": [1.5] } } }), &call)
                .unwrap();
        let content = reply["result"]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[2]["type"], "image");
        assert_eq!(content[2]["data"], "AQID");
        assert_eq!(reply["result"]["isError"], false);
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": { "name": "rm_rf" } }), &call).unwrap();
        assert_eq!(reply["error"]["code"], -32602);
        assert!(is_tool_call(&json!([{ "method": "ping" }, { "method": "tools/call" }])));
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same("vf_abc", "vf_abc"));
        assert!(!same("vf_abc", "vf_abd"));
        assert!(!same("vf_abc", "vf_ab"));
        assert_eq!(random_hex().len(), 32);
        assert_ne!(random_hex(), random_hex());
    }

    #[test]
    fn the_server_needs_the_token_and_a_local_host() {
        let data = std::env::temp_dir().join(format!("vf-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let service = Service::new(&data);
        let url = service.start().unwrap();
        let (_, token) = service.details().unwrap();
        let link = read_connection(&data).unwrap();
        assert_eq!((link.url.as_str(), link.token.as_str()), (url.as_str(), token.as_str()));
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string();
        assert!(reachable(&agent, &link));
        let wrong = Connection { token: "vf_wrong".into(), ..link.clone() };
        assert!(post(&agent, &wrong, &ping).unwrap_err().contains("401"));
        let foreign =
            agent.post(&url).header("Authorization", &format!("Bearer {token}")).header("Origin", "https://evil.example").send(ping.as_str()).unwrap();
        assert_eq!(foreign.status().as_u16(), 403);
        let notification = post(&agent, &link, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()).unwrap();
        assert!(notification.is_none());
        drop(service);
        assert!(read_connection(&data).is_none(), "the details go with the server");
        let again = Service::new(&data);
        again.start().unwrap();
        assert_eq!(again.details().unwrap().1, token, "the token is kept");
        drop(again);
        let _ = std::fs::remove_dir_all(&data);
    }
}
