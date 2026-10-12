//! SREP 87: the ids a build-mode program emits are scoped by the program.
//!
//! An `xs:ID` value `E` in the output of the program whose id is `P` becomes `P.E` in the document (Semantics 2).
//! Inside the output, a reference to one of the output's own ids resolves to it (Semantics 3); any other bare
//! reference resolves to a document id, else to the one program that emitted it (W13), and is an error when several
//! did (PRG17) (Semantics 4). The references rewritten are the tokens of `xs:IDREF` and `xs:IDREFS` attributes, the
//! id part of the literal of `prop("id.property")` and `propAtTime("id.property", t)` (everything before the last
//! `.`), `override/@target` of a layout or a variant (from outside only), and — beyond the SREP's list — the
//! `nodeId.property` form of `link/@source`, which names a node exactly as `prop` does.
//!
//! The rewriting runs on the document text with every output in place, before that text is validated again
//! (Semantics 5), so the identity constraints of SREP 59 hold for the rewritten document.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use roxmltree::Node;

use crate::diag::{element_path, Diagnostic, Loc};
use crate::xsd::simple::collapse;
use crate::xsd::{derives_from, Content, IdKind};

/// A build-mode program in the composition: its id and its element in the original document.
pub(crate) struct Emitter<'a, 'i> {
    /// `@id` (`P`).
    pub id: String,
    /// The `<program>` element.
    pub node: Node<'a, 'i>,
}

/// Maps byte offsets of the text with the outputs in place back to the original document.
#[derive(Default)]
pub(crate) struct OffsetMap {
    /// Copied stretches: (range in the new text, start in the original).
    copied: Vec<(Range<usize>, usize)>,
    /// Single positions written anew (the group a program becomes, its attributes): new offset → original offset.
    anchors: HashMap<usize, usize>,
}

impl OffsetMap {
    /// `new` holds the original bytes from `orig` on.
    pub fn copied(&mut self, new: Range<usize>, orig: usize) {
        self.copied.push((new, orig));
    }

    /// The text at `new` stands for the original text at `orig`.
    pub fn anchor(&mut self, new: usize, orig: usize) {
        self.anchors.insert(new, orig);
    }

    fn orig(&self, new: usize) -> Option<usize> {
        if let Some(o) = self.anchors.get(&new) {
            return Some(*o);
        }
        self.copied.iter().find(|(r, _)| r.contains(&new)).map(|(r, o)| o + (new - r.start))
    }
}

/// Maps byte offsets of the expanded text (the outputs in place, the references scoped) back to the original
/// document, where they lie in copied text.
#[derive(Default)]
pub(crate) struct Origin {
    map: OffsetMap,
    /// The scoping edits: (range replaced in the text with the outputs in place, length of the new text), in order.
    edits: Vec<(Range<usize>, usize)>,
}

impl Origin {
    pub fn new(map: OffsetMap, edits: &[(Range<usize>, String)]) -> Origin {
        Origin { map, edits: edits.iter().map(|(r, s)| (r.clone(), s.len())).collect() }
    }

    /// The original offset of `out`, an offset of the expanded text; inside an edit, the start of what it replaced.
    pub fn orig(&self, out: usize) -> Option<usize> {
        let mut delta: isize = 0;
        let mut text = None;
        for (r, len) in &self.edits {
            let start = (r.start as isize + delta) as usize;
            if out < start {
                break;
            }
            if out < start + len {
                text = Some(r.start);
                break;
            }
            delta += *len as isize - r.len() as isize;
        }
        self.map.orig(text.unwrap_or_else(|| (out as isize - delta) as usize))
    }
}

/// What scoping the outputs changes.
#[derive(Default)]
pub(crate) struct Scoped {
    /// Replacements in the text with the outputs in place, by byte range.
    pub edits: Vec<(Range<usize>, String)>,
    /// W13 warnings.
    pub warnings: Vec<Diagnostic>,
    /// `xs:IDREF` tokens outside every output that name an id of the document after scoping: (offset of their
    /// attribute in the original document, token as written).
    pub resolved: HashSet<(usize, String)>,
    /// Namespaced id → local id of every element an output holds (Semantics 7).
    pub local_ids: HashMap<String, String>,
}

/// How a reference resolves.
enum Res {
    /// As written.
    Same,
    /// To an id of the program's own output (Semantics 3).
    Local(String),
    /// To the one program that emitted it (Semantics 4, W13): the new id and that program.
    Fallback(String, usize),
    /// Several programs emitted it (PRG17).
    Ambiguous(Vec<usize>),
}

struct Scope<'e, 'a, 'i> {
    emitters: &'e [Emitter<'a, 'i>],
    /// The local ids of each program.
    local: Vec<HashSet<String>>,
    /// Every id of the document after scoping.
    ids: HashSet<String>,
    /// Local id → the programs that emitted it.
    emitted_by: HashMap<String, Vec<usize>>,
}

impl Scope<'_, '_, '_> {
    fn resolve(&self, r: &str, k: Option<usize>) -> Res {
        if let Some(k) = k {
            if self.local[k].contains(r) {
                return Res::Local(format!("{}.{r}", self.emitters[k].id));
            }
        }
        if self.ids.contains(r) {
            return Res::Same;
        }
        match self.emitted_by.get(r).map(Vec::as_slice) {
            None | Some([]) => Res::Same,
            Some([q]) => Res::Fallback(format!("{}.{r}", self.emitters[*q].id), *q),
            Some(many) => Res::Ambiguous(many.to_vec()),
        }
    }
}

/// Where a finding about a reference is reported.
struct Site<'t> {
    /// The program whose output holds the reference, if any.
    owner: Option<usize>,
    /// Offset of the reference's attribute (or element) in the text with the outputs in place.
    at: usize,
    /// Offset of its element there.
    element: usize,
    /// "@matte of <shape>" and the like.
    what: &'t str,
}

struct Findings<'m, 'o, 'e, 'a, 'i> {
    map: &'m OffsetMap,
    original: &'o roxmltree::Document<'o>,
    emitters: &'e [Emitter<'a, 'i>],
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
}

impl Findings<'_, '_, '_, '_, '_> {
    fn place(&self, site: &Site) -> (Loc, String, String) {
        if let Some(k) = site.owner {
            let n = self.emitters[k].node;
            return (Loc::of(n), element_path(n), format!("program {:?}: in its output, ", self.emitters[k].id));
        }
        let loc = self.map.orig(site.at).map(Loc::at).unwrap_or_default();
        let path = self
            .map
            .orig(site.element)
            .and_then(|o| self.original.descendants().find(|n| n.is_element() && n.range().start == o))
            .map(element_path)
            .unwrap_or_else(|| "/".into());
        (loc, path, String::new())
    }

    /// The new value of reference `r` at `site`, after reporting what its resolution asks for.
    fn apply(&mut self, res: Res, r: &str, site: &Site) -> Option<String> {
        match res {
            Res::Same => None,
            Res::Local(id) => Some(id),
            Res::Fallback(id, q) => {
                let (loc, path, prefix) = self.place(site);
                self.warnings.push(
                    Diagnostic::warning(
                        "W13",
                        format!(
                            "{prefix}{} names {r:?}, which only program {:?} emits: it refers to {id:?} (SREP 87)",
                            site.what, self.emitters[q].id
                        ),
                        loc,
                        path,
                    )
                    .with_help(format!("write {id:?}")),
                );
                Some(id)
            }
            Res::Ambiguous(qs) => {
                let (loc, path, prefix) = self.place(site);
                let names: Vec<String> = qs.iter().map(|q| format!("{:?}", self.emitters[*q].id)).collect();
                self.errors.push(
                    Diagnostic::error(
                        "PRG17",
                        format!(
                            "{prefix}{} names {r:?}, which programs {} all emit (SREP 87)",
                            site.what,
                            names.join(", ")
                        ),
                        loc,
                        path,
                    )
                    .with_help(format!("write the program's id before it, as in \"{}.{r}\"", self.emitters[qs[0]].id)),
                );
                None
            }
        }
    }
}

/// Scopes the ids of the outputs of `emitters` in `text`, the document with every output in place. `outputs` are
/// the byte ranges of the outputs in `text`, each with the index of its program in `emitters`.
pub(crate) fn scope(
    text: &str,
    outputs: &[(Range<usize>, usize)],
    emitters: &[Emitter<'_, '_>],
    map: &OffsetMap,
    original: &roxmltree::Document<'_>,
) -> Result<Scoped, Vec<Diagnostic>> {
    if emitters.is_empty() {
        return Ok(Scoped::default());
    }
    let fail_all = |message: String| {
        emitters
            .iter()
            .map(|e| {
                Diagnostic::error(
                    "PRG14",
                    format!("program {:?}: {message}", e.id),
                    Loc::of(e.node),
                    element_path(e.node),
                )
            })
            .collect::<Vec<_>>()
    };
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| fail_all(format!("the document with the outputs in place is not well-formed: {e}")))?;
    let (_, types) = crate::xsd::structure::validate_typed(&doc, &mut Vec::new());
    let owner = |at: usize| outputs.iter().find(|(r, _)| r.contains(&at)).map(|(_, k)| *k);
    let kind = |e: Node, name: &str, inside: bool| match types.get(&e.id()).and_then(|t| t.attr(name)) {
        Some(d) => (d.id, Some(d.ty)),
        // an element its place does not accept: its `id` is still an id, for the checks of Semantics 1 and 2
        None if inside && name == "id" => (IdKind::Id, None),
        None => (IdKind::None, None),
    };
    let elements: Vec<Node> = doc.descendants().filter(Node::is_element).collect();

    // Semantics 1 and 2: the local ids of each output, and every id of the document after scoping
    let mut errors = Vec::new();
    let mut local: Vec<HashSet<String>> = vec![HashSet::new(); emitters.len()];
    let mut ids: HashSet<String> = HashSet::new();
    let mut inside: Vec<(usize, String)> = Vec::new();
    for e in &elements {
        let k = owner(e.range().start);
        for a in e.attributes() {
            if kind(*e, a.name(), k.is_some()).0 != IdKind::Id {
                continue;
            }
            let v = collapse(a.value());
            match k {
                None => {
                    ids.insert(v);
                }
                Some(k) => {
                    if !local[k].insert(v.clone()) {
                        let p = &emitters[k];
                        let message = format!(
                            "program {:?}: its output has two elements with the id {v:?} (SREP 87, Semantics 1)",
                            p.id
                        );
                        errors.push(Diagnostic::error("PRG14", message, Loc::of(p.node), element_path(p.node)));
                    } else {
                        inside.push((k, v));
                    }
                }
            }
        }
    }
    let mut local_ids = HashMap::new();
    let mut made_by: HashMap<String, usize> = HashMap::new();
    let mut emitted_by: HashMap<String, Vec<usize>> = HashMap::new();
    for (k, v) in &inside {
        let p = &emitters[*k];
        let ns = format!("{}.{v}", p.id);
        let clash = if ids.contains(&ns) {
            Some("another element of the document has that id".to_string())
        } else {
            made_by.get(&ns).map(|j| format!("program {:?} also produces it", emitters[*j].id))
        };
        if let Some(why) = clash {
            let message = format!("program {:?}: its id {v:?} becomes {ns:?}, and {why} (SREP 87, Semantics 2)", p.id);
            errors.push(Diagnostic::error("PRG14", message, Loc::of(p.node), element_path(p.node)));
            continue;
        }
        made_by.insert(ns.clone(), *k);
        let by = emitted_by.entry(v.clone()).or_default();
        if !by.contains(k) {
            by.push(*k);
        }
        local_ids.insert(ns, v.clone());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    ids.extend(made_by.into_keys());
    let scope = Scope { emitters, local, ids, emitted_by };

    // Semantics 3 and 4: the references
    let mut f = Findings { map, original, emitters, errors: Vec::new(), warnings: Vec::new() };
    let mut edits = Vec::new();
    let mut resolved = HashSet::new();
    for e in &elements {
        let k = owner(e.range().start);
        let name = e.tag_name().name();
        let parent = e.parent_element().map(|p| p.tag_name().name()).unwrap_or("");
        for a in e.attributes() {
            let (role, ty) = kind(*e, a.name(), k.is_some());
            let what = format!("@{} of <{name}>", a.name());
            let site = Site { owner: k, at: a.range().start, element: e.range().start, what: &what };
            let new = match role {
                IdKind::Id => k.map(|k| format!("{}.{}", emitters[k].id, collapse(a.value()))),
                IdKind::IdRef | IdKind::IdRefs => {
                    let written = collapse(a.value());
                    let mut changed = false;
                    let mut tokens = Vec::new();
                    for t in written.split(' ').filter(|t| !t.is_empty()) {
                        let to = f.apply(scope.resolve(t, k), t, &site);
                        let now = to.clone().unwrap_or_else(|| t.to_string());
                        if k.is_none() && scope.ids.contains(&now) {
                            if let Some(o) = map.orig(a.range().start) {
                                resolved.insert((o, t.to_string()));
                            }
                        }
                        changed |= to.is_some();
                        tokens.push(now);
                    }
                    changed.then(|| tokens.join(" "))
                }
                IdKind::None if ty.is_some_and(|t| derives_from(t, "expressionString")) => {
                    rewrite_expression(a.value(), &mut |id| f.apply(scope.resolve(id, k), id, &site))
                }
                IdKind::None if name == "link" && a.name() == "source" => {
                    let v = a.value();
                    let special = ["param:", "audio:", "marker:"].iter().any(|p| v.starts_with(p));
                    match v.rsplit_once('.') {
                        Some((id, prop)) if !special => {
                            f.apply(scope.resolve(id, k), id, &site).map(|id| format!("{id}.{prop}"))
                        }
                        _ => None,
                    }
                }
                IdKind::None
                    if name == "override"
                        && a.name() == "target"
                        && k.is_none()
                        && matches!(parent, "layout" | "variant") =>
                {
                    let t = collapse(a.value());
                    f.apply(scope.resolve(&t, None), &t, &site)
                }
                IdKind::None => None,
            };
            if let Some(v) = new {
                edits.push((a.range_value(), escape_attr(&v)));
            }
        }
        let expression_text = types
            .get(&e.id())
            .is_some_and(|t| matches!(t.content, Content::Simple(ty) if derives_from(ty, "expressionString")));
        if expression_text {
            if let Some(range) = content_range(text, *e) {
                let what = format!("the expression of <{name}>");
                let site = Site { owner: k, at: e.range().start, element: e.range().start, what: &what };
                let src: String = e.children().filter(Node::is_text).filter_map(|t| t.text()).collect();
                if let Some(new) = rewrite_expression(&src, &mut |id| f.apply(scope.resolve(id, k), id, &site)) {
                    edits.push((range, escape_text(&new)));
                }
            }
        }
    }
    if !f.errors.is_empty() {
        return Err(f.errors);
    }
    edits.sort_by_key(|(r, _)| r.start);
    Ok(Scoped { edits, warnings: f.warnings, resolved, local_ids })
}

/// The byte range of the content of `e` in `text`, between its start tag and its end tag.
fn content_range(text: &str, e: Node) -> Option<Range<usize>> {
    let r = e.range();
    let src = &text[r.clone()];
    let mut quote = None;
    let mut gt = None;
    for (i, c) in src.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => {
                gt = Some(i);
                break;
            }
            None => {}
        }
    }
    let gt = gt?;
    if src[..gt].ends_with('/') {
        return None;
    }
    let close = src.rfind("</")?;
    (close > gt).then(|| r.start + gt + 1..r.start + close)
}

fn escape_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\n' => o.push_str("&#10;"),
            '\t' => o.push_str("&#9;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

/// A token of the expression language, as far as finding the literals of `prop` and `propAtTime` needs.
#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// A string literal: its range (quotes included), quote and decoded value.
    Str(Range<usize>, char, String),
    Punct(char),
    Num,
}

/// The tokens of `src` up to the first lexical error (sr-eval's lexer: `//` and `/* */` comments, `'` and `"`
/// strings with backslash escapes, identifiers of letters, digits, `_` and `$`).
fn tokens(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = src[i..].chars().next() {
        let rest = &src[i..];
        if c.is_whitespace() {
            i += c.len_utf8();
        } else if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if let Some(body) = rest.strip_prefix("/*") {
            match body.find("*/") {
                Some(e) => i += e + 4,
                None => break,
            }
        } else if c == '"' || c == '\'' {
            let mut value = String::new();
            let mut end = None;
            let mut it = rest.char_indices().skip(1);
            while let Some((j, ch)) = it.next() {
                if ch == c {
                    end = Some(j + 1);
                    break;
                }
                if ch == '\n' {
                    break;
                }
                if ch == '\\' {
                    let Some((_, e)) = it.next() else { break };
                    value.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                } else {
                    value.push(ch);
                }
            }
            let Some(end) = end else { break };
            out.push(Tok::Str(i..i + end, c, value));
            i += end;
        } else if c.is_ascii_digit() || (c == '.' && rest[1..].starts_with(|d: char| d.is_ascii_digit())) {
            i += rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.')).unwrap_or(rest.len());
            out.push(Tok::Num);
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let len = rest.find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$')).unwrap_or(rest.len());
            out.push(Tok::Ident(rest[..len].to_string()));
            i += len;
        } else {
            out.push(Tok::Punct(c));
            i += c.len_utf8();
        }
    }
    out
}

/// Rewrites the id part (before the last `.`) of every literal of `prop(…)` and `propAtTime(…, t)` in `src` by `f`,
/// which returns the new id or `None` to keep it. `None` when nothing changes.
fn rewrite_expression(src: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    let toks = tokens(src);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let run = |from: usize, step: isize, c: char| {
        let mut n = 0;
        let mut i = from as isize;
        while i >= 0 && toks.get(i as usize) == Some(&Tok::Punct(c)) {
            n += 1;
            i += step;
        }
        n
    };
    for (j, t) in toks.iter().enumerate() {
        let Tok::Ident(name) = t else { continue };
        if !(name == "prop" || name == "propAtTime") {
            continue;
        }
        // grouping parentheses are transparent to the parser: `(prop)("g1.x")` and `prop(("g1.x"))` are
        // `prop("g1.x")`. The callee's closing parentheses must each have an opening one before it
        let wrapped = run(j + 1, 1, ')');
        if wrapped > if j == 0 { 0 } else { run(j - 1, -1, '(') } {
            continue;
        }
        if j > wrapped && toks[j - wrapped - 1] == Tok::Punct('.') {
            continue;
        }
        let open = j + wrapped + 1;
        if toks.get(open) != Some(&Tok::Punct('(')) {
            continue;
        }
        let inner = run(open + 1, 1, '(');
        let Some(Tok::Str(range, quote, value)) = toks.get(open + 1 + inner) else { continue };
        // the literal is the whole argument: its groups close, then the argument list goes on or ends
        let after = open + 2 + inner;
        if run(after, 1, ')') < inner
            || !matches!(toks.get(after + inner), Some(Tok::Punct(')')) | Some(Tok::Punct(',')))
        {
            continue;
        }
        let Some((id, prop)) = value.rsplit_once('.') else { continue };
        if let Some(new) = f(id) {
            let mut lit = String::new();
            lit.push(*quote);
            for c in format!("{new}.{prop}").chars() {
                match c {
                    '\\' => lit.push_str("\\\\"),
                    '\n' => lit.push_str("\\n"),
                    '\r' => lit.push_str("\\r"),
                    '\t' => lit.push_str("\\t"),
                    c if c == *quote => {
                        lit.push('\\');
                        lit.push(c);
                    }
                    c => lit.push(c),
                }
            }
            lit.push(*quote);
            edits.push((range.clone(), lit));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(src.len() + 16);
    let mut at = 0;
    for (r, s) in edits {
        out.push_str(&src[at..r.start]);
        out.push_str(&s);
        at = r.end;
    }
    out.push_str(&src[at..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(src: &str) -> Option<String> {
        rewrite_expression(src, &mut |id| (id == "g1").then(|| "p.g1".to_string()))
    }

    #[test]
    fn prop_and_prop_at_time_literals_are_rewritten_by_their_id_part() {
        assert_eq!(rename(r#"prop("g1.x") + 1"#).as_deref(), Some(r#"prop("p.g1.x") + 1"#));
        assert_eq!(rename("propAtTime('g1.x', time - 1)").as_deref(), Some("propAtTime('p.g1.x', time - 1)"));
        assert_eq!(rename(r#"prop ( "g1.opacity" )"#).as_deref(), Some(r#"prop ( "p.g1.opacity" )"#));
    }

    #[test]
    fn the_last_dot_splits_the_id_from_the_property() {
        // "a.g1.x" names the property x of "a.g1", not of "a"
        let mut seen = Vec::new();
        rewrite_expression(r#"prop("a.g1.x")"#, &mut |id| {
            seen.push(id.to_string());
            None
        });
        assert_eq!(seen, ["a.g1"]);
    }

    #[test]
    fn other_strings_names_and_comments_are_left_alone() {
        assert_eq!(rename(r#""g1.x" + param("g1.x")"#), None);
        assert_eq!(
            rename(
                r#"// prop("g1.x")
            1"#
            ),
            None
        );
        assert_eq!(rename(r#"/* prop("g1.x") */ 1"#), None);
        assert_eq!(rename(r#"Math.prop("g1.x")"#), None);
        assert_eq!(rename(r#"prop("g2.x")"#), None);
        assert_eq!(rename(r#"prop("nodot")"#), None);
        assert_eq!(rename(r#"prop(("g1.x") + "")"#), None, "not a literal argument");
        assert_eq!(rename(r#"prop)("g1.x")"#), None, "an unbalanced callee");
        assert_eq!(rename(r#"Math.(prop)("g1.x")"#), None);
    }

    #[test]
    fn grouping_parentheses_around_the_callee_or_the_literal_are_seen_through() {
        assert_eq!(rename(r#"prop(("g1.x"))"#).as_deref(), Some(r#"prop(("p.g1.x"))"#));
        assert_eq!(rename(r#"((prop))( ( "g1.x" ) )"#).as_deref(), Some(r#"((prop))( ( "p.g1.x" ) )"#));
        assert_eq!(rename(r#"propAtTime(("g1.x"), 1)"#).as_deref(), Some(r#"propAtTime(("p.g1.x"), 1)"#));
        assert_eq!(rename(r#"f(prop("g1.x"))"#).as_deref(), Some(r#"f(prop("p.g1.x"))"#));
        assert_eq!(rename(r#"(prop("g1.x"))"#).as_deref(), Some(r#"(prop("p.g1.x"))"#));
    }

    #[test]
    fn escapes_in_literals_are_decoded_and_kept_valid() {
        assert_eq!(rename(r#"prop("g\x31.x")"#), None, "\\x is not an escape of the language: the id is gx31");
        assert_eq!(rename(r#"prop("\g1.x")"#).as_deref(), Some(r#"prop("p.g1.x")"#));
    }

    #[test]
    fn the_content_of_an_element_lies_between_its_tags() {
        let text = r#"<a><expression property="x" note='>'>prop("g1.x")</expression><b/></a>"#;
        let doc = roxmltree::Document::parse(text).unwrap();
        let e = doc.descendants().find(|n| n.has_tag_name("expression")).unwrap();
        assert_eq!(&text[content_range(text, e).unwrap()], r#"prop("g1.x")"#);
        let b = doc.descendants().find(|n| n.has_tag_name("b")).unwrap();
        assert_eq!(content_range(text, b), None);
    }
}
