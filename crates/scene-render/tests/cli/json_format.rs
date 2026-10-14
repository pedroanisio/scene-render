//! `--format json` on every subcommand: one envelope `scene-render-cli/1` on standard output, valid against
//! `schema/cli-output-1.schema.json`, with the exit status of the text mode and a typed error.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

use crate::common::no_gpu;

fn corpus(p: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus").join(p).display().to_string()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

/// A fresh directory for one test.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-cli-json-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &std::path::Path, name: &str, text: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.display().to_string()
}

const SMALL: &str = r##"<scene version="1.1"><project width="32" height="24" fps="10" duration="0.5" background="#102030"/>
<composition><shape id="a" shape="rect" x="2" y="2" width="10" height="10" fill="#FF0000"/></composition></scene>"##;

/// The envelope a run printed: standard output is exactly one line holding one JSON document, valid against the
/// schema, whose exit_code is the process exit status and whose command is `command`.
fn envelope(o: &Output, command: Option<&str>) -> Value {
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        text.lines().count(),
        1,
        "one line on standard output: {text}\nstderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    let schema = schema();
    validate(&schema, &schema, &v, "").unwrap_or_else(|e| panic!("the envelope breaks the schema: {e}\n{v:#}"));
    assert_eq!(v["exit_code"].as_i64(), o.status.code().map(i64::from), "{v:#}");
    assert_eq!(v["command"], command.map_or(Value::Null, Value::from));
    v
}

fn kind(v: &Value) -> &str {
    v["error"]["kind"].as_str().unwrap_or_else(|| panic!("no error: {v:#}"))
}

#[test]
fn validate_reports_each_file_and_a_typed_failure() {
    let o = run(&["validate", "--format", "json", &corpus("invalid/r6.scene.xml"), &corpus("valid/minimal.scene.xml")]);
    assert_eq!(o.status.code(), Some(1));
    let v = envelope(&o, Some("validate"));
    assert_eq!(v["ok"], false);
    assert_eq!(v["result"]["valid"], false);
    assert_eq!(v["result"]["files"][0]["diagnostics"][0]["code"], "R6");
    assert_eq!(v["result"]["files"][1]["valid"], true);
    assert_eq!(kind(&v), "invalid");
    assert!(v["error"]["codes"].as_array().unwrap().contains(&Value::from("R6")), "{v:#}");

    let o = run(&["validate", "--json", &corpus("valid/minimal.scene.xml")]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("validate"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["error"], Value::Null);
    assert_eq!(v["schema_version"], "1.1.0");

    let o = run(&["validate", "--format=json", "/nonexistent/scene.xml"]);
    assert_eq!(o.status.code(), Some(2));
    let v = envelope(&o, Some("validate"));
    assert_eq!(kind(&v), "io");
    assert!(v["result"]["files"][0]["io_error"].is_string(), "{v:#}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot read"), "the message stays on standard error");
}

#[test]
fn inspect_gives_the_summary_and_the_model() {
    let o = run(&["inspect", "--format", "json", &corpus("valid/kitchen-sink.scene.xml")]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("inspect"));
    let r = &v["result"];
    assert_eq!(r["fps"], "30000/1001");
    assert_eq!(r["fps_ratio"]["num"], 30000);
    assert_eq!(r["frames"], 375);
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1920), Some(1080)));
    assert!(r["nodes"].as_u64().unwrap() > 0);
    assert_eq!(r["model"]["project"]["fps"], "30000/1001");

    let o = run(&["inspect", "--json", &corpus("invalid/c21.scene.xml")]);
    assert_eq!(o.status.code(), Some(1));
    let v = envelope(&o, Some("inspect"));
    assert_eq!(kind(&v), "invalid");
    assert_eq!(v["result"], Value::Null);
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "C21");
    assert!(d["file"].as_str().unwrap().ends_with("c21.scene.xml"), "{d}");
    assert_eq!(v["error"]["codes"][0], "C21");
}

#[test]
fn eval_gives_a_frame_or_a_benchmark() {
    let f = corpus("valid/kitchen-sink.scene.xml");
    let o = run(&["eval", &f, "--frame", "150", "--format", "json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("eval"));
    assert_eq!(v["result"]["frame"], 150);
    assert!(v["result"]["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "hero"));

    let o = run(&["eval", &f, "--bench", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("eval"));
    assert_eq!(v["result"]["frames"], 375);
    assert!(v["result"]["median_ms"].is_number());

    // a template error: exit 1 as in text mode, with its code
    let o = run(&["eval", &f, "--variant", "light", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let v = envelope(&o, Some("eval"));
    assert_eq!(kind(&v), "invalid");
    assert!(v["error"]["codes"].as_array().unwrap().contains(&Value::from("E10")), "{v:#}");
}

#[test]
fn captions_gives_the_tracks() {
    let d = dir("captions");
    let f = write(
        &d,
        "c.scene.xml",
        r#"<scene version="1.2"><project width="640" height="360" fps="24" duration="4"/><composition/><captions>
  <captionTrack id="src" language="en" preset="highlight" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb cc"/></captionTrack>
</captions></scene>"#,
    );
    let o = run(&["captions", &f, "--at", "0.5", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("captions"));
    assert_eq!(v["result"]["tracks"][0]["id"], "src");
    // the text mode prints the bare dump, not an envelope
    let text: Value = serde_json::from_slice(&run(&["captions", &f, "--at", "0.5"]).stdout).unwrap();
    assert_eq!(text, v["result"]);
}

#[test]
fn simulate_resolve_and_bake_volume_give_what_they_wrote() {
    let d = dir("simulate");
    let f = write(
        &d,
        "sim.scene.xml",
        r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="1"/><composition><shape id="box" shape="rect" width="10" height="10" fill="#FF0000"><rigidBody/></shape></composition><physics bounds="floor" cache="box.physics"/></scene>"##,
    );
    let o = run(&["simulate", &f, "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("simulate"));
    assert_eq!(v["result"]["sha256"].as_str().unwrap().len(), 64);
    assert!(v["result"]["cache"].as_str().unwrap().ends_with("box.physics"));
    // no cache to write: a usage error, exit 2 as in text mode
    let f =
        write(&d, "nocache.scene.xml", &std::fs::read_to_string(&f).unwrap().replace(r#" cache="box.physics""#, ""));
    let o = run(&["simulate", &f, "--json"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(kind(&envelope(&o, Some("simulate"))), "usage");

    let f = write(&d, "small.scene.xml", SMALL);
    let o = run(&["resolve", &f, "--check", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(envelope(&o, Some("resolve"))["result"]["targets"], serde_json::json!([]));

    let scene = write(
        &d,
        "source.scene.xml",
        r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="0.3"/><composition><object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" dt="0.1"><pyroSource radius="1.5" densityRate="1"/></pyro></object3D></composition></scene>"#,
    );
    let take = d.join("take").display().to_string();
    let o = run(&["bake-volume", &scene, "--object", "cloud", "-o", &take, "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("bake-volume"));
    assert_eq!(v["result"]["frames"], 3);
    assert_eq!(v["result"]["object"], "cloud");
    // the output exists: refused, and left as it was
    let o = run(&["bake-volume", &scene, "--object", "cloud", "-o", &take, "--json"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(kind(&envelope(&o, Some("bake-volume"))), "usage");
}

#[test]
fn explain_capabilities_and_completions_give_their_data() {
    let o = run(&["explain", "c21", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("explain"));
    assert_eq!(v["result"]["codes"][0]["code"], "C21");
    assert!(v["result"]["codes"][0]["test"].as_str().unwrap().contains("@shader"));
    let o = run(&["explain", "--json"]);
    assert!(envelope(&o, Some("explain"))["result"]["codes"].as_array().unwrap().len() > 100);
    let o = run(&["explain", "Z99", "--json"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(kind(&envelope(&o, Some("explain"))), "usage");

    let o = run(&["capabilities", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("capabilities"));
    let text: Value = serde_json::from_slice(&run(&["capabilities"]).stdout).unwrap();
    assert_eq!(v["result"], text, "the result is the manifest the text mode prints");

    let o = run(&["completions", "bash", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("completions"));
    assert_eq!(v["result"]["shell"], "bash");
    assert!(v["result"]["script"].as_str().unwrap().contains("scene-render"));
}

#[test]
fn scopes_reports_levels_or_an_io_error() {
    let d = dir("scopes");
    let still = d.join("grey.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([128, 128, 128])).save(&still).unwrap();
    let o = run(&["scopes", still.to_str().unwrap(), "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let v = envelope(&o, Some("scopes"));
    assert_eq!(v["result"]["schema"], "scene-render-scopes/1", "{v:#}");
    assert_eq!(v["result"]["frames"]["analysed"], 1, "{v:#}");

    let o = run(&["scopes", d.join("missing.png").to_str().unwrap(), "--format", "json"]);
    assert_eq!(o.status.code(), Some(2));
    let v = envelope(&o, Some("scopes"));
    assert_eq!(kind(&v), "io", "{v:#}");
    assert!(v["result"].is_null(), "{v:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn gpus_lists_the_adapters_or_says_none_is_available() {
    let o = run(&["gpus", "--json"]);
    let v = envelope(&o, Some("gpus"));
    if o.status.code() == Some(2) {
        assert_eq!(kind(&v), "unavailable", "{v:#}");
        assert_eq!(v["result"]["adapters"], serde_json::json!([]), "{v:#}");
        if no_gpu(&o) {
            return;
        }
    }
    assert_eq!(o.status.code(), Some(0), "{v:#}");
    let adapters = v["result"]["adapters"].as_array().unwrap();
    assert_eq!(adapters.iter().filter(|a| a["chosen"] == true).count(), 1, "{v:#}");
}

#[test]
fn render_and_encode_give_what_they_wrote_or_a_typed_failure() {
    let d = dir("render");
    let f = write(&d, "r.scene.xml", SMALL);
    let png = d.join("f.png").display().to_string();
    let o = run(&["render", &f, "--frame", "0", "-o", &png, "--json"]);
    if no_gpu(&o) {
        assert_eq!(kind(&envelope(&o, Some("render"))), "unavailable");
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("render"));
    assert_eq!(v["result"]["written"][0]["path"], png.as_str());
    assert_eq!(v["result"]["unsupported"], serde_json::json!([]));
    // a frame range on a single file name: a usage error
    let o = run(&["render", &f, "--frames", "0..3", "-o", &png, "--json"]);
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(kind(&envelope(&o, Some("render"))), "usage");

    // the document names no output: a usage error from encode
    let o = run(&["encode", &f, "--json"]);
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("encode"));
    assert_eq!(kind(&v), "usage");
    assert!(v["error"]["message"].as_str().unwrap().contains("<output>"), "{v:#}");
    if !crate::common::ffmpeg("render_and_encode_give_what_they_wrote_or_a_typed_failure") {
        return;
    }
    let pattern = d.join("e_%03d.png").display().to_string();
    let o = run(&["encode", &f, "-o", &pattern, "--end", "0.2", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("encode"));
    assert_eq!(v["result"]["outputs"].as_array().unwrap().len(), 1, "one document for every output: {v:#}");
    assert!(v["result"]["outputs"][0]["frames"].as_u64().is_some_and(|n| n > 0), "{v:#}");
}

#[test]
fn watch_prints_one_envelope_per_run() {
    let d = dir("watch");
    let f = write(&d, "w.scene.xml", SMALL);
    let png = d.join("w.png").display().to_string();
    let o = run(&["watch", &f, "--frames", "0..1", "-o", &png, "--max-runs", "1", "--json"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = envelope(&o, Some("watch"));
    assert_eq!(v["result"]["written"][0]["path"], png.as_str());
}

#[test]
fn a_command_line_that_does_not_parse_gives_a_usage_envelope() {
    let o = run(&["--json", "validate", "--no-such-flag", &corpus("valid/minimal.scene.xml")]);
    assert_eq!(o.status.code(), Some(2));
    let v = envelope(&o, Some("validate"));
    assert_eq!(kind(&v), "usage");
    assert!(v["error"]["message"].as_str().unwrap().contains("--no-such-flag"), "{v:#}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("--no-such-flag"), "clap's message stays on standard error");

    let o = run(&["--format", "json"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(kind(&envelope(&o, None)), "usage");

    let o = run(&["--format", "yaml", "validate", &corpus("valid/minimal.scene.xml")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty(), "JSON was not asked for");

    // help is not an error, and is text
    let o = run(&["--json", "validate", "--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("Usage:"));
}

#[test]
fn text_stays_the_default_and_both_spellings_agree() {
    let f = corpus("valid/minimal.scene.xml");
    let o = run(&["validate", &f]);
    assert!(serde_json::from_slice::<Value>(&o.stdout).is_err(), "text by default");
    let a = run(&["inspect", &f, "--json"]);
    let b = run(&["--format", "json", "inspect", &f]);
    assert_eq!(envelope(&a, Some("inspect")), envelope(&b, Some("inspect")));
    let o = run(&["inspect", &f, "--format", "text"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("version"));
    let o = run(&["inspect", &f, "--format", "human"]);
    assert!(serde_json::from_slice::<Value>(&o.stdout).is_err(), "human is text");
}

#[test]
fn every_subcommand_is_in_the_schema() {
    let help = String::from_utf8(run(&["--help"]).stdout).unwrap();
    let listed: Vec<String> = help
        .split("Commands:")
        .nth(1)
        .expect("a command list")
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        // a command starts two spaces in; a description clap wraps continues deeper
        .filter(|l| !l.starts_with("   "))
        .filter_map(|l| l.split_whitespace().next())
        // mcp answers in JSON-RPC, never with an envelope
        .filter(|c| *c != "help" && *c != "mcp")
        .map(String::from)
        .collect();
    assert!(listed.len() >= 14, "{listed:?}");
    let schema = schema();
    let known: Vec<&str> =
        schema["properties"]["command"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let mut missing: Vec<&String> = listed.iter().filter(|c| !known.contains(&c.as_str())).collect();
    missing.sort();
    assert!(missing.is_empty(), "subcommands the envelope schema does not name: {missing:?}");
    let extra: Vec<&&str> = known.iter().filter(|k| !listed.iter().any(|c| c == *k)).collect();
    assert!(extra.is_empty(), "the schema names commands the binary does not have: {extra:?}");
}

#[test]
fn the_schema_refuses_inconsistent_envelopes() {
    let schema = schema();
    let good = serde_json::json!({
        "format": "scene-render-cli/1", "schema_version": "1.0.0", "command": "explain", "ok": false, "exit_code": 2,
        "result": null, "diagnostics": [], "error": {"kind": "usage", "message": "unknown code", "codes": []}
    });
    validate(&schema, &schema, &good, "").unwrap();
    let broken = |path: &str, value: Value| {
        let mut v = good.clone();
        *v.pointer_mut(path).unwrap() = value;
        v
    };
    let cases = [
        ("an exit status the error kind does not give", broken("/exit_code", 1.into())),
        ("ok with an error", broken("/ok", true.into())),
        ("a failure without an error", broken("/error", Value::Null)),
        ("an unknown kind", broken("/error/kind", "oops".into())),
        ("an unknown command", broken("/command", "frobnicate".into())),
        ("another format", broken("/format", "scene-render-cli/2".into())),
        ("a version that is not semver", broken("/schema_version", "1.0".into())),
        ("an exit status out of range", broken("/exit_code", 3.into())),
        ("a result of the wrong shape", broken("/result", serde_json::json!({"codes": []}))),
    ];
    for (what, v) in cases {
        assert!(validate(&schema, &schema, &v, "").is_err(), "accepted {what}: {v}");
    }
    let mut extra = good.clone();
    extra["surprise"] = true.into();
    assert!(validate(&schema, &schema, &extra, "").is_err(), "accepted an undeclared key");
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("diagnostics");
    assert!(validate(&schema, &schema, &missing, "").is_err(), "accepted a missing key");
}

// ---- the JSON Schema of the envelope (schema/cli-output-1.schema.json) ----

fn schema() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/cli-output-1.schema.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The `pattern` of a schema string. A pattern this table does not know panics, so a new one is looked at.
fn pattern_holds(pattern: &str, value: &str) -> bool {
    match pattern {
        r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$" => {
            let parts: Vec<&str> = value.split('.').collect();
            parts.len() == 3
                && parts.iter().all(|p| {
                    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (*p == "0" || !p.starts_with('0'))
                })
        }
        other => panic!("the schema has a pattern this test does not know: {other}"),
    }
}

fn type_holds(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        other => panic!("type {other}"),
    }
}

/// Validates `value` against the keywords the schema uses; returns the first violation. A keyword it does not know
/// panics, so the schema cannot use one this check silently ignores.
fn validate(schema: &Value, root: &Value, value: &Value, at: &str) -> Result<(), String> {
    let obj = schema.as_object().expect("a schema object");
    for (key, rule) in obj {
        match key.as_str() {
            "$schema" | "$id" | "title" | "description" | "$defs" | "then" => {}
            "$ref" => {
                let name = rule.as_str().unwrap().strip_prefix("#/$defs/").expect("a local $defs reference");
                let def = root["$defs"].get(name).unwrap_or_else(|| panic!("no $defs/{name}"));
                validate(def, root, value, at)?;
            }
            "type" => {
                let ok = match rule {
                    Value::String(t) => type_holds(t, value),
                    Value::Array(ts) => ts.iter().any(|t| type_holds(t.as_str().unwrap(), value)),
                    other => panic!("type {other}"),
                };
                if !ok {
                    return Err(format!("{at}: not of type {rule}"));
                }
            }
            "const" if value != rule => return Err(format!("{at}: not {rule}")),
            "const" => {}
            "enum" if !rule.as_array().unwrap().contains(value) => return Err(format!("{at}: {value} not in {rule}")),
            "enum" => {}
            "minLength" => {
                if value.as_str().is_some_and(|s| (s.chars().count() as u64) < rule.as_u64().unwrap()) {
                    return Err(format!("{at}: shorter than {rule}"));
                }
            }
            "minItems" => {
                if value.as_array().is_some_and(|a| (a.len() as u64) < rule.as_u64().unwrap()) {
                    return Err(format!("{at}: fewer than {rule} items"));
                }
            }
            "minimum" => {
                if value.as_f64().is_some_and(|n| n < rule.as_f64().unwrap()) {
                    return Err(format!("{at}: below {rule}"));
                }
            }
            "pattern" => {
                if let Some(s) = value.as_str() {
                    if !pattern_holds(rule.as_str().unwrap(), s) {
                        return Err(format!("{at}: {s:?} does not match {rule}"));
                    }
                }
            }
            "required" => {
                if let Some(o) = value.as_object() {
                    for name in rule.as_array().unwrap() {
                        if !o.contains_key(name.as_str().unwrap()) {
                            return Err(format!("{at}: {name} is required"));
                        }
                    }
                }
            }
            "additionalProperties" => {
                assert_eq!(rule, &Value::Bool(false), "only additionalProperties: false is used");
                let known = obj.get("properties").and_then(Value::as_object);
                for name in value.as_object().into_iter().flat_map(|o| o.keys()) {
                    if !known.is_some_and(|k| k.contains_key(name)) {
                        return Err(format!("{at}: {name} is not declared"));
                    }
                }
            }
            "properties" => {
                for (name, sub) in rule.as_object().unwrap() {
                    if let Some(v) = value.get(name) {
                        validate(sub, root, v, &format!("{at}/{name}"))?;
                    }
                }
            }
            "items" => {
                for (i, v) in value.as_array().into_iter().flatten().enumerate() {
                    validate(rule, root, v, &format!("{at}/{i}"))?;
                }
            }
            "allOf" => {
                for sub in rule.as_array().unwrap() {
                    validate(sub, root, value, at)?;
                }
            }
            "anyOf" => {
                let errors: Vec<String> =
                    rule.as_array().unwrap().iter().filter_map(|sub| validate(sub, root, value, at).err()).collect();
                if errors.len() == rule.as_array().unwrap().len() {
                    return Err(format!("{at}: matches no alternative ({})", errors.join("; ")));
                }
            }
            "if" => {
                if validate(rule, root, value, at).is_ok() {
                    if let Some(then) = obj.get("then") {
                        validate(then, root, value, at)?;
                    }
                }
            }
            other => panic!("the schema uses a keyword this test does not know: {other}"),
        }
    }
    Ok(())
}
