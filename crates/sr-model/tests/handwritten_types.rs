//! The simple types of the schema whose Rust representation is written by hand (`HANDWRITTEN` in `build.rs`) accept what
//! the schema's facets accept.
//!
//! The validator is generated from the schema, so a document that reaches the model has passed the facets already. What
//! can drift is the hand-written parser behind it: a schema that widens a pattern while `values.rs` keeps the old
//! reading turns a valid document into a model error, and nothing in the schema changes to say so. Each type has samples
//! that are valid or not by the schema's own facets (a small interpreter of the schema below decides, and the table says
//! what it must decide), and the parser must take every valid one.
//!
//! The parser may take more than the schema does; the validator has refused those documents before the parser sees them.
//! `LENIENT` lists what is known to be taken, so a parser that becomes stricter or looser than that is seen.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use regex::Regex;
use sr_model::parse::ParseValue;
use sr_model::values::{Aspect, Color, Fps, LanguageTag, Length, Paint, PaintRef, Point2, Sha256, Timecode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn schema_text() -> String {
    std::fs::read_to_string(root().join("schema/scene-render-1.1.xsd")).unwrap()
}

/// The names of `HANDWRITTEN` in `build.rs`.
fn handwritten() -> BTreeSet<String> {
    let text = std::fs::read_to_string(root().join("crates/sr-model/build.rs")).unwrap();
    let start = text.find("const HANDWRITTEN").expect("HANDWRITTEN is gone from build.rs");
    let body = &text[start..];
    let body = &body[..body.find("];").unwrap()];
    let re = Regex::new(r#"^\s*\("([A-Za-z0-9@.]+)",\s*"[^"]+"\),"#).unwrap();
    body.lines().filter_map(|l| re.captures(l).map(|c| c[1].to_string())).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// A reading of the schema's simple types: restriction (pattern, length, maxLength, min/max, enumeration), list, union.
// Anything else is a facet this reading has not been written for and stops the test, so a schema that starts to use
// it is looked at.
// ---------------------------------------------------------------------------------------------------------------

struct Schema<'a> {
    doc: &'a roxmltree::Document<'a>,
}

impl<'a> Schema<'a> {
    fn named(&self, name: &str) -> roxmltree::Node<'a, 'a> {
        self.doc
            .descendants()
            .find(|n| n.has_tag_name("simpleType") && n.attribute("name") == Some(name))
            .unwrap_or_else(|| panic!("the schema has no simpleType {name}"))
    }

    fn accepts_named(&self, name: &str, text: &str) -> bool {
        match name.strip_prefix("xs:") {
            Some(builtin) => builtin_accepts(builtin, text),
            None => self.accepts(self.named(name), text),
        }
    }

    fn accepts(&self, simple: roxmltree::Node<'a, 'a>, text: &str) -> bool {
        let body = simple.children().find(|c| c.is_element() && c.tag_name().name() != "annotation").unwrap();
        match body.tag_name().name() {
            "union" => {
                let mut members: Vec<bool> = Vec::new();
                for name in body.attribute("memberTypes").unwrap_or("").split_whitespace() {
                    members.push(self.accepts_named(name, text));
                }
                for inline in body.children().filter(|c| c.has_tag_name("simpleType")) {
                    members.push(self.accepts(inline, text));
                }
                members.into_iter().any(|m| m)
            }
            "list" => self.items(body, text).iter().all(|(ok, _)| *ok),
            "restriction" => self.restriction(body, text),
            other => panic!("unread simpleType form {other}"),
        }
    }

    /// The whitespace-separated items of a list and whether each is accepted by the item type.
    fn items(&self, list: roxmltree::Node<'a, 'a>, text: &str) -> Vec<(bool, String)> {
        let inline = list.children().find(|c| c.has_tag_name("simpleType"));
        text.split_whitespace()
            .map(|item| {
                let ok = match (list.attribute("itemType"), inline) {
                    (Some(name), _) => self.accepts_named(name, item),
                    (None, Some(node)) => self.accepts(node, item),
                    _ => panic!("a list with no item type"),
                };
                (ok, item.to_string())
            })
            .collect()
    }

    fn restriction(&self, node: roxmltree::Node<'a, 'a>, text: &str) -> bool {
        let inline = node.children().find(|c| c.has_tag_name("simpleType"));
        let base_is_list = inline.is_some_and(|b| b.children().any(|c| c.has_tag_name("list")));
        // the base type: it must accept the text first
        let base_ok = match (node.attribute("base"), inline) {
            (Some(name), _) => self.accepts_named(name, text),
            (None, Some(base)) => self.accepts(base, text),
            _ => panic!("a restriction with no base"),
        };
        if !base_ok {
            return false;
        }
        let numeric = node.attribute("base").is_some_and(|b| matches!(b, "xs:double" | "xs:decimal" | "xs:integer"));
        let value = || parse_number(text.trim());
        let mut patterns: Vec<&str> = Vec::new();
        let mut enumeration: Vec<&str> = Vec::new();
        for facet in
            node.children().filter(|c| c.is_element() && !matches!(c.tag_name().name(), "simpleType" | "annotation"))
        {
            let v = facet.attribute("value").unwrap();
            match facet.tag_name().name() {
                "pattern" => patterns.push(v),
                "enumeration" => enumeration.push(v),
                "maxLength" => {
                    if text.chars().count() > v.parse::<usize>().unwrap() {
                        return false;
                    }
                }
                "length" => {
                    let count = if base_is_list { text.split_whitespace().count() } else { text.chars().count() };
                    if count != v.parse::<usize>().unwrap() {
                        return false;
                    }
                }
                "minInclusive" | "maxInclusive" | "minExclusive" | "maxExclusive" => {
                    assert!(numeric, "a numeric facet on a base this reading does not take as a number");
                    let bound: f64 = v.parse().unwrap();
                    let Some(x) = value() else { return false };
                    let ok = match facet.tag_name().name() {
                        "minInclusive" => x >= bound,
                        "maxInclusive" => x <= bound,
                        "minExclusive" => x > bound,
                        _ => x < bound,
                    };
                    if !ok {
                        return false;
                    }
                }
                other => panic!("unread facet {other}"),
            }
        }
        if !patterns.is_empty() {
            let any = patterns.iter().any(|p| anchored(p).is_match(text));
            if !any {
                return false;
            }
        }
        enumeration.is_empty() || enumeration.contains(&text)
    }
}

/// The pattern `p`, anchored the way a schema pattern is, compiled once.
fn anchored(p: &str) -> Regex {
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<String, Regex>> = Default::default();
    }
    CACHE.with(|c| {
        c.borrow_mut().entry(p.to_string()).or_insert_with(|| Regex::new(&format!("^(?:{p})$")).unwrap()).clone()
    })
}

fn parse_number(s: &str) -> Option<f64> {
    match s {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ => s.parse::<f64>().ok(),
    }
}

fn builtin_accepts(name: &str, text: &str) -> bool {
    match name {
        "string" | "anyURI" => true,
        "double" => {
            let t = text.trim();
            Regex::new(r"^(?:[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?|-?INF|NaN)$")
                .unwrap()
                .is_match(t)
        }
        "decimal" => Regex::new(r"^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)$").unwrap().is_match(text.trim()),
        "integer" => Regex::new(r"^[+-]?[0-9]+$").unwrap().is_match(text.trim()),
        other => panic!("unread built-in type xs:{other}"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The parsers, by schema type name.
// ---------------------------------------------------------------------------------------------------------------

fn rust_accepts(name: &str, text: &str) -> Option<bool> {
    fn ok<T: ParseValue>(text: &str) -> Option<bool> {
        Some(T::parse_value(text).is_ok())
    }
    match name {
        "fpsType" => ok::<Fps>(text),
        "aspectType" => ok::<Aspect>(text),
        "sha256Type" => ok::<Sha256>(text),
        "timecodeType" => ok::<Timecode>(text),
        "languageTagType" => ok::<LanguageTag>(text),
        "numberListType" | "unitTripleType" => ok::<Vec<f64>>(text),
        "colorListType" => ok::<Vec<Color>>(text),
        "pointType" => ok::<Point2>(text),
        "pointListType" => ok::<Vec<Point2>>(text),
        "relativeLength" | "positiveRelativeLength" | "lengthType" | "positiveLengthType" => ok::<Length>(text),
        "colorType" => ok::<Color>(text),
        "paintRefType" => ok::<PaintRef>(text),
        "paintType" => ok::<Paint>(text),
        _ => None,
    }
}

/// Types whose Rust representation takes any text, so there is no facet on the Rust side to compare. The schema checks
/// them; what is read afterwards is a `String` or a list of them.
const TEXT_ONLY: &[(&str, &str)] = &[
    ("volumeSourceType", "a String; the schema's union of anyURI and the sequence pattern is checked by the validator"),
    ("audioRoleListType", "a Vec<String>; each item is an enumeration member, checked by the validator"),
    ("expressionString", "a String; the expression language is compiled and diagnosed by sr-eval (E01)"),
    ("unitTripleType.base", "the base of unitTripleType, read as the same Vec<f64>"),
    ("densityTonemapType@reference", "an anonymous union kept as its text"),
    ("densityTonemapType@reference.member1", "a member of that union, kept as its text"),
];

/// Per type, how many values one edit from a valid sample the parser takes and the schema's facets (as interpreted
/// here, without whitespace collapsing) refuse. What the walk finds is mostly a leading or trailing space, a leading
/// `+`, a leading zero and a bare `.5` or `5.`: the parsers read numbers with Rust's number parsers and trim their
/// input, where the schema's patterns are narrower. The values reach a parser only after the schema validator has
/// passed the document, so these are a property of the parsers, not of what a document may contain. The counts pin
/// that: a parser that is tightened or loosened moves its count and has to be looked at.
const EDIT_LENIENT: &[(&str, usize)] = &[
    ("aspectType", 15),
    ("colorListType", 16),
    ("colorType", 259),
    ("fpsType", 15),
    ("languageTagType", 2744),
    ("lengthType", 10),
    ("paintRefType", 343),
    ("paintType", 107),
    ("pointListType", 8),
    ("pointType", 21),
    ("positiveLengthType", 27),
    ("positiveRelativeLength", 55),
    ("relativeLength", 54),
    ("sha256Type", 226),
    ("timecodeType", 12),
    ("unitTripleType", 123),
];

// ---------------------------------------------------------------------------------------------------------------
// Samples: (text, valid by the schema).
// ---------------------------------------------------------------------------------------------------------------

fn samples() -> Vec<(&'static str, Vec<(String, bool)>)> {
    let s = |v: &[(&str, bool)]| v.iter().map(|(t, b)| (t.to_string(), *b)).collect::<Vec<_>>();
    let sha = "0123456789abcdef".repeat(4);
    vec![
        (
            "fpsType",
            s(&[
                ("30", true),
                ("30000/1001", true),
                ("0", false),
                ("30/0", false),
                ("030", false),
                ("30/", false),
                ("29.97", false),
                ("-30", false),
                ("", false),
                ("30/1/2", false),
            ]),
        ),
        (
            "aspectType",
            s(&[
                ("16:9", true),
                ("1:1", true),
                ("0:9", false),
                ("16:0", false),
                ("16/9", false),
                ("16:", false),
                ("16:9:1", false),
            ]),
        ),
        (
            "sha256Type",
            vec![
                (sha.clone(), true),
                (sha.to_uppercase(), false),
                (sha[..63].to_string(), false),
                (format!("{sha}0"), false),
                (format!("0x{}", &sha[..62]), false),
                ("g".repeat(64), false),
            ],
        ),
        (
            "timecodeType",
            s(&[
                ("00:00:00:00", true),
                ("01:02:03;04", true),
                ("00:60:00:00", false),
                ("00:00:60:00", false),
                ("0:00:00:00", false),
                ("00:00:00:0", false),
                ("00:00:00.00", false),
                ("100:00:00:00", false),
            ]),
        ),
        (
            "languageTagType",
            s(&[
                ("en", true),
                ("en-US", true),
                ("zh-Hant-TW", true),
                ("e", false),
                ("en_US", false),
                ("en-", false),
                ("abcdefghi", false),
                ("en-123456789", false),
                ("en-abcdefgh-abcdefgh-abcdefgh-abcde", true),
                ("en-abcdefgh-abcdefgh-abcdefgh-abcdef", false),
            ]),
        ),
        (
            "numberListType",
            s(&[
                ("", true),
                ("1 2 3", true),
                ("1e3 -2.5 .5", true),
                ("INF -INF NaN", true),
                ("1,2", false),
                ("abc", false),
                ("0x10", false),
                ("1 two", false),
            ]),
        ),
        (
            "unitTripleType",
            s(&[
                ("0 0.5 1", true),
                ("1 0.5", false),
                ("0 0 0 0", false),
                ("-0.1 0 0", false),
                ("0 0 1.0001", false),
                ("a b c", false),
                ("", false),
            ]),
        ),
        (
            "colorListType",
            s(&[
                ("#FF0000 #00ff0080", true),
                ("1,0.5,0 #000000", true),
                ("", true),
                ("#F00", false),
                ("red", false),
                ("#GGGGGG", false),
            ]),
        ),
        (
            "pointType",
            s(&[
                ("0,0", true),
                ("-1.5,2e3", true),
                (".5,-.5", true),
                ("1,2,3", false),
                ("1 2", false),
                ("1,", false),
                ("a,b", false),
                ("1.,2", false),
            ]),
        ),
        ("pointListType", s(&[("0,0 1,1", true), ("", true), ("0,0 1", false), ("0,0;1,1", false)])),
        (
            "relativeLength",
            s(&[
                ("50%", true),
                ("-.5vw", true),
                ("10.vh", true),
                ("100vmin", true),
                ("0%", true),
                ("10px", false),
                ("10", false),
                ("%", false),
                ("1e2%", false),
                ("+5%", false),
                (".%", false),
            ]),
        ),
        (
            "positiveRelativeLength",
            s(&[
                ("50%", true),
                ("0.5vh", true),
                ("00.10vw", true),
                ("0%", false),
                ("0.0vw", false),
                ("-5%", false),
                ("10", false),
            ]),
        ),
        (
            "lengthType",
            s(&[
                ("10", true),
                ("-2.5e1", true),
                ("50%", true),
                ("INF", true),
                ("0%", true),
                ("10px", false),
                ("", false),
                ("1,5", false),
            ]),
        ),
        (
            "positiveLengthType",
            s(&[
                ("1", true),
                ("0.001", true),
                ("5%", true),
                ("INF", true),
                ("0", false),
                ("-1", false),
                ("0%", false),
                ("NaN", false),
            ]),
        ),
        (
            "colorType",
            s(&[
                ("#112233", true),
                ("#11223344", true),
                ("1,0.5,0", true),
                ("1,0.5,0,1", true),
                ("0,0,0", true),
                (".5,.5,.5", true),
                ("1.0,0,0", true),
                ("var(--accent)", true),
                ("#123", false),
                ("#12345", false),
                ("1,0.5", false),
                ("1,0.5,0,1,1", false),
                ("1.5,0,0", false),
                ("00.5,0,0", false),
                ("var(--)", false),
                ("var(accent)", false),
                ("#GG0000", false),
                ("red", false),
            ]),
        ),
        (
            "paintRefType",
            s(&[
                ("url(#grad)", true),
                ("url(#a.b-c)", true),
                ("url(#_x1)", true),
                ("url(#1a)", false),
                ("url(grad)", false),
                ("url(#)", false),
                ("#grad", false),
            ]),
        ),
        (
            "paintType",
            s(&[
                ("#000000", true),
                ("url(#g)", true),
                ("var(--x)", true),
                ("1,0,0", true),
                ("none", false),
                ("url(#)", false),
            ]),
        ),
    ]
}

/// `(type, text)` the parser is known to take although the schema refuses it. The parsers read a value; they do not
/// re-check its pattern, range or length, which the validator has checked before a value is read. The list is exact, so
/// a parser that starts to refuse one of these, or to take another, is seen here. (`relativeLength` and the other
/// length types share `Length`, which also reads a bare number: "10" is a length.)
const LENIENT: &[(&str, &str)] = &[
    ("colorType", "00.5,0,0"),
    ("colorType", "var(--)"),
    ("fpsType", "030"),
    ("languageTagType", "abcdefghi"),
    ("languageTagType", "e"),
    ("languageTagType", "en-"),
    ("languageTagType", "en-123456789"),
    ("languageTagType", "en-abcdefgh-abcdefgh-abcdefgh-abcdef"),
    ("languageTagType", "en_US"),
    ("paintRefType", "url(#1a)"),
    ("pointType", "1.,2"),
    ("positiveLengthType", "-1"),
    ("positiveLengthType", "0"),
    ("positiveLengthType", "0%"),
    ("positiveLengthType", "NaN"),
    ("positiveRelativeLength", "-5%"),
    ("positiveRelativeLength", "0%"),
    ("positiveRelativeLength", "0.0vw"),
    ("positiveRelativeLength", "10"),
    ("relativeLength", "+5%"),
    ("relativeLength", "10"),
    ("relativeLength", "1e2%"),
    ("sha256Type", "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"),
    ("unitTripleType", ""),
    ("unitTripleType", "-0.1 0 0"),
    ("unitTripleType", "0 0 0 0"),
    ("unitTripleType", "0 0 1.0001"),
    ("unitTripleType", "1 0.5"),
];

#[test]
fn the_interpreter_of_the_schema_gives_the_verdicts_the_table_states() {
    let text = schema_text();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let schema = Schema { doc: &doc };
    let mut wrong = Vec::new();
    for (name, rows) in samples() {
        for (sample, valid) in rows {
            if schema.accepts_named(name, &sample) != valid {
                wrong.push(format!("{name}: {sample:?} should be {}", if valid { "valid" } else { "invalid" }));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the samples and the schema disagree (the schema changed, or a sample is wrong):\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_parser_of_a_handwritten_type_takes_every_value_the_schema_accepts() {
    let mut refused = Vec::new();
    for (name, rows) in samples() {
        for (sample, valid) in rows {
            if valid && rust_accepts(name, &sample) == Some(false) {
                refused.push(format!("{name}: {sample:?}"));
            }
        }
    }
    assert!(refused.is_empty(), "values.rs refuses what the schema accepts:\n{}", refused.join("\n"));
}

/// The characters the schema's patterns are made of, and those of numbers and lists.
fn alphabet(schema_text: &str) -> Vec<char> {
    let doc = roxmltree::Document::parse(schema_text).unwrap();
    let mut set: BTreeSet<char> = "0123456789 ,.-+eE:;%#()".chars().collect();
    for n in doc.descendants().filter(|n| n.has_tag_name("pattern")) {
        let v = n.attribute("value").unwrap();
        let mut escaped = false;
        for c in v.chars() {
            if !escaped && c == '\\' {
                escaped = true;
                continue;
            }
            escaped = false;
            if c.is_ascii_graphic() && !"[]{}()*+?|^$".contains(c) {
                set.insert(c);
            }
        }
    }
    set.into_iter().collect()
}

/// A single edit of `s`: one character replaced, inserted or removed, over `alphabet`.
fn edits(s: &str, alphabet: &[char]) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    for i in 0..=chars.len() {
        for &a in alphabet {
            let mut ins = chars.clone();
            ins.insert(i, a);
            out.push(ins.into_iter().collect());
            if i < chars.len() {
                let mut rep = chars.clone();
                rep[i] = a;
                out.push(rep.into_iter().collect());
            }
        }
        if i < chars.len() {
            let mut del = chars.clone();
            del.remove(i);
            out.push(del.into_iter().collect());
        }
    }
    out
}

/// The samples probe the schema where it was read when the table was written. This walks one edit away from each valid
/// sample and asks the schema's own facets whether the result is valid: a pattern that has since been widened admits
/// some of them, and the parser must take what is admitted.
#[test]
fn the_parser_takes_what_a_widened_schema_would_admit_one_edit_from_a_sample() {
    let text = schema_text();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let schema = Schema { doc: &doc };
    let alphabet = alphabet(&text);
    let mut refused = BTreeSet::new();
    for (name, rows) in samples() {
        for (sample, valid) in &rows {
            if !valid {
                continue;
            }
            for edited in edits(sample, &alphabet) {
                if schema.accepts_named(name, &edited) && rust_accepts(name, &edited) == Some(false) {
                    refused.insert(format!("{name}: {edited:?}"));
                }
            }
        }
    }
    assert!(
        refused.is_empty(),
        "values.rs refuses what the schema accepts:\n{}",
        refused.into_iter().collect::<Vec<_>>().join("\n")
    );
}

/// The other direction of the walk: one edit away from a valid sample the schema refuses, and the parser takes it. The
/// samples list some of these by hand (`LENIENT`); this finds the rest, and pins them so a parser that grows more
/// permissive, or is tightened, shows up.
#[test]
fn what_the_parser_takes_one_edit_from_a_sample_beyond_the_schema_is_what_is_pinned() {
    let text = schema_text();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let schema = Schema { doc: &doc };
    let alphabet = alphabet(&text);
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    for (name, rows) in samples() {
        let mut seen = BTreeSet::new();
        for (sample, valid) in &rows {
            if !valid {
                continue;
            }
            for edited in edits(sample, &alphabet) {
                if !schema.accepts_named(name, &edited) && rust_accepts(name, &edited) == Some(true) {
                    seen.insert(edited);
                }
            }
        }
        if !seen.is_empty() {
            taken.insert(name.to_string(), seen.len());
        }
    }
    let pinned: BTreeMap<String, usize> = EDIT_LENIENT.iter().map(|(n, c)| (n.to_string(), *c)).collect();
    assert_eq!(
        taken, pinned,
        "left: types whose parser takes this many one-edit values the schema refuses; right: EDIT_LENIENT. \
         Tighten the parser, or record the new count with the reason"
    );
}

#[test]
fn what_the_parser_takes_beyond_the_schema_is_what_is_listed() {
    let mut taken: BTreeSet<(String, String)> = BTreeSet::new();
    for (name, rows) in samples() {
        for (sample, valid) in rows {
            if !valid && rust_accepts(name, &sample) == Some(true) {
                taken.insert((name.to_string(), sample));
            }
        }
    }
    let listed: BTreeSet<(String, String)> = LENIENT.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect();
    assert_eq!(
        taken, listed,
        "left: the parser takes these; right: the list. Tighten the parser or list the value with its reason"
    );
}

#[test]
fn every_handwritten_type_is_sampled_or_named_as_text() {
    let covered: BTreeSet<String> =
        samples().iter().map(|(n, _)| n.to_string()).chain(TEXT_ONLY.iter().map(|(n, _)| n.to_string())).collect();
    let missing: Vec<String> = handwritten().difference(&covered).cloned().collect();
    assert!(missing.is_empty(), "types hand-written in build.rs with no samples here: {missing:?}");
    let stale: Vec<String> = covered.difference(&handwritten()).cloned().collect();
    assert!(stale.is_empty(), "samples for types that build.rs no longer hand-writes: {stale:?}");
}

#[test]
fn every_sampled_type_has_a_valid_and_an_invalid_sample() {
    for (name, rows) in samples() {
        assert!(rows.iter().any(|(_, v)| *v), "{name} has no valid sample");
        assert!(rows.iter().any(|(_, v)| !*v), "{name} has no invalid sample");
    }
}

#[test]
fn a_sampled_type_has_a_parser() {
    for (name, _) in samples() {
        assert!(rust_accepts(name, "").is_some(), "{name} has samples and no parser in rust_accepts");
    }
}
