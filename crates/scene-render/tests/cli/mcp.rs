//! `scene-render mcp` over its stdio transport: a session from initialize through tool calls, the errors of
//! unusable messages without the session ending, and no file written, the program cache included.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

fn corpus(p: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus").join(p).display().to_string()
}

fn modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules")
}

fn sha(name: &str) -> String {
    use sha2::Digest;
    let b = std::fs::read(modules().join(format!("{name}.wasm"))).unwrap();
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// A fresh directory for one test.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-cli-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Every file under `root`.
fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                found.push(p);
            }
        }
    }
    found
}

/// A document whose build-mode program runs when it loads, which stores its output in the program cache.
fn program_document(d: &Path) -> String {
    let body = format!(r#"<program id="p1" src="srep66-rect.wasm" sha256="{}"/>"#, sha("srep66-rect"));
    let file = d.join("doc.scene.xml");
    std::fs::write(
        &file,
        format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" seed="1"/><composition>{body}</composition></scene>"#
        ),
    )
    .unwrap();
    file.display().to_string()
}

/// The binary with `home` as its home and cache directory and no cache override.
fn binary(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_scene-render"));
    c.env("HOME", home).env("XDG_CACHE_HOME", home.join(".cache")).env_remove("SR_PROGRAM_CACHE").env("NO_COLOR", "1");
    c
}

/// Runs a session: writes `input` to the server's standard input from another thread, so that its answers are read
/// while it is written, then closes it.
fn session(home: &Path, input: Vec<u8>) -> Output {
    let mut child = binary(home)
        .arg("mcp")
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let o = child.wait_with_output().expect("the server ends");
    writer.join().unwrap().expect("the server reads its input");
    o
}

fn line(v: Value) -> Vec<u8> {
    let mut b = v.to_string().into_bytes();
    b.push(b'\n');
    b
}

fn call(id: u64, name: &str, arguments: Value) -> Vec<u8> {
    line(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}))
}

/// The envelope of a tool result, the same as its text content.
fn envelope(answer: &Value, command: &str) -> Value {
    let r = &answer["result"];
    let doc = r["structuredContent"].clone();
    assert_eq!(doc["format"], "scene-render-cli/1", "{answer}");
    assert_eq!(doc["command"], command);
    let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, doc);
    assert_eq!(r["isError"], doc["exit_code"] != 0);
    doc
}

#[test]
fn a_session_runs_read_only_tools_and_survives_unusable_messages() {
    let root = dir("session");
    let home = root.join("home");
    let docs = root.join("docs");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&docs).unwrap();
    let program = program_document(&docs);
    let modules = modules().display().to_string();

    let mut input = Vec::new();
    input.extend(line(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}})));
    input.extend(line(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})));
    input.extend(line(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})));
    input.extend(call(3, "validate", json!({"files": [corpus("valid/minimal.scene.xml")]})));
    input.extend(call(4, "validate", json!({"files": [program], "base_dir": modules})));
    input.extend(call(5, "inspect", json!({"file": corpus("invalid/c21.scene.xml")})));
    input.extend(call(6, "eval", json!({"file": corpus("valid/kitchen-sink.scene.xml"), "frame": 150})));
    input.extend(b"{\"jsonrpc\": \"2.0\", \"id\": 7, \"method\":\n");
    input.extend(line(json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"})));
    input.extend(vec![b' '; (1 << 20) + 1]);
    input.push(b'\n');
    input.extend(line(json!({"jsonrpc": "2.0", "id": 9, "method": "ping"})));
    input.extend(call(10, "render", json!({"file": program_document(&docs), "output": "out.png"})));

    let o = session(&home, input);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{stderr}");
    let answers: Vec<Value> = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    for a in &answers {
        assert_eq!(a["jsonrpc"], "2.0", "{a}");
    }
    let ids: Vec<Value> = answers.iter().map(|a| a["id"].clone()).collect();
    assert_eq!(
        ids,
        [
            json!(1),
            json!(2),
            json!(3),
            json!(4),
            json!(5),
            json!(6),
            Value::Null,
            json!(8),
            Value::Null,
            json!(9),
            json!(10)
        ]
    );

    let init = &answers[0]["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "scene-render");
    assert_eq!(init["capabilities"]["tools"]["listChanged"], false);

    let tools = answers[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["validate", "inspect", "eval", "captions", "capabilities", "explain"]);
    assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true && t["inputSchema"]["type"] == "object"));

    let v = envelope(&answers[2], "validate");
    assert_eq!((v["ok"].clone(), v["result"]["valid"].clone()), (json!(true), json!(true)), "{v}");
    let v = envelope(&answers[3], "validate");
    assert_eq!(v["ok"], true, "a document with a program validates: {v}");

    let v = envelope(&answers[4], "inspect");
    assert_eq!(answers[4]["result"]["isError"], true);
    assert_eq!(v["error"]["kind"], "invalid");
    assert_eq!(v["error"]["codes"][0], "C21");

    let v = envelope(&answers[5], "eval");
    assert_eq!(v["result"]["frame"], 150);
    assert!(v["result"]["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "hero"));

    assert_eq!(answers[6]["error"]["code"], -32700, "malformed JSON");
    assert_eq!(answers[7]["error"]["code"], -32601, "an unknown method");
    assert_eq!(answers[8]["error"]["code"], -32600, "an oversize message");
    assert_eq!(answers[9]["result"], json!({}), "the session goes on");
    assert_eq!(answers[10]["error"]["code"], -32602, "no tool renders");

    // nothing was written: no program output cached, no file beside the documents or in the working directory
    assert_eq!(files(&home), Vec::<PathBuf>::new());
    let mut written = files(&docs);
    written.sort();
    assert_eq!(written, [docs.join("doc.scene.xml")]);

    // and the same validation outside the server does store the program's output there
    let o = binary(&home).args(["validate", "--base-dir", &modules, &program]).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!files(&home.join(".cache/scene-render/program")).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_session_ends_when_its_input_closes_even_before_initialize() {
    let home = dir("eof");
    let o = session(&home, line(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})));
    assert_eq!(o.status.code(), Some(0));
    let a: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(a["error"]["code"], -32600, "tools before initialize: {a}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_format_flag_does_not_wrap_the_protocol() {
    let home = dir("format");
    let mut child = binary(&home)
        .args(["--format", "json", "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&line(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8(o.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    let a: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(a, json!({"jsonrpc": "2.0", "id": 1, "result": {}}), "a JSON-RPC answer, not an envelope");
    let _ = std::fs::remove_dir_all(&home);
}
