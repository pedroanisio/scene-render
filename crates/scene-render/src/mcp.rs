//! `scene-render mcp`: a read-only Model Context Protocol server on standard input and output.
//!
//! The transport is the protocol's stdio transport: JSON-RPC 2.0, one message per line. The server answers
//! `initialize`, `ping`, `tools/list` and `tools/call`, and ignores notifications. Its tools run the commands of
//! the same name in JSON mode and return their envelope (`scene-render-cli/1`, `schema/cli-output-1.schema.json`)
//! as the tool result's structured content: `validate`, `inspect`, `eval` (one frame's FrameGraph, what a render
//! would draw), `captions`, `capabilities` and `explain`. None writes a file or renders, and the session turns the
//! program cache off, so a document's programs run without storing their outputs.
//!
//! Everything a client sends is untrusted, the tool arguments above all, which a language model typically writes.
//! A message is parsed into a typed request that refuses unknown keys, its parameters into the typed parameters of
//! its method, and a tool's arguments into the typed arguments of that tool, which are then checked (paths and
//! names non-empty, bounded and without NUL, times finite, lists bounded, exclusive arguments not combined) before
//! the command is built from them. Whatever fails is answered with a JSON-RPC error and the session goes on. The
//! bounds are this module's: a message holds at most [`MAX_MESSAGE_BYTES`] bytes, and a session at most
//! [`MAX_MESSAGES`] messages.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::output::Out;
use crate::Command;

/// The largest message read, newline excluded: a longer one is refused unread.
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;

/// The messages one session reads; the session ends when it has read them (exit status 2).
pub const MAX_MESSAGES: u64 = 100_000;

/// The protocol revisions spoken, newest first: a client asking for another gets the newest.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The most files one `validate` call names.
const MAX_FILES: usize = 64;
/// The most `at` times one `captions` call names.
const MAX_TIMES: usize = 1_000;
/// The most parameter values one call sets.
const MAX_PARAMS: usize = 256;
/// The longest path, name or value, in bytes.
const MAX_TEXT: usize = 4_096;

/// What a message gets instead of a result: a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// The message is not JSON (-32700).
    Parse(String),
    /// The message is JSON but not a request this server reads (-32600).
    InvalidRequest(String),
    /// The method is not one this server has (-32601).
    MethodNotFound(String),
    /// The parameters, a tool's name or its arguments are not usable (-32602).
    InvalidParams(String),
}

impl RpcError {
    /// The JSON-RPC error code.
    pub fn code(&self) -> i64 {
        match self {
            RpcError::Parse(_) => -32700,
            RpcError::InvalidRequest(_) => -32600,
            RpcError::MethodNotFound(_) => -32601,
            RpcError::InvalidParams(_) => -32602,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Parse(m) => write!(f, "parse error: {m}"),
            RpcError::InvalidRequest(m) => write!(f, "invalid request: {m}"),
            RpcError::MethodNotFound(m) => write!(f, "method not found: {m}"),
            RpcError::InvalidParams(m) => write!(f, "invalid params: {m}"),
        }
    }
}

impl std::error::Error for RpcError {}

/// Why a session ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// Standard input closed: the client ended the session.
    Eof,
    /// The client stopped reading the answers.
    Closed,
    /// The session read [`MAX_MESSAGES`] messages.
    Limit,
    /// Standard input or output failed.
    Io(String),
}

impl End {
    /// The exit status: 0 when the client ended the session, 2 when the server did.
    pub fn exit(&self) -> u8 {
        match self {
            End::Eof | End::Closed => 0,
            End::Limit | End::Io(_) => 2,
        }
    }

    /// Why the server ended the session, for standard error.
    pub fn message(&self) -> Option<String> {
        match self {
            End::Eof | End::Closed => None,
            End::Limit => Some(format!("the session read its limit of {MAX_MESSAGES} messages")),
            End::Io(e) => Some(e.clone()),
        }
    }
}

/// One line of input.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// A line of at most `max` bytes, without its newline.
    Message(Vec<u8>),
    /// A longer line, read through to its end and dropped.
    Oversize,
    /// The end of the input.
    Eof,
}

/// Reads one line, keeping at most `max` bytes of it in memory.
fn read_line(input: &mut impl BufRead, max: usize) -> std::io::Result<Line> {
    let mut kept = Vec::new();
    let mut over = false;
    let mut seen = false;
    loop {
        let buf = match input.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            return Ok(match (seen, over) {
                (false, _) => Line::Eof,
                (true, true) => Line::Oversize,
                (true, false) => Line::Message(kept),
            });
        }
        seen = true;
        let newline = buf.iter().position(|&b| b == b'\n');
        let part = &buf[..newline.unwrap_or(buf.len())];
        if !over && kept.len() + part.len() <= max {
            kept.extend_from_slice(part);
        } else {
            over = true;
            kept = Vec::new();
        }
        let used = newline.map_or(buf.len(), |i| i + 1);
        input.consume(used);
        if newline.is_some() {
            return Ok(if over { Line::Oversize } else { Line::Message(kept) });
        }
    }
}

/// Serves one session: reads messages from `input` and writes one answer line to `output` for each request.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> End {
    let mut server = Server::default();
    for _ in 0..MAX_MESSAGES {
        let answer = match read_line(&mut input, MAX_MESSAGE_BYTES) {
            Ok(Line::Eof) => return End::Eof,
            Ok(Line::Oversize) => Some(failure(
                &Value::Null,
                &RpcError::InvalidRequest(format!("a message holds at most {MAX_MESSAGE_BYTES} bytes")),
            )),
            Ok(Line::Message(bytes)) if bytes.iter().all(u8::is_ascii_whitespace) => None,
            Ok(Line::Message(bytes)) => server.handle(&bytes),
            Err(e) => return End::Io(format!("cannot read standard input: {e}")),
        };
        if let Some(answer) = answer {
            if let Err(e) = writeln!(output, "{answer}").and_then(|()| output.flush()) {
                return if e.kind() == std::io::ErrorKind::BrokenPipe {
                    End::Closed
                } else {
                    End::Io(format!("cannot write standard output: {e}"))
                };
            }
        }
    }
    End::Limit
}

/// Serves a session on standard input and output; the exit status of [`End::exit`].
pub fn run_stdio() -> u8 {
    // the tools write nothing: loading a document with programs would otherwise store their outputs
    std::env::set_var("SR_PROGRAM_CACHE", "off");
    let end = serve(std::io::stdin().lock(), std::io::stdout().lock());
    if let Some(m) = end.message() {
        eprintln!("error: {m}");
    }
    end.exit()
}

/// The `jsonrpc` member: only 2.0.
#[derive(Debug, Deserialize)]
enum JsonRpc {
    #[serde(rename = "2.0")]
    V2,
}

/// A request id: a string or an integer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Id {
    Int(i64),
    Str(String),
}

/// A request, or a notification when it has no id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(rename = "jsonrpc")]
    _jsonrpc: JsonRpc,
    #[serde(default)]
    id: Option<Id>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

/// The parameters' `_meta`, which every method may carry and none reads.
type Meta = Option<Map<String, Value>>;

/// `initialize`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: String,
    #[serde(rename = "capabilities")]
    _capabilities: Map<String, Value>,
    #[serde(rename = "clientInfo")]
    _client_info: ClientInfo,
    #[serde(default, rename = "_meta")]
    _meta: Meta,
}

/// The client's description of itself. Revisions of the protocol add descriptive keys to it (title, icons, a web
/// address), which are read past: nothing here acts on them.
#[derive(Debug, Deserialize)]
struct ClientInfo {
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "version")]
    _version: String,
}

/// `ping`, and a method without parameters.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {
    #[serde(default, rename = "_meta")]
    _meta: Meta,
}

/// `tools/list`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default, rename = "_meta")]
    _meta: Meta,
}

/// `tools/call`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default, rename = "_meta")]
    _meta: Meta,
}

/// The `validate` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateArgs {
    files: Vec<String>,
    #[serde(default)]
    no_assets: bool,
    #[serde(default)]
    base_dir: Option<String>,
    #[serde(default)]
    deny_warnings: bool,
}

/// The `inspect` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectArgs {
    file: String,
    #[serde(default)]
    no_assets: bool,
}

/// The `eval` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalArgs {
    file: String,
    #[serde(default)]
    time: Option<f64>,
    #[serde(default)]
    frame: Option<u64>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default)]
    row: Option<usize>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    no_assets: bool,
}

/// The `captions` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptionsArgs {
    file: String,
    #[serde(default)]
    at: Vec<f64>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default)]
    no_assets: bool,
}

/// The `explain` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplainArgs {
    #[serde(default)]
    code: Option<String>,
}

/// The `capabilities` tool: no arguments.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

fn invalid(m: impl Into<String>) -> RpcError {
    RpcError::InvalidParams(m.into())
}

/// A path, name or value: at most [`MAX_TEXT`] bytes, without NUL, and non-empty unless `empty` allows it.
fn text(field: &str, s: String, empty: bool) -> Result<String, RpcError> {
    if (s.is_empty() && !empty) || s.len() > MAX_TEXT || s.contains('\0') {
        let what = if empty { "0" } else { "1" };
        return Err(invalid(format!("{field}: expected {what} to {MAX_TEXT} bytes without NUL")));
    }
    Ok(s)
}

fn path(field: &str, s: String) -> Result<PathBuf, RpcError> {
    text(field, s, false).map(PathBuf::from)
}

fn name(field: &str, s: Option<String>) -> Result<Option<String>, RpcError> {
    s.map(|s| text(field, s, false)).transpose()
}

fn seconds(field: &str, t: f64) -> Result<f64, RpcError> {
    if t.is_finite() {
        Ok(t)
    } else {
        Err(invalid(format!("{field}: expected a finite time in seconds")))
    }
}

/// Parameter values as `--param ID=VALUE` gives them: an id is a name without `=`.
fn params(map: BTreeMap<String, String>) -> Result<Vec<(String, String)>, RpcError> {
    if map.len() > MAX_PARAMS {
        return Err(invalid(format!("params: at most {MAX_PARAMS} values")));
    }
    map.into_iter()
        .map(|(k, v)| {
            let k = text("params", k, false)?;
            if k.contains('=') {
                return Err(invalid(format!("params: the id {k:?} contains '='")));
            }
            let v = text(&format!("params.{k}"), v, true)?;
            Ok((k, v))
        })
        .collect()
}

impl ValidateArgs {
    fn command(self) -> Result<Command, RpcError> {
        if self.files.is_empty() || self.files.len() > MAX_FILES {
            return Err(invalid(format!("files: expected 1 to {MAX_FILES} paths")));
        }
        let files = self.files.into_iter().map(|f| path("files", f)).collect::<Result<_, _>>()?;
        let base_dir = self.base_dir.map(|d| path("base_dir", d)).transpose()?;
        Ok(Command::Validate {
            files,
            no_assets: self.no_assets,
            base_dir,
            deny_warnings: self.deny_warnings,
            quiet: false,
        })
    }
}

impl InspectArgs {
    fn command(self) -> Result<Command, RpcError> {
        Ok(Command::Inspect { file: path("file", self.file)?, no_assets: self.no_assets })
    }
}

impl EvalArgs {
    fn command(self) -> Result<Command, RpcError> {
        if self.time.is_some() && self.frame.is_some() {
            return Err(invalid("time and frame exclude each other"));
        }
        if self.data.is_some() && self.row.is_none() {
            return Err(invalid("data names the source of row, and needs it"));
        }
        Ok(Command::Eval {
            file: path("file", self.file)?,
            time: self.time.map(|t| seconds("time", t)).transpose()?,
            frame: self.frame,
            variant: name("variant", self.variant)?,
            layout: name("layout", self.layout)?,
            params: params(self.params)?,
            row: self.row,
            data: name("data", self.data)?,
            bench: false,
            no_assets: self.no_assets,
        })
    }
}

impl CaptionsArgs {
    fn command(self) -> Result<Command, RpcError> {
        if self.at.len() > MAX_TIMES {
            return Err(invalid(format!("at: at most {MAX_TIMES} times")));
        }
        Ok(Command::Captions {
            file: path("file", self.file)?,
            at: self.at.into_iter().map(|t| seconds("at", t)).collect::<Result<_, _>>()?,
            variant: name("variant", self.variant)?,
            layout: name("layout", self.layout)?,
            params: params(self.params)?,
            no_assets: self.no_assets,
        })
    }
}

impl ExplainArgs {
    fn command(self) -> Result<Command, RpcError> {
        let code = name("code", self.code)?;
        if code.as_ref().is_some_and(|c| c.len() > 64) {
            return Err(invalid("code: at most 64 bytes"));
        }
        Ok(Command::Explain { code })
    }
}

/// Parses a method's parameters or a tool's arguments; absent ones read as `{}`.
fn typed<T: DeserializeOwned>(what: &str, v: Option<Value>) -> Result<T, RpcError> {
    let v = v.unwrap_or_else(|| Value::Object(Map::new()));
    if !v.is_object() {
        return Err(invalid(format!("{what}: expected an object")));
    }
    serde_json::from_value(v).map_err(|e| invalid(format!("{what}: {e}")))
}

/// The command a tool call runs.
fn tool_command(name: &str, arguments: Option<Value>) -> Result<Command, RpcError> {
    let what = format!("the arguments of {name}");
    match name {
        "validate" => typed::<ValidateArgs>(&what, arguments)?.command(),
        "inspect" => typed::<InspectArgs>(&what, arguments)?.command(),
        "eval" => typed::<EvalArgs>(&what, arguments)?.command(),
        "captions" => typed::<CaptionsArgs>(&what, arguments)?.command(),
        "capabilities" => typed::<NoArgs>(&what, arguments).map(|NoArgs {}| Command::Capabilities),
        "explain" => typed::<ExplainArgs>(&what, arguments)?.command(),
        _ => Err(invalid(format!("no tool is named {name:?}"))),
    }
}

/// Runs a command as `--format json` would, and gives its envelope as a tool result.
fn run_tool(command: Command) -> Value {
    let name = command.name();
    let mut out = Out::capture();
    let code = crate::run(command, &mut out);
    let (doc, exit) = out.finish(Some(name), code);
    json!({
        "content": [{ "type": "text", "text": doc.to_string() }],
        "structuredContent": doc,
        "isError": exit != 0,
    })
}

const PATH: &str = "a path, relative to the server's working directory";

fn string(description: &str) -> Value {
    json!({ "type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": description })
}

fn flag(description: &str) -> Value {
    json!({ "type": "boolean", "default": false, "description": description })
}

fn param_values() -> Value {
    json!({
        "type": "object",
        "maxProperties": MAX_PARAMS,
        "additionalProperties": { "type": "string", "maxLength": MAX_TEXT },
        "description": "parameter values by parameter id",
    })
}

/// An object schema that refuses keys it does not name.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

/// The tools, as `tools/list` gives them.
fn tools() -> Value {
    let no_assets = flag("skip file existence and SHA-256 checks");
    let tool = |name: &str, title: &str, description: &str, input: Value| {
        json!({
            "name": name,
            "title": title,
            "description": description,
            "inputSchema": input,
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false,
            },
        })
    };
    json!([
        tool(
            "validate",
            "Validate documents",
            "Validate scene documents: XSD structure, co-occurrence and reference rules, and every referenced file \
             with its SHA-256 digest. The result is the scene-render-cli/1 envelope of `scene-render validate`.",
            object(
                json!({
                    "files": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_FILES,
                        "items": string(PATH),
                        "description": "the documents to validate",
                    },
                    "no_assets": no_assets,
                    "base_dir": string("directory relative URIs resolve against (default: each document's)"),
                    "deny_warnings": flag("treat warnings as errors"),
                }),
                &["files"],
            ),
        ),
        tool(
            "inspect",
            "Inspect a document",
            "Load a document and give a summary of its typed model (size, frame rate, duration, counts) and the \
             whole model under `model`. The result is the envelope of `scene-render inspect`.",
            object(json!({ "file": string(PATH), "no_assets": no_assets }), &["file"]),
        ),
        tool(
            "eval",
            "Evaluate a frame",
            "Evaluate one frame and give its FrameGraph: every node as a render would draw it, without rendering. \
             time and frame exclude each other; with neither, frame 0. The result is the envelope of \
             `scene-render eval`.",
            object(
                json!({
                    "file": string(PATH),
                    "time": { "type": "number", "description": "composition time in seconds" },
                    "frame": { "type": "integer", "minimum": 0, "description": "frame index at the project frame rate" },
                    "variant": string("variant to apply"),
                    "layout": string("layout to evaluate"),
                    "params": param_values(),
                    "row": { "type": "integer", "minimum": 0, "description": "data row for batch rendering" },
                    "data": string("data source of row (default: the first one)"),
                    "no_assets": no_assets,
                }),
                &["file"],
            ),
        ),
        tool(
            "captions",
            "Page captions",
            "Give each caption track after paging: the words of each cue with their times, the pages, and with at \
             the page and current word at those composition times. The result is the envelope of \
             `scene-render captions`.",
            object(
                json!({
                    "file": string(PATH),
                    "at": {
                        "type": "array",
                        "maxItems": MAX_TIMES,
                        "items": { "type": "number" },
                        "description": "composition times in seconds",
                    },
                    "variant": string("variant to apply"),
                    "layout": string("layout to evaluate"),
                    "params": param_values(),
                    "no_assets": no_assets,
                }),
                &["file"],
            ),
        ),
        tool(
            "capabilities",
            "Engine capabilities",
            "Give this engine's capability manifest (scene-render-capabilities/1): the constructs of the scene \
             format it does not draw exactly. The result is the envelope of `scene-render capabilities`.",
            object(json!({}), &[]),
        ),
        tool(
            "explain",
            "Explain a diagnostic code",
            "Describe a diagnostic code such as S06, C21 or R24-fill, or list every code when none is given. The \
             result is the envelope of `scene-render explain`.",
            object(json!({ "code": { "type": "string", "minLength": 1, "maxLength": 64 } }), &[]),
        ),
    ])
}

fn success(id: &Id, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: &Value, e: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code(), "message": e.to_string() } })
}

/// A session's state: whether the client has initialized it.
#[derive(Debug, Default)]
pub struct Server {
    initialized: bool,
}

impl Server {
    /// The answer to one message: none for a notification.
    pub fn handle(&mut self, message: &[u8]) -> Option<Value> {
        let raw: Value = match serde_json::from_slice(message) {
            Ok(v) => v,
            Err(e) => return Some(failure(&Value::Null, &RpcError::Parse(e.to_string()))),
        };
        let Value::Object(fields) = &raw else {
            let e = RpcError::InvalidRequest("a message is one JSON object; batches are not read".into());
            return Some(failure(&Value::Null, &e));
        };
        // the id to answer with, while the rest of the request may still be unusable
        let echo = match fields.get("id") {
            Some(v @ (Value::String(_) | Value::Number(_))) if serde_json::from_value::<Id>(v.clone()).is_ok() => {
                v.clone()
            }
            Some(_) => {
                let e = RpcError::InvalidRequest("id: expected a string or an integer".into());
                return Some(failure(&Value::Null, &e));
            }
            None => Value::Null,
        };
        let request: Request = match serde_json::from_value(raw) {
            Ok(r) => r,
            Err(e) => return Some(failure(&echo, &RpcError::InvalidRequest(e.to_string()))),
        };
        // a notification is never answered, not even with an error
        let id = request.id.clone()?;
        Some(match self.answer(request) {
            Ok(result) => success(&id, result),
            Err(e) => failure(&echo, &e),
        })
    }

    fn answer(&mut self, request: Request) -> Result<Value, RpcError> {
        let method = request.method.as_str();
        match method {
            "initialize" => {
                let p: InitializeParams = typed("initialize", request.params)?;
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == p.protocol_version)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                self.initialized = true;
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "scene-render", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Read-only tools over scene-render documents: validate, inspect, eval (one \
                        frame's FrameGraph, what a render would draw), captions, capabilities and explain. Paths \
                        resolve against the server's working directory. Each result is the scene-render-cli/1 \
                        envelope of the command of the same name.",
                }))
            }
            "ping" => typed::<NoParams>("ping", request.params).map(|_| json!({})),
            "tools/list" | "tools/call" if !self.initialized => {
                Err(RpcError::InvalidRequest(format!("{method} before initialize")))
            }
            "tools/list" => {
                let p: ListParams = typed("tools/list", request.params)?;
                if p.cursor.is_some() {
                    return Err(invalid("tools/list: the list has one page, and no cursor"));
                }
                Ok(json!({ "tools": tools() }))
            }
            "tools/call" => {
                let p: CallParams = typed("tools/call", request.params)?;
                Ok(run_tool(tool_command(&p.name, p.arguments)?))
            }
            _ => Err(RpcError::MethodNotFound(method.chars().take(128).collect())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(v: Value) -> Vec<u8> {
        v.to_string().into_bytes()
    }

    fn ready() -> Server {
        let mut s = Server::default();
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
        assert!(s.handle(&msg(init)).unwrap().get("result").is_some());
        s
    }

    fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
        let m = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        s.handle(&msg(m)).expect("a request is answered")
    }

    fn code(answer: &Value) -> i64 {
        answer["error"]["code"].as_i64().unwrap_or_else(|| panic!("not an error: {answer}"))
    }

    #[test]
    fn lines_are_bounded_and_an_oversize_one_is_dropped_to_its_end() {
        let mut input = std::io::Cursor::new(b"abcdef\nab\r\nlast".to_vec());
        assert_eq!(read_line(&mut input, 4).unwrap(), Line::Oversize);
        assert_eq!(read_line(&mut input, 4).unwrap(), Line::Message(b"ab\r".to_vec()));
        assert_eq!(read_line(&mut input, 4).unwrap(), Line::Message(b"last".to_vec()));
        assert_eq!(read_line(&mut input, 4).unwrap(), Line::Eof);
        // a reader that hands over one byte at a time is bounded the same way
        let mut slow = std::io::BufReader::with_capacity(1, &b"abcdefgh\nok\n"[..]);
        assert_eq!(read_line(&mut slow, 4).unwrap(), Line::Oversize);
        assert_eq!(read_line(&mut slow, 4).unwrap(), Line::Message(b"ok".to_vec()));
        let mut tail = std::io::Cursor::new(b"abcdefgh".to_vec());
        assert_eq!(read_line(&mut tail, 4).unwrap(), Line::Oversize);
        assert_eq!(read_line(&mut tail, 4).unwrap(), Line::Eof);
    }

    #[test]
    fn a_session_ends_at_its_message_limit_whatever_the_input_asks() {
        let ping = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        let input = ping.repeat(MAX_MESSAGES as usize + 5);
        let mut output = Vec::new();
        let end = serve(input.as_bytes(), &mut output);
        assert_eq!(end, End::Limit);
        assert_eq!(end.exit(), 2);
        assert!(end.message().unwrap().contains("100000"));
        assert_eq!(output.iter().filter(|&&b| b == b'\n').count(), MAX_MESSAGES as usize);
    }

    #[test]
    fn a_session_answers_requests_skips_blank_lines_and_ends_at_eof() {
        let input = "\n  \n{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"method\":\"ping\"}\nnot json\n";
        let mut output = Vec::new();
        assert_eq!(serve(input.as_bytes(), &mut output), End::Eof);
        let lines: Vec<Value> =
            String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], json!({"jsonrpc": "2.0", "id": "a", "result": {}}));
        assert_eq!(code(&lines[1]), -32700);
        assert_eq!(lines[1]["id"], Value::Null);
    }

    #[test]
    fn an_oversize_message_is_refused_and_the_session_goes_on() {
        let input =
            format!("{}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n", "x".repeat(MAX_MESSAGE_BYTES + 1));
        let mut output = Vec::new();
        assert_eq!(serve(input.as_bytes(), &mut output), End::Eof);
        let text = String::from_utf8(output).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(code(&lines[0]), -32600);
        assert_eq!(lines[1]["result"], json!({}));
    }

    #[test]
    fn a_closed_output_ends_the_session_quietly() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let end = serve(&b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n"[..], Closed);
        assert_eq!((end.exit(), end.message()), (0, None));
    }

    #[test]
    fn malformed_messages_get_typed_errors() {
        let mut s = Server::default();
        let cases = [
            (b"{\"jsonrpc\":".to_vec(), -32700),
            (b"[1,2]".to_vec(), -32600),
            (b"42".to_vec(), -32600),
            (msg(json!({"jsonrpc": "1.0", "id": 1, "method": "ping"})), -32600),
            (msg(json!({"id": 1, "method": "ping"})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": null, "method": "ping"})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": 1.5, "method": "ping"})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": {"a": 1}, "method": "ping"})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "extra": true})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": 3})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"})), -32601),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": [1]})), -32602),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {"x": 1}})), -32602),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})), -32600),
            (msg(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})), -32602),
        ];
        for (m, want) in cases {
            let answer = s.handle(&m).unwrap_or_else(|| panic!("no answer to {}", String::from_utf8_lossy(&m)));
            assert_eq!(code(&answer), want, "{} -> {answer}", String::from_utf8_lossy(&m));
            assert!(answer.get("result").is_none());
        }
        // the id of a request that is otherwise unusable is kept
        let a = s.handle(&msg(json!({"jsonrpc": "2.0", "id": "k", "method": "nope"}))).unwrap();
        assert_eq!((a["id"].clone(), code(&a)), (json!("k"), -32601));
    }

    #[test]
    fn notifications_are_never_answered() {
        let mut s = Server::default();
        for m in [
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "no/such/method"}),
            json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "explain"}}),
        ] {
            assert_eq!(s.handle(&msg(m)), None);
        }
    }

    #[test]
    fn initialize_agrees_on_a_version_it_speaks() {
        let mut s = Server::default();
        let init = |v: &str| {
            msg(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": v, "capabilities": {"roots": {}}, "_meta": {},
                "clientInfo": {"name": "c", "version": "2", "title": "C"}}}))
        };
        let a = s.handle(&init("2024-11-05")).unwrap();
        assert_eq!(a["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(a["result"]["serverInfo"]["name"], "scene-render");
        let a = s.handle(&init("1999-01-01")).unwrap();
        assert_eq!(a["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn every_tool_is_read_only_and_its_schema_names_exactly_what_it_reads() {
        let list = tools();
        let names: Vec<&str> = list.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["validate", "inspect", "eval", "captions", "capabilities", "explain"]);
        for t in list.as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            assert_eq!(t["annotations"]["readOnlyHint"], true);
            let schema = &t["inputSchema"];
            assert_eq!(schema["additionalProperties"], false, "{name}");
            // a value of every property the schema names is read, so no key it names is refused as unknown
            let mut full = Map::new();
            for (key, p) in schema["properties"].as_object().unwrap() {
                let v = match p["type"].as_str().unwrap() {
                    "string" => json!("x"),
                    "boolean" => json!(false),
                    "integer" => json!(0),
                    "number" => json!(0.5),
                    "array" => json!([]),
                    "object" => json!({}),
                    other => panic!("{other}"),
                };
                full.insert(key.clone(), v);
            }
            if name == "validate" {
                full.insert("files".into(), json!(["x"]));
            }
            if name == "eval" {
                full.remove("time");
            }
            tool_command(name, Some(Value::Object(full.clone())))
                .unwrap_or_else(|e| panic!("{name} refuses what its schema allows: {e}"));
            // and every key it requires is required
            for key in schema["required"].as_array().unwrap() {
                let mut partial = full.clone();
                partial.remove(key.as_str().unwrap());
                assert!(tool_command(name, Some(Value::Object(partial))).is_err(), "{name} without {key}");
            }
            // and a key it does not name is refused
            full.insert("render_to".into(), json!("/tmp/out.png"));
            assert!(tool_command(name, Some(Value::Object(full))).is_err(), "{name} took an unknown key");
        }
    }

    #[test]
    fn tool_arguments_are_checked_before_anything_runs() {
        let mut s = ready();
        let too_many: Vec<String> = (0..=MAX_FILES).map(|i| format!("{i}.xml")).collect();
        let long = "a".repeat(MAX_TEXT + 1);
        let many_params: Map<String, Value> = (0..=MAX_PARAMS).map(|i| (format!("p{i}"), json!("v"))).collect();
        let cases = [
            ("render", json!({"file": "a.xml"})),
            ("validate", json!({})),
            ("validate", json!({"files": []})),
            ("validate", json!({"files": too_many})),
            ("validate", json!({"files": [""]})),
            ("validate", json!({"files": ["a\u{0}.xml"]})),
            ("validate", json!({"files": "a.xml"})),
            ("validate", json!({"files": ["a.xml"], "base_dir": ""})),
            ("inspect", json!({"file": long})),
            ("inspect", json!({"file": 3})),
            ("inspect", json!(["a.xml"])),
            ("eval", json!({"file": "a.xml", "time": 1.0, "frame": 2})),
            ("eval", json!({"file": "a.xml", "frame": -1})),
            ("eval", json!({"file": "a.xml", "data": "d"})),
            ("eval", json!({"file": "a.xml", "variant": ""})),
            ("eval", json!({"file": "a.xml", "params": {"a=b": "c"}})),
            ("eval", json!({"file": "a.xml", "params": {"a": 1}})),
            ("eval", json!({"file": "a.xml", "params": many_params})),
            ("eval", json!({"file": "a.xml", "bench": true})),
            ("captions", json!({"file": "a.xml", "at": vec![0.0; MAX_TIMES + 1]})),
            ("captions", json!({"file": "a.xml", "at": ["1"]})),
            ("capabilities", json!({"verbose": true})),
            ("explain", json!({"code": "C".repeat(65)})),
        ];
        for (name, arguments) in cases {
            let a = call(&mut s, name, arguments.clone());
            assert_eq!(code(&a), -32602, "{name} {arguments} -> {a}");
            assert_eq!(a["id"], 7);
        }
        let a = s.handle(&msg(json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {}}))).unwrap();
        assert_eq!(code(&a), -32602);
        let a = s
            .handle(&msg(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list", "params": {"cursor": "2"}})))
            .unwrap();
        assert_eq!(code(&a), -32602);
    }

    #[test]
    fn non_finite_times_are_refused() {
        assert!(seconds("time", f64::NAN).is_err());
        assert!(seconds("time", f64::INFINITY).is_err());
        assert_eq!(seconds("time", -1.5), Ok(-1.5));
    }

    #[test]
    fn a_tool_result_is_the_command_envelope() {
        let mut s = ready();
        let a = call(&mut s, "explain", json!({"code": "C21"}));
        let r = &a["result"];
        assert_eq!(r["isError"], false, "{a}");
        let doc = &r["structuredContent"];
        assert_eq!(doc["format"], crate::output::FORMAT);
        assert_eq!(doc["command"], "explain");
        assert_eq!(doc["result"]["codes"][0]["code"], "C21");
        let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&text, doc);

        let a = call(&mut s, "explain", json!({"code": "NOPE-99"}));
        assert_eq!(a["result"]["isError"], true, "{a}");
        assert_eq!(a["result"]["structuredContent"]["ok"], false);

        let a = call(&mut s, "capabilities", json!({}));
        assert_eq!(a["result"]["structuredContent"]["result"]["format"], "scene-render-capabilities/1");
        let a = s
            .handle(&msg(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "capabilities"}})))
            .unwrap();
        assert_eq!(a["result"]["isError"], false, "absent arguments read as none");
    }
}
