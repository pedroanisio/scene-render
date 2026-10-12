//! SREP 87, Semantics 7: a value the format derives from an element's id (a per-element seed, a cell seed) uses the
//! element's local id when a build-mode program emitted it: `E`, not the `P.E` it has in the document. The picture of
//! a program's output then depends only on the output, and every document SREP 66 accepted draws the same pixels.
//!
//! Each document of a build (the main one and each include) has its own map: an id is mapped only by the document
//! that holds it, so a written id of one document that equals an emitted id of another stays as written.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

/// The namespaced ids of every element that a build-mode program emitted, with their local ids, per document.
#[derive(Debug, Clone, Default)]
pub struct LocalIds(Arc<Docs>);

#[derive(Debug, Clone, Default)]
struct Docs {
    /// By document index of the build (0 is the main document): namespaced id → local id.
    maps: Vec<HashMap<String, String>>,
    /// The scope (effective id of its include, "" for the main document) of each document with ids, and its index.
    scopes: HashMap<String, u16>,
}

impl LocalIds {
    /// The main document's `namespaced id → local id` (`sr_model::Document::program_ids`).
    pub fn new(ids: &HashMap<String, String>) -> Self {
        let mut l = LocalIds::default();
        l.add_document(0, "", ids);
        l
    }

    /// Adds the ids of the document `doc` of the build (an include), whose elements' effective ids begin with
    /// `scope/`.
    pub fn add_document(&mut self, doc: u16, scope: &str, ids: &HashMap<String, String>) {
        // registered even without ids: the parts of an effective id after `scope/` are this document's, and none
        // of them is mapped then
        let d = Arc::make_mut(&mut self.0);
        if d.maps.len() <= doc as usize {
            d.maps.resize_with(doc as usize + 1, HashMap::new);
        }
        d.maps[doc as usize] = ids.clone();
        d.scopes.insert(scope.to_string(), doc);
    }

    /// Whether no program emitted an element.
    pub fn is_empty(&self) -> bool {
        self.0.maps.iter().all(HashMap::is_empty)
    }

    /// The local id of the element `id` of document `doc`, when a program of that document emitted it.
    pub fn local_of(&self, doc: u16, id: &str) -> Option<&str> {
        self.0.maps.get(doc as usize)?.get(id).map(String::as_str)
    }

    /// The id that values derived from the element `id` of document `doc` use: its local id when a program
    /// emitted it, else `id`.
    pub fn element<'a>(&'a self, doc: u16, id: &'a str) -> &'a str {
        self.local_of(doc, id).unwrap_or(id)
    }

    /// The id that values derived from `id` use. `id` is an element id or an effective id: instance and include
    /// scopes joined with `/`, an anonymous node as `parent~nameN`, a copy as `id[k]`. Each id in it that a program
    /// emitted is replaced by its local id, in the map of the document that holds it (the part after an include's
    /// scope belongs to the included document); the rest is kept.
    pub fn seed_id<'a>(&self, id: &'a str) -> Cow<'a, str> {
        if self.is_empty() {
            return Cow::Borrowed(id);
        }
        let empty = HashMap::new();
        let map_of = |doc: u16| self.0.maps.get(doc as usize).unwrap_or(&empty);
        let mut map = map_of(0);
        let mut out = String::new();
        let mut changed = false;
        let mut start = 0;
        let delimiter = |c: char| matches!(c, '/' | '~' | '[' | ']');
        for (i, c) in id.char_indices().filter(|(_, c)| delimiter(*c)).chain(std::iter::once((id.len(), '/'))) {
            let part = &id[start..i];
            match map.get(part) {
                Some(local) => {
                    out.push_str(local);
                    changed = true;
                }
                None => out.push_str(part),
            }
            if i < id.len() {
                out.push(c);
            }
            if c == '/' {
                if let Some(doc) = self.0.scopes.get(&id[..i]).filter(|d| **d != 0) {
                    map = map_of(*doc);
                }
            }
            start = i + c.len_utf8();
        }
        if changed {
            Cow::Owned(out)
        } else {
            Cow::Borrowed(id)
        }
    }

    /// [`Self::seed_id`] of an id held in an `Arc`.
    pub fn seed_arc(&self, id: &Arc<str>) -> Arc<str> {
        match self.seed_id(id) {
            Cow::Borrowed(_) => id.clone(),
            Cow::Owned(s) => s.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn ids() -> LocalIds {
        LocalIds::new(&map(&[("p.g1", "g1"), ("p.h", "h")]))
    }

    #[test]
    fn every_emitted_id_of_an_effective_id_becomes_local() {
        let l = ids();
        assert_eq!(l.seed_id("p.g1"), "g1");
        assert_eq!(l.seed_id("i1/p.g1"), "i1/g1");
        assert_eq!(l.seed_id("p.g1~shape2"), "g1~shape2");
        assert_eq!(l.seed_id("p.h[3]"), "h[3]");
        assert_eq!(l.seed_id("i1/p.g1/p.h"), "i1/g1/h");
    }

    #[test]
    fn other_ids_are_borrowed_unchanged() {
        let l = ids();
        for id in ["g1", "p", "q.g1", "i1/g1", "p~shape1", "", "p.g1x"] {
            assert!(matches!(l.seed_id(id), Cow::Borrowed(s) if s == id), "{id}");
        }
        assert!(matches!(LocalIds::default().seed_id("p.g1"), Cow::Borrowed("p.g1")));
    }

    #[test]
    fn each_part_is_mapped_by_the_document_that_holds_it() {
        // the main document emits p.g1 and includes B as `inc`; B writes its own `p.g1` and emits q.k
        let mut l = ids();
        l.add_document(1, "inc", &map(&[("q.k", "k"), ("a.b.c", "c")]));
        assert_eq!(l.seed_id("inc/p.g1"), "inc/p.g1", "B's own p.g1 is written, not emitted");
        assert_eq!(l.seed_id("inc/q.k"), "inc/k");
        assert_eq!(l.seed_id("q.k"), "q.k", "the main document did not emit q.k");
        assert_eq!(l.seed_id("p.g1/inc/q.k~x1"), "g1/inc/q.k~x1", "inc is not the scope p.g1/inc");
        assert_eq!(l.element(0, "p.g1"), "g1");
        assert_eq!(l.element(1, "p.g1"), "p.g1");
        assert_eq!(l.element(1, "q.k"), "k");
        assert_eq!(l.element(7, "q.k"), "q.k");
        assert_eq!(l.local_of(1, "a.b.c"), Some("c"));
        assert_eq!(l.local_of(0, "a.b.c"), None);
    }

    #[test]
    fn an_include_of_a_document_without_programs_maps_nothing() {
        let mut l = ids();
        l.add_document(1, "inc", &HashMap::new());
        assert_eq!(l.seed_id("inc/p.g1"), "inc/p.g1", "inc's own p.g1 is written, not emitted");
        assert_eq!(l.seed_id("p.g1/x"), "g1/x");
    }
}
