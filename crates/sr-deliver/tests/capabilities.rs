//! SREP 22: the engine's capability manifest, `capabilities.json`, and the `SUP-*` findings a render report gets from
//! it.

use sr_deliver::capabilities::{findings, manifest, Construct, Entry, Manifest, Status, FORMAT, MANIFEST};

/// Every complex type an element of this name is declared with, anywhere in the schema.
fn element_types(name: &str) -> Vec<&'static sr_model::xsd::ComplexType> {
    use sr_model::xsd::{Content, Particle, COMPLEX_TYPES};
    let mut out = Vec::new();
    fn walk(p: &Particle, name: &str, out: &mut Vec<usize>) {
        match p {
            Particle::Element { name: n, ty, .. } => {
                if *n == name && !out.contains(ty) {
                    out.push(*ty);
                }
            }
            Particle::Seq { items, .. } | Particle::Choice { items, .. } => {
                items.iter().for_each(|i| walk(i, name, out));
            }
        }
    }
    let mut idx = Vec::new();
    for ct in COMPLEX_TYPES.iter() {
        if let Content::Elements(p) = ct.content {
            walk(p, name, &mut idx);
        }
    }
    out.extend(idx.into_iter().map(|i| &COMPLEX_TYPES[i]));
    out
}

/// Whether the schema has the construct: the element, the attribute on it, the value in the attribute's type.
fn in_schema(c: &Construct) -> bool {
    element_types(c.element).iter().any(|ct| match c.attribute {
        None => true,
        Some(a) => ct.attr(a).is_some_and(|d| c.value.is_none_or(|v| sr_model::xsd::simple::check(d.ty, v).is_ok())),
    })
}

#[test]
fn the_published_manifest_is_well_formed() {
    let m = manifest();
    assert_eq!(m.format, FORMAT);
    assert_eq!(m.engine.name, "rs-scene-render");
    assert_eq!(m.engine.version, env!("CARGO_PKG_VERSION"), "the manifest names this release");
    // the schema this engine vendors, as schema/UPSTREAM records its release
    let upstream =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/UPSTREAM"))
            .unwrap();
    let named = upstream
        .match_indices(&format!("release {}", m.schema))
        .any(|(i, t)| !upstream[i + t.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '.'));
    assert!(named, "schema {} is not the vendored release in schema/UPSTREAM", m.schema);
    // the engine accepts the vendored release's version, or the next one when schema/UPSTREAM records it as a local
    // addition ahead of a release: the clause that names SCHEMA_VERSION, up to its colon, calls it one
    let v = sr_model::SCHEMA_VERSION;
    let declared = format!("Document version {v} (sr_model::SCHEMA_VERSION is \"{v}\")");
    let local = upstream.find(&declared).is_some_and(|i| {
        let clause = &upstream[i + declared.len()..];
        clause[..clause.find(':').unwrap_or(clause.len())].contains("a local addition")
    });
    assert!(
        m.schema.starts_with(v) || local,
        "SCHEMA_VERSION {} is neither the manifest's schema {} nor a local addition in schema/UPSTREAM",
        sr_model::SCHEMA_VERSION,
        m.schema
    );
    let mut seen = std::collections::HashSet::new();
    for e in &m.entries {
        let c = Construct::parse(&e.construct).unwrap_or_else(|| panic!("{}: not a construct", e.construct));
        assert!(in_schema(&c), "{}: not in the scene schema", e.construct);
        assert_ne!(e.status, Status::Exact, "{}: an exact construct is not listed", e.construct);
        assert!(seen.insert(&e.construct), "{}: listed twice", e.construct);
    }
    // entries are sorted by construct, so the file diffs cleanly
    let names: Vec<&str> = m.entries.iter().map(|e| e.construct.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    // the JSON carries no field outside the format
    let v: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    let keys: std::collections::BTreeSet<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["engine", "entries", "format", "schema"].into_iter().collect());
}

#[test]
fn constructs_parse_in_the_three_forms_only() {
    let c = |s| Construct::parse(s);
    assert_eq!(c("pattern"), Some(Construct { element: "pattern", attribute: None, value: None }));
    assert_eq!(
        c("layer/@frameBlend"),
        Some(Construct { element: "layer", attribute: Some("frameBlend"), value: None })
    );
    assert_eq!(
        c("layer/@frameBlend=optical-flow"),
        Some(Construct { element: "layer", attribute: Some("frameBlend"), value: Some("optical-flow") })
    );
    for bad in ["", "a/b", "a/@", "a/@b=", "/@b", "a b"] {
        assert_eq!(c(bad), None, "{bad:?}");
    }
}

fn test_manifest() -> Manifest {
    let entry = |construct: &str, status| Entry {
        construct: construct.into(),
        status,
        definition: None,
        note: None,
        when: Vec::new(),
    };
    Manifest {
        format: FORMAT.into(),
        engine: sr_deliver::capabilities::Engine { name: "rs-scene-render".into(), version: "0.0.0".into() },
        schema: "1.3".into(),
        entries: vec![
            entry("layer/@frameBlend=optical-flow", Status::Reported),
            entry("pattern", Status::Approximate),
            entry("shape/@radius", Status::Unsupported),
        ],
    }
}

#[test]
fn srep_0022_reported() {
    // a scene using a construct the manifest lists as reported: SUP-REPORTED naming the construct, once
    let xml = r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><assets><video id="v" src="a.mp4" width="8" height="8" fps="1" duration="1"/></assets>
      <composition><layer id="a" asset="v" frameBlend="optical-flow"/><layer id="b" asset="v" frameBlend="optical-flow"/><layer id="c" asset="v" frameBlend="frame-mix"/></composition></scene>"#;
    let f = findings(&test_manifest(), xml);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].code.as_str(), f[0].severity), ("SUP-REPORTED", sr_model::Severity::Warning));
    assert!(f[0].message.contains("layer/@frameBlend=optical-flow"), "{}", f[0].message);
    assert_eq!(f[0].path, "/scene/composition/layer[1]");
}

#[test]
fn approximate_constructs_are_sup_approx_and_unused_ones_nothing() {
    let xml = r##"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><paints><pattern id="p" asset="img"/></paints><composition/></scene>"##;
    let f = findings(&test_manifest(), xml);
    assert_eq!(f.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(), ["SUP-APPROX"]);
    // an unsupported construct is not reported (the document could not use it), nor is anything the document lacks
    let none = r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><composition><shape id="s" shape="rect" width="4" height="4" radius="2"/></composition></scene>"#;
    assert!(findings(&test_manifest(), none).is_empty());
}

/// The note of a construct an accepted SREP adds and this engine does not implement yet.
fn pending_note(srep: u32) -> String {
    format!("SREP {srep} is accepted and not implemented by this engine yet")
}

#[test]
fn every_pending_construct_is_listed_as_reported_and_nothing_else_is_pending() {
    use sr_eval::pending::PENDING;
    let m = manifest();
    let listed = |construct: &str| m.entries.iter().find(|e| e.construct == construct);
    let mut pending_constructs = Vec::new();
    for p in PENDING {
        let constructs =
            p.elements.iter().map(|e| e.to_string()).chain(p.attributes.iter().map(|(e, a, _)| format!("{e}/@{a}")));
        for c in constructs {
            let e = listed(&c).unwrap_or_else(|| panic!("{c} (SREP {}) is not in capabilities.json", p.srep));
            assert_eq!(e.status, Status::Reported, "{c}");
            assert!(e.note.as_deref().is_some_and(|n| n.starts_with(&pending_note(p.srep))), "{c}: {:?}", e.note);
            pending_constructs.push(c);
        }
    }
    // an implemented SREP leaves both the table and the manifest
    for e in &m.entries {
        if e.note.as_deref().is_some_and(|n| n.contains("not implemented by this engine yet")) {
            assert!(pending_constructs.contains(&e.construct), "{} is listed as pending but has no row", e.construct);
        }
    }
}

#[test]
fn pathtrace_limitations_are_reported_on_the_affected_constructs_only() {
    // Conditional entries identify each affected construct only when the path tracer is also used.
    let scene = |camera: &str| {
        format!(
            r#"<scene version="1.2"><project width="32" height="32" fps="1" duration="1"/><materials><material id="m" unevenness="0.4"/></materials>
      <composition><camera id="c" {camera}/><object3D id="floor" primitive="plane" width="10" height="10" shadowCatcher="true" material="m"/></composition></scene>"#
        )
    };
    assert!(findings(&manifest(), &scene("")).is_empty(), "default renderer (raster)");
    assert!(findings(&manifest(), &scene(r#"renderer="raster""#)).is_empty());
    let f = findings(&manifest(), &scene(r#"renderer="pathtrace""#));
    assert_eq!(f.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(), ["SUP-APPROX", "SUP-REPORTED"]);
    assert!(f[0].message.contains("material/@unevenness"), "{}", f[0].message);
    assert_eq!(f[0].path, "/scene/materials/material");
    assert!(f[1].message.contains("object3D/@shadowCatcher=true"), "{}", f[1].message);
    assert_eq!(f[1].path, "/scene/composition/object3D");
    let exact =
        r#"<scene><composition><camera renderer="pathtrace"/><object3D primitive="plane"/></composition></scene>"#;
    assert!(findings(&manifest(), exact).is_empty(), "the path tracer alone is not a limitation");
}

#[test]
fn a_conditional_capability_requires_every_companion_construct() {
    let mut m = test_manifest();
    m.entries.retain(|e| e.construct == "pattern");
    m.entries[0].when = vec!["camera/@renderer=pathtrace".into(), "object3D/@shadowCatcher=true".into()];
    assert!(findings(&m, r#"<scene><pattern/><camera renderer="pathtrace"/></scene>"#).is_empty());
    assert_eq!(
        findings(&m, r#"<scene><pattern/><camera renderer="pathtrace"/><object3D shadowCatcher="true"/></scene>"#)
            .len(),
        1
    );
}

// ---- the vendored JSON Schema of the format (schema/capabilities-1.schema.json) ----

fn schema_json() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/capabilities-1.schema.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The `pattern` of a schema string, checked by the engine's own parser. A pattern this table does not know panics,
/// so a re-vendored schema with a new pattern is looked at, not skipped.
fn pattern_holds(pattern: &str, value: &str) -> bool {
    match pattern {
        // construct
        r"^[A-Za-z_][A-Za-z0-9_.-]*(/@[A-Za-z_][A-Za-z0-9_.-]*(=[^=\s]+)?)?$" => Construct::parse(value).is_some(),
        // schema version
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

/// Validates `value` against the keywords the schema uses; returns the first violation. A keyword it does not know
/// panics for the same reason as in `pattern_holds`.
fn validate(
    schema: &serde_json::Value,
    root: &serde_json::Value,
    value: &serde_json::Value,
    at: &str,
) -> Result<(), String> {
    use serde_json::Value;
    let obj = schema.as_object().expect("a schema object");
    for (key, rule) in obj {
        match key.as_str() {
            "$schema" | "$id" | "title" | "description" | "$defs" => {}
            "$ref" => {
                let name = rule.as_str().unwrap().strip_prefix("#/$defs/").expect("a local $defs reference");
                validate(&root["$defs"][name], root, value, at)?;
            }
            "type" => {
                let ok = match rule.as_str().unwrap() {
                    "object" => value.is_object(),
                    "array" => value.is_array(),
                    "string" => value.is_string(),
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
            "pattern" => {
                if let Some(s) = value.as_str() {
                    if !pattern_holds(rule.as_str().unwrap(), s) {
                        return Err(format!("{at}: {s:?} does not match {rule}"));
                    }
                }
            }
            "required" => {
                for name in rule.as_array().unwrap() {
                    if value.get(name.as_str().unwrap()).is_none() {
                        return Err(format!("{at}: {name} is required"));
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
            other => panic!("the schema uses a keyword this test does not know: {other}"),
        }
    }
    Ok(())
}

#[test]
fn the_published_manifest_validates_against_the_vendored_json_schema() {
    let schema = schema_json();
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    validate(&schema, &schema, &manifest, "").unwrap();
}

#[test]
fn the_json_schema_refuses_what_the_rust_structs_refuse() {
    let schema = schema_json();
    let good: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v = good.clone();
        f(&mut v);
        v
    };
    let cases = [
        ("a field outside the format", mutate(&|v| v["extra"] = "x".into())),
        ("an unknown status", mutate(&|v| v["entries"][0]["status"] = "partial".into())),
        ("a bad construct", mutate(&|v| v["entries"][0]["construct"] = "a b".into())),
        ("a bad schema version", mutate(&|v| v["schema"] = "1.7".into())),
        ("an empty when", mutate(&|v| v["entries"][0]["when"] = serde_json::json!([]))),
        (
            "a missing entries",
            mutate(&|v| {
                v.as_object_mut().unwrap().remove("entries");
            }),
        ),
    ];
    for (what, doc) in cases {
        assert!(validate(&schema, &schema, &doc, "").is_err(), "the JSON Schema accepts {what}");
        // the engine reads the same document or refuses it; it never reads one the schema refuses and reports it whole
        if let Ok(m) = serde_json::from_value::<Manifest>(doc.clone()) {
            assert!(
                what == "a bad construct" || what == "a bad schema version" || what == "an empty when",
                "the Rust structs accept {what}: {m:?}"
            );
        }
    }
}

#[test]
fn the_status_enum_of_the_json_schema_is_the_set_of_rust_statuses() {
    let schema = schema_json();
    let declared: std::collections::BTreeSet<String> = schema["properties"]["entries"]["items"]["properties"]["status"]
        ["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    // an exhaustive match: a new variant does not compile until it is listed here
    let all = [Status::Exact, Status::Approximate, Status::Reported, Status::Unsupported];
    for s in all {
        match s {
            Status::Exact | Status::Approximate | Status::Reported | Status::Unsupported => {}
        }
    }
    let rust: std::collections::BTreeSet<String> =
        all.iter().map(|s| serde_json::to_value(s).unwrap().as_str().unwrap().to_string()).collect();
    assert_eq!(declared, rust);
    assert_eq!(schema["properties"]["format"]["const"], FORMAT);
}
