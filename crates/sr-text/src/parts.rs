//! Tagged formula parts of the semantic-tex draft of sr-core (`srep/srep-0000-semantic-tex.md`, Semantics 1 to 4):
//! `\part{name}{body}` in a formula's TeX, the formula the typesetter sees with the tags removed, the matching of
//! parts between two formulas, and the motion of a matched part and the fade of an unmatched one. Every function is
//! a pure function of its arguments.

/// A tagged part: its name, its TeX (the body, tags included), the number of parts that contain it, and its index
/// among the parts in order of their opening tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub tex: String,
    pub depth: usize,
    pub index: usize,
}

const TAG: &str = "\\part{";

/// The byte index after the escape that starts at `s[i] == '\\'`: the backslash and the whole next character.
fn skip_escape(s: &str, i: usize) -> usize {
    i + 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8)
}

/// The byte index just past the group that opens at `s[i] == '{'`, honouring nesting and backslash escapes.
fn group_end(s: &str, i: usize) -> Result<usize, String> {
    let b = s.as_bytes();
    let (mut depth, mut j) = (0i32, i);
    while j < b.len() {
        match b[j] {
            b'\\' => {
                j = skip_escape(s, j);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    Err("unclosed group".into())
}

fn is_nmtoken(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
}

/// Every `\part{name}{body}` as (name, start of the tag, start of the body, end past the body), by position.
fn scan(s: &str) -> Result<Vec<(String, usize, usize, usize)>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(k) = s[i..].find(TAG) {
        let start = i + k;
        let name_end = group_end(s, start + TAG.len() - 1)?;
        let name = &s[start + TAG.len()..name_end - 1];
        if !is_nmtoken(name) {
            return Err(format!("part name {name:?} is not an NMTOKEN"));
        }
        if s.as_bytes().get(name_end) != Some(&b'{') {
            return Err(format!("part {name:?} has no body group"));
        }
        let end = group_end(s, name_end)?;
        out.push((name.to_string(), start, name_end + 1, end));
        // continue inside the body, so nested parts are found
        i = name_end + 1;
    }
    Ok(out)
}

/// Rule 1: the tagged parts of a formula in order of their opening tag, with their depth. Names are NMTOKENs and
/// unique in the formula; a tag without its two groups, an unclosed group or a stray closing brace is an error.
pub fn parts(tex: &str) -> Result<Vec<Part>, String> {
    let b = tex.as_bytes();
    let (mut depth, mut j) = (0i32, 0);
    while j < b.len() {
        if b[j] == b'\\' {
            j = skip_escape(tex, j);
            continue;
        }
        depth += match b[j] {
            b'{' => 1,
            b'}' => -1,
            _ => 0,
        };
        if depth < 0 {
            return Err("stray closing brace".into());
        }
        j += 1;
    }
    if depth != 0 {
        return Err("unclosed group".into());
    }
    let found = scan(tex)?;
    let mut seen = std::collections::BTreeSet::new();
    let dup: std::collections::BTreeSet<_> = found.iter().filter(|f| !seen.insert(&f.0)).map(|f| &f.0).collect();
    if !dup.is_empty() {
        return Err(format!("duplicate part names: {dup:?}"));
    }
    Ok(found
        .iter()
        .enumerate()
        .map(|(index, (name, start, body, end))| Part {
            name: name.clone(),
            tex: tex[*body..end - 1].to_string(),
            depth: found.iter().filter(|o| o.1 < *start && o.3 >= *end).count(),
            index,
        })
        .collect())
}

/// Rule 1: the formula the typesetter sees: every `\part{name}{body}` replaced by `{body}`, so a part is a plain
/// group with the same layout.
pub fn strip_parts(tex: &str) -> Result<String, String> {
    parts(tex)?;
    let mut tex = tex.to_string();
    while let Some(i) = tex.find(TAG) {
        let name_end = group_end(&tex, i + TAG.len() - 1)?;
        tex.replace_range(i..name_end, "");
    }
    Ok(tex)
}

/// What `formulaMorph/@match` pairs parts by (rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBy {
    Name,
    Tex,
}

/// Rule 2: the pairs of outermost parts (source, target), the source parts left out and the target parts incoming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matching {
    pub matched: Vec<(Part, Part)>,
    pub out: Vec<Part>,
    pub incoming: Vec<Part>,
}

fn norm(tex: &str) -> String {
    tex.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Rule 2: with [`MatchBy::Name`] the outermost parts with the same name are paired; with [`MatchBy::Tex`] they are
/// paired in order when their TeX is the same up to whitespace, each part at most once. Nested parts move with their
/// container and take no part.
pub fn match_parts(a: &[Part], b: &[Part], by: MatchBy) -> Matching {
    let a: Vec<&Part> = a.iter().filter(|p| p.depth == 0).collect();
    let b: Vec<&Part> = b.iter().filter(|p| p.depth == 0).collect();
    let mut taken = vec![false; b.len()];
    let mut matched = Vec::new();
    let mut out = Vec::new();
    for p in a {
        let hit = match by {
            MatchBy::Name => b.iter().position(|q| q.name == p.name),
            MatchBy::Tex => (0..b.len()).find(|&k| !taken[k] && norm(&b[k].tex) == norm(&p.tex)),
        };
        match hit {
            Some(k) => {
                taken[k] = true;
                matched.push((p.clone(), b[k].clone()));
            }
            None => out.push(p.clone()),
        }
    }
    let incoming = b.iter().zip(&taken).filter(|(_, t)| !**t).map(|(q, _)| (*q).clone()).collect();
    Matching { matched, out, incoming }
}

/// How a matched pair goes from source to target (rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairKind {
    /// The same TeX up to whitespace: the source glyphs move and scale onto the target box.
    Move,
    /// Different TeX: the glyph outlines morph.
    Morph,
}

/// Rule 3: a pair with the same TeX up to whitespace moves; one with different TeX morphs.
pub fn pair_kind(a: &str, b: &str) -> PairKind {
    if norm(a) == norm(b) {
        PairKind::Move
    } else {
        PairKind::Morph
    }
}

/// A part's box in the formula image: top-left and size, pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl PartBox {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> PartBox {
        PartBox { x, y, w, h }
    }

    pub fn center(&self) -> [f64; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }
}

fn clamp(p: f64) -> f64 {
    p.clamp(0.0, 1.0)
}

/// Rule 3: the box of a matched part at progress `p`: centre and size interpolated linearly.
pub fn box_at(a: PartBox, b: PartBox, p: f64) -> PartBox {
    let p = clamp(p);
    let (ca, cb) = (a.center(), b.center());
    let (w, h) = (a.w + (b.w - a.w) * p, a.h + (b.h - a.h) * p);
    let (cx, cy) = (ca[0] + (cb[0] - ca[0]) * p, ca[1] + (cb[1] - ca[1]) * p);
    PartBox::new(cx - w / 2.0, cy - h / 2.0, w, h)
}

/// Rule 3: `[dx, dy, s]`, the translation of the source centre and the uniform scale about it (by height) that carry a
/// moving part's glyphs to the box of [`box_at`]; an error when either box has no area.
pub fn part_transform(a: PartBox, b: PartBox, p: f64) -> Result<[f64; 3], String> {
    if !(a.w > 0.0 && a.h > 0.0 && b.w > 0.0 && b.h > 0.0) {
        return Err(format!("part boxes have positive size: {a:?} {b:?}"));
    }
    let p = clamp(p);
    let (ca, cb) = (a.center(), b.center());
    Ok([(cb[0] - ca[0]) * p, (cb[1] - ca[1]) * p, 1.0 + (b.h / a.h - 1.0) * p])
}

/// Rule 4: an unmatched source part keeps its place and fades: opacity 1 − p.
pub fn opacity_out(p: f64) -> f64 {
    1.0 - clamp(p)
}

/// Rule 4: an unmatched target part appears in its place: opacity p.
pub fn opacity_in(p: f64) -> f64 {
    clamp(p)
}
