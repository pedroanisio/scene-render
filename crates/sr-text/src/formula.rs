//! Formulas: a TeX math subset typeset with an OpenType MATH font —
//! letters (math italic), digits, Greek, binary operators and relations
//! with TeX spacing, grouping, `\frac`, `\sqrt[n]{}`, sub- and superscripts,
//! big operators (`\sum`, `\prod`, `\int`) with limits in display style,
//! `\left`/`\right` delimiters stretched to their content, `\text`,
//! `\mathrm`, function names, matrices, accents, infix fractions and spaces.
//! `\part{name}{body}` (the semantic-tex draft) lays out as the group `{body}` and tags the glyphs and rules the
//! body produces with the part, for [`typeset_parts`].

use rustybuzz::ttf_parser::GlyphId;
use sr_vector::geom::{Rect, Xf};
use sr_vector::scene::{FillRule, Paint};
use sr_vector::{shapes, Path};

use crate::font::FontLib;
use crate::glyph::Drawing;
use crate::parts::{Part, PartBox};

#[derive(Debug, Clone, PartialEq)]
enum N {
    Row(Vec<N>),
    Ord(char),
    Bin(char),
    Rel(char),
    Open(char),
    Close(char),
    Punct(char),
    Big(char, bool),
    Fn(String, bool),
    Frac(Box<N>, Box<N>),
    Matrix(Vec<Vec<N>>, String),
    Accent(String, Box<N>),
    Sqrt(Option<Box<N>>, Box<N>),
    Scripts(Box<N>, Option<Box<N>>, Option<Box<N>>),
    Delim(char, Box<N>, char),
    Text(String),
    Space(f64),
    /// A tagged part, numbered in order of its opening tag.
    Part(u32, Box<N>),
}

fn symbol(name: &str) -> Option<N> {
    let greek = [
        ("alpha", 'α'),
        ("beta", 'β'),
        ("gamma", 'γ'),
        ("delta", 'δ'),
        ("epsilon", 'ϵ'),
        ("varepsilon", 'ε'),
        ("zeta", 'ζ'),
        ("eta", 'η'),
        ("theta", 'θ'),
        ("iota", 'ι'),
        ("kappa", 'κ'),
        ("lambda", 'λ'),
        ("mu", 'μ'),
        ("nu", 'ν'),
        ("xi", 'ξ'),
        ("pi", 'π'),
        ("rho", 'ρ'),
        ("sigma", 'σ'),
        ("tau", 'τ'),
        ("upsilon", 'υ'),
        ("phi", 'ϕ'),
        ("varphi", 'φ'),
        ("chi", 'χ'),
        ("psi", 'ψ'),
        ("omega", 'ω'),
        ("Gamma", 'Γ'),
        ("Delta", 'Δ'),
        ("Theta", 'Θ'),
        ("Lambda", 'Λ'),
        ("Xi", 'Ξ'),
        ("Pi", 'Π'),
        ("Sigma", 'Σ'),
        ("Upsilon", 'Υ'),
        ("Phi", 'Φ'),
        ("Psi", 'Ψ'),
        ("Omega", 'Ω'),
    ];
    if let Some(&(_, c)) = greek.iter().find(|(n, _)| *n == name) {
        return Some(N::Ord(c));
    }
    Some(match name {
        "cdot" => N::Bin('⋅'),
        "times" => N::Bin('×'),
        "pm" => N::Bin('±'),
        "mp" => N::Bin('∓'),
        "div" => N::Bin('÷'),
        "cup" => N::Bin('∪'),
        "cap" => N::Bin('∩'),
        "leq" | "le" => N::Rel('≤'),
        "geq" | "ge" => N::Rel('≥'),
        "neq" | "ne" => N::Rel('≠'),
        "approx" => N::Rel('≈'),
        "equiv" => N::Rel('≡'),
        "sim" => N::Rel('∼'),
        "to" | "rightarrow" => N::Rel('→'),
        "leftarrow" => N::Rel('←'),
        "Rightarrow" | "implies" => N::Rel('⇒'),
        "in" => N::Rel('∈'),
        "subset" => N::Rel('⊂'),
        "infty" => N::Ord('∞'),
        "partial" => N::Ord('∂'),
        "nabla" => N::Ord('∇'),
        "prime" => N::Ord('′'),
        "ldots" | "dots" => N::Ord('…'),
        "cdots" => N::Ord('⋯'),
        "sum" => N::Big('∑', true),
        "prod" => N::Big('∏', true),
        "int" => N::Big('∫', false),
        "oint" => N::Big('∮', false),
        "," => N::Space(3.0 / 18.0),
        ":" | ">" => N::Space(4.0 / 18.0),
        ";" => N::Space(5.0 / 18.0),
        "!" => N::Space(-3.0 / 18.0),
        "quad" => N::Space(1.0),
        "qquad" => N::Space(2.0),
        " " => N::Space(0.33),
        "{" | "lbrace" => N::Open('{'),
        "}" | "rbrace" => N::Close('}'),
        "langle" => N::Open('⟨'),
        "rangle" => N::Close('⟩'),
        "sin" | "cos" | "tan" | "log" | "ln" | "exp" | "sec" | "csc" | "cot" | "arcsin" | "arccos" | "arctan"
        | "sinh" | "cosh" | "tanh" | "det" => N::Fn(name.to_string(), false),
        "lim" | "max" | "min" | "sup" | "inf" => N::Fn(name.to_string(), true),
        _ => return None,
    })
}

/// Nested atoms and rows a formula may have (a braced group is two).
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    depth: usize,
    /// The `\part` tags opened so far.
    parts: u32,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }
    fn command(&mut self) -> String {
        let start = self.i;
        if self.peek().is_some_and(|c| !c.is_ascii_alphabetic()) {
            self.i += 1;
        } else {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
        }
        self.s[start..self.i].iter().collect()
    }
    fn group(&mut self) -> Result<N, String> {
        self.skip_ws();
        if self.peek() == Some('{') {
            self.i += 1;
            let r = self.row(Some('}'))?;
            self.i += 1;
            Ok(r)
        } else {
            self.atom()?.ok_or_else(|| "expected an argument".into())
        }
    }
    fn raw_group(&mut self) -> Result<String, String> {
        self.skip_ws();
        if self.peek() != Some('{') {
            return Err("expected {".into());
        }
        self.i += 1;
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            self.i += 1;
        }
        if self.peek().is_none() {
            return Err("unclosed group".into());
        }
        let t: String = self.s[start..self.i].iter().collect();
        self.i += 1;
        Ok(t)
    }
    fn at_command(&self, name: &str) -> bool {
        let chars: Vec<char> = std::iter::once('\\').chain(name.chars()).collect();
        self.s[self.i..].starts_with(&chars) && !self.s.get(self.i + chars.len()).is_some_and(char::is_ascii_alphabetic)
    }
    fn matrix(&mut self) -> Result<N, String> {
        let env = self.raw_group()?;
        let (left, right) = match env.as_str() {
            "matrix" | "smallmatrix" | "aligned" => (' ', ' '),
            "pmatrix" => ('(', ')'),
            "bmatrix" => ('[', ']'),
            "Bmatrix" => ('{', '}'),
            "vmatrix" => ('|', '|'),
            "Vmatrix" => ('‖', '‖'),
            "cases" => ('{', ' '),
            _ => return Err(format!("unknown environment {env}")),
        };
        let mut rows = Vec::new();
        let mut row = Vec::new();
        loop {
            row.push(self.row(None)?);
            if self.peek() == Some('&') {
                self.i += 1;
                continue;
            }
            if self.s[self.i..].starts_with(&['\\', '\\']) {
                self.i += 2;
                rows.push(std::mem::take(&mut row));
                self.skip_ws();
                if !self.at_command("end") {
                    continue;
                }
            }
            if !self.at_command("end") {
                return Err(format!("unclosed {env} environment"));
            }
            self.i += 4;
            let end = self.raw_group()?;
            if end != env {
                return Err(format!("expected \\end{{{env}}}, got \\end{{{end}}}"));
            }
            if !row.is_empty() {
                rows.push(row);
            }
            break;
        }
        let matrix = N::Matrix(rows, env);
        Ok(if left == ' ' && right == ' ' { matrix } else { N::Delim(left, Box::new(matrix), right) })
    }
    fn delim(&mut self) -> Result<char, String> {
        self.skip_ws();
        match self.peek() {
            Some('\\') => {
                self.i += 1;
                let c = self.command();
                Ok(match c.as_str() {
                    "{" | "lbrace" => '{',
                    "}" | "rbrace" => '}',
                    "langle" => '⟨',
                    "rangle" => '⟩',
                    "|" => '‖',
                    _ => ' ',
                })
            }
            Some(c) => {
                self.i += 1;
                Ok(if c == '.' { ' ' } else { c })
            }
            None => Err("missing delimiter".into()),
        }
    }
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("nested deeper than {} groups", MAX_DEPTH / 2));
        }
        Ok(())
    }
    fn atom(&mut self) -> Result<Option<N>, String> {
        self.enter()?;
        let r = self.atom_in();
        self.depth -= 1;
        r
    }
    fn atom_in(&mut self) -> Result<Option<N>, String> {
        self.skip_ws();
        let Some(c) = self.peek() else { return Ok(None) };
        self.i += 1;
        Ok(Some(match c {
            '{' => {
                let r = self.row(Some('}'))?;
                self.i += 1;
                r
            }
            '\\' => {
                let name = self.command();
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => N::Frac(Box::new(self.group()?), Box::new(self.group()?)),
                    "begin" => self.matrix()?,
                    "hat" | "widehat" | "bar" | "overline" | "underline" | "vec" | "overrightarrow" | "tilde"
                    | "widetilde" | "dot" | "ddot" | "acute" | "grave" | "breve" | "check" => {
                        N::Accent(name, Box::new(self.group()?))
                    }
                    "sqrt" => {
                        self.skip_ws();
                        let idx = if self.peek() == Some('[') {
                            self.i += 1;
                            let r = self.row(Some(']'))?;
                            self.i += 1;
                            Some(Box::new(r))
                        } else {
                            None
                        };
                        N::Sqrt(idx, Box::new(self.group()?))
                    }
                    "left" => {
                        let l = self.delim()?;
                        let body = self.row(None)?;
                        let r = if self.s[self.i..].starts_with(&['\\', 'r', 'i', 'g', 'h', 't']) {
                            self.i += 6;
                            self.delim()?
                        } else {
                            return Err("\\left without \\right".into());
                        };
                        N::Delim(l, Box::new(body), r)
                    }
                    "text" | "mathrm" | "operatorname" | "mathbf" => N::Text(self.raw_group()?),
                    "part" => {
                        // the name: checked with the whole formula by parts::parts before parsing
                        self.raw_group()?;
                        self.skip_ws();
                        if self.peek() != Some('{') {
                            return Err("\\part has no body group".into());
                        }
                        let index = self.parts;
                        self.parts += 1;
                        N::Part(index, Box::new(self.group()?))
                    }
                    other => symbol(other).ok_or_else(|| format!("unknown command \\{other}"))?,
                }
            }
            '+' | '*' => N::Bin(if c == '*' { '∗' } else { c }),
            '-' => N::Bin('−'),
            '=' | '<' | '>' => N::Rel(c),
            '(' | '[' => N::Open(c),
            ')' | ']' => N::Close(c),
            '|' => N::Ord('|'),
            ',' | ';' => N::Punct(c),
            '\'' => N::Ord('′'),
            '^' | '_' => return Err(format!("{c} needs a base")),
            '}' => return Err("unbalanced }".into()),
            c => N::Ord(c),
        }))
    }
    fn row(&mut self, until: Option<char>) -> Result<N, String> {
        self.enter()?;
        let r = self.row_in(until, true);
        self.depth -= 1;
        r
    }
    fn row_in(&mut self, until: Option<char>, allow_over: bool) -> Result<N, String> {
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if until.is_none()
                && (self.peek() == Some('&') || self.at_command("end") || self.s[self.i..].starts_with(&['\\', '\\']))
            {
                break;
            }
            if self.at_command("over") {
                if !allow_over {
                    return Err("multiple \\over commands in one group".into());
                }
                self.i += 5;
                let denominator = self.row_in(until, false)?;
                return Ok(N::Frac(Box::new(N::Row(items)), Box::new(denominator)));
            }
            match self.peek() {
                None => {
                    if until.is_some() {
                        return Err("unclosed group".into());
                    }
                    break;
                }
                Some(c) if Some(c) == until => break,
                Some('\\') if until.is_none() && self.at_command("right") => break,
                Some('^') | Some('_') => {
                    let base = items.pop().unwrap_or(N::Row(vec![]));
                    let (mut sub, mut sup) = match base {
                        N::Scripts(b, s, p) => {
                            items.push(*b);
                            (s, p)
                        }
                        other => {
                            items.push(other);
                            (None, None)
                        }
                    };
                    let b = items.pop().unwrap();
                    while let Some(op) = self.peek().filter(|c| *c == '^' || *c == '_') {
                        self.i += 1;
                        let g = Box::new(self.group()?);
                        if op == '^' {
                            sup = Some(g);
                        } else {
                            sub = Some(g);
                        }
                        self.skip_ws();
                    }
                    items.push(N::Scripts(Box::new(b), sub, sup));
                }
                _ => match self.atom()? {
                    Some(a) => items.push(a),
                    None => break,
                },
            }
        }
        Ok(N::Row(items))
    }
}

/// Glyph or rule of a laid-out formula (y down, baseline 0).
#[derive(Debug, Clone)]
enum Item {
    Glyph { gid: u16, x: f64, y: f64, k: f64, sy: f64, part: Option<u32> },
    Rule { x: f64, y: f64, w: f64, h: f64, part: Option<u32> },
}

impl Item {
    fn part_mut(&mut self) -> &mut Option<u32> {
        match self {
            Item::Glyph { part, .. } | Item::Rule { part, .. } => part,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct B {
    w: f64,
    asc: f64,
    desc: f64,
    items: Vec<Item>,
}

impl B {
    fn put(&mut self, o: B, x: f64, y: f64) {
        for it in o.items {
            self.items.push(match it {
                Item::Glyph { gid, x: gx, y: gy, k, sy, part } => {
                    Item::Glyph { gid, x: gx + x, y: gy + y, k, sy, part }
                }
                Item::Rule { x: rx, y: ry, w, h, part } => Item::Rule { x: rx + x, y: ry + y, w, h, part },
            });
        }
        self.asc = self.asc.max(o.asc - y);
        self.desc = self.desc.max(o.desc + y);
    }
}

struct Ctx<'f> {
    face: &'f rustybuzz::Face<'f>,
    upem: f64,
    c: Option<rustybuzz::ttf_parser::math::Constants<'f>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
}

impl Ctx<'_> {
    fn k(&self, size: f64) -> f64 {
        size / self.upem
    }
    fn cst(&self, f: impl Fn(&rustybuzz::ttf_parser::math::Constants) -> i16, fallback: f64, size: f64) -> f64 {
        self.c.as_ref().map(|c| f(c) as f64).unwrap_or(fallback * self.upem) * self.k(size)
    }
    fn script_scale(&self, level: u8) -> f64 {
        let (s1, s2) = self
            .c
            .as_ref()
            .map(|c| (c.script_percent_scale_down() as f64, c.script_script_percent_scale_down() as f64))
            .unwrap_or((70.0, 50.0));
        match level {
            0 => 1.0,
            1 => s1.max(1.0) / 100.0,
            _ => s2.max(1.0) / 100.0,
        }
    }
    fn gid(&self, ch: char) -> u16 {
        self.face.glyph_index(ch).map(|g| g.0).unwrap_or(0)
    }
    fn glyph(&self, ch: char, size: f64, italic: bool) -> B {
        let styled = if italic { math_italic(ch) } else { ch };
        let gid = if styled != ch && self.face.glyph_index(styled).is_some() { self.gid(styled) } else { self.gid(ch) };
        let k = self.k(size);
        let adv = self.face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f64 * k;
        let bb = self.face.glyph_bounding_box(GlyphId(gid));
        let (asc, desc) = bb.map(|b| (b.y_max as f64 * k, -(b.y_min as f64) * k)).unwrap_or((size * 0.7, 0.0));
        B {
            w: adv,
            asc: asc.max(0.0),
            desc: desc.max(0.0),
            items: vec![Item::Glyph { gid, x: 0.0, y: 0.0, k, sy: 1.0, part: None }],
        }
    }
    /// A delimiter or radical stretched to cover `target` height, centred on the axis unless `bottom` aligns it.
    fn stretched(&self, ch: char, size: f64, target: f64, axis: f64) -> B {
        let mut g = self.glyph(ch, size, false);
        let h = (g.asc + g.desc).max(1e-6);
        let sy = (target / h).max(1.0);
        if let Some(Item::Glyph { sy: s, y, .. }) = g.items.first_mut() {
            *s = sy;
            // keep the stretched glyph centred on the axis
            let mid = (g.asc - g.desc) * 0.5 * sy;
            *y = -(axis - mid);
        }
        let (asc, desc) = (g.asc * sy, g.desc * sy);
        let shift = axis - (asc - desc) * 0.5;
        g.asc = asc + shift;
        g.desc = desc - shift;
        g
    }
    fn class(n: &N) -> Class {
        match n {
            N::Bin(_) => Class::Bin,
            N::Rel(_) => Class::Rel,
            N::Open(_) => Class::Open,
            N::Close(_) => Class::Close,
            N::Punct(_) => Class::Punct,
            N::Big(..) | N::Fn(..) => Class::Op,
            N::Scripts(b, _, _) => Self::class(b),
            _ => Class::Ord,
        }
    }
    fn layout(&self, n: &N, size: f64, level: u8, display: bool) -> B {
        let sz = size * self.script_scale(level);
        let axis = self.cst(|c| c.axis_height().value, 0.25, sz);
        match n {
            N::Ord(c) => self.glyph(*c, sz, c.is_alphabetic() && c.is_ascii() || ('α'..='ω').contains(c)),
            N::Bin(c) | N::Rel(c) | N::Open(c) | N::Close(c) | N::Punct(c) => self.glyph(*c, sz, false),
            N::Text(t) | N::Fn(t, _) => {
                let mut b = B::default();
                for ch in t.chars() {
                    let g = self.glyph(ch, sz, false);
                    let w = g.w;
                    b.put(g, b.w, 0.0);
                    b.w += w;
                }
                b
            }
            N::Space(em) => B { w: em * sz, ..Default::default() },
            N::Part(index, body) => {
                // a part lays out as its group; what an inner part already holds stays the inner part's
                let mut b = self.layout(body, size, level, display);
                for it in &mut b.items {
                    it.part_mut().get_or_insert(*index);
                }
                b
            }
            N::Big(c, _) => {
                let g = self.glyph(*c, sz, false);
                if display {
                    // display operators are larger, centred on the axis
                    let h = (g.asc + g.desc).max(1e-6);
                    let min_h = self
                        .c
                        .as_ref()
                        .map(|c| c.display_operator_min_height() as f64 * self.k(sz))
                        .unwrap_or(sz * 1.5);
                    let s = (min_h / h).max(1.4);
                    let mut o = self.stretched(*c, sz, h * s, axis);
                    if let Some(Item::Glyph { k, .. }) = o.items.first_mut() {
                        *k *= 1.0;
                    }
                    o
                } else {
                    g
                }
            }
            N::Row(items) => {
                let mut b = B::default();
                let mut prev: Option<Class> = None;
                for it in items {
                    let cl = Self::class(it);
                    let mu = sz / 18.0;
                    let gap = match (prev, cl) {
                        (None, _) => 0.0,
                        (_, Class::Rel) | (Some(Class::Rel), _) if level == 0 => 5.0 * mu,
                        (Some(Class::Bin), _) | (_, Class::Bin) if level == 0 => 4.0 * mu,
                        (Some(Class::Op), Class::Ord) | (Some(Class::Ord), Class::Op) => 3.0 * mu,
                        (Some(Class::Punct), _) => 3.0 * mu,
                        _ => 0.0,
                    };
                    b.w += gap;
                    let o = self.layout(it, size, level, display);
                    let w = o.w;
                    b.put(o, b.w, 0.0);
                    b.w += w;
                    prev = Some(cl);
                }
                b
            }
            N::Frac(num, den) => {
                let lv = if display { level } else { level + 1 };
                let nb = self.layout(num, size, lv, false);
                let db = self.layout(den, size, lv, false);
                let t = self.cst(|c| c.fraction_rule_thickness().value, 0.05, sz);
                let (up, down) = if display {
                    (
                        self.cst(|c| c.fraction_numerator_display_style_shift_up().value, 0.68, sz),
                        self.cst(|c| c.fraction_denominator_display_style_shift_down().value, 0.68, sz),
                    )
                } else {
                    (
                        self.cst(|c| c.fraction_numerator_shift_up().value, 0.39, sz),
                        self.cst(|c| c.fraction_denominator_shift_down().value, 0.34, sz),
                    )
                };
                let gap = self.cst(|c| c.fraction_numerator_gap_min().value, 0.04, sz);
                let up = up.max(axis + t * 0.5 + gap + nb.desc);
                let down = down.max(-axis + t * 0.5 + gap + db.asc);
                let pad = sz * 0.12;
                let w = nb.w.max(db.w) + 2.0 * pad;
                let mut b = B { w, ..Default::default() };
                let (nw, dw) = (nb.w, db.w);
                b.put(nb, (w - nw) * 0.5, -up);
                b.put(db, (w - dw) * 0.5, down);
                b.items.push(Item::Rule { x: 0.0, y: -axis - t * 0.5, w, h: t, part: None });
                b.asc = b.asc.max(axis + t);
                b
            }
            N::Matrix(rows, environment) => {
                let level = level + u8::from(environment == "smallmatrix");
                let sz = size * self.script_scale(level);
                let cells: Vec<Vec<B>> =
                    rows.iter().map(|row| row.iter().map(|n| self.layout(n, size, level, false)).collect()).collect();
                let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
                let widths: Vec<f64> = (0..cols)
                    .map(|col| cells.iter().filter_map(|row| row.get(col)).map(|b| b.w).fold(0.0, f64::max))
                    .collect();
                let heights: Vec<(f64, f64)> = cells
                    .iter()
                    .map(|row| {
                        (
                            row.iter().map(|b| b.asc).fold(sz * 0.7, f64::max),
                            row.iter().map(|b| b.desc).fold(sz * 0.2, f64::max),
                        )
                    })
                    .collect();
                let gap = sz * 0.3;
                let total: f64 =
                    heights.iter().map(|(a, d)| a + d).sum::<f64>() + gap * rows.len().saturating_sub(1) as f64;
                let mut y = -total / 2.0 - axis;
                let mut out = B {
                    w: widths.iter().sum::<f64>() + sz * 0.7 * cols.saturating_sub(1) as f64,
                    ..Default::default()
                };
                for (row, (asc, desc)) in cells.into_iter().zip(heights) {
                    let mut x = 0.0;
                    for (column, (cell, width)) in row.into_iter().zip(&widths).enumerate() {
                        let shift = if environment == "cases" {
                            0.0
                        } else if environment == "aligned" {
                            if column % 2 == 0 {
                                width - cell.w
                            } else {
                                0.0
                            }
                        } else {
                            (width - cell.w) / 2.0
                        };
                        out.put(cell, x + shift, y + asc);
                        x += width + sz * 0.7;
                    }
                    y += asc + desc + gap;
                }
                out
            }
            N::Accent(name, body) => {
                let bb = self.layout(body, size, level, display);
                let (w, asc, desc) = (bb.w, bb.asc, bb.desc);
                let mut out = B { w, ..Default::default() };
                out.put(bb, 0.0, 0.0);
                let thickness = self.cst(|c| c.overbar_rule_thickness().value, 0.05, sz);
                let gap = sz * 0.08;
                if matches!(name.as_str(), "bar" | "overline" | "underline") {
                    let y = if name == "underline" { desc + gap } else { -asc - gap - thickness };
                    out.items.push(Item::Rule { x: 0.0, y, w, h: thickness, part: None });
                    out.asc = out.asc.max(-y);
                    out.desc = out.desc.max(y + thickness);
                } else {
                    let ch = match name.as_str() {
                        "hat" | "widehat" => '^',
                        "tilde" | "widetilde" => '~',
                        "vec" | "overrightarrow" => '→',
                        "dot" => '˙',
                        "ddot" => '¨',
                        "acute" => '´',
                        "grave" => '`',
                        "breve" => '˘',
                        "check" => 'ˇ',
                        _ => unreachable!(),
                    };
                    let mut accent = self.glyph(ch, sz, false);
                    if let Some(bounds) = self.face.glyph_bounding_box(GlyphId(self.gid(ch))) {
                        accent.desc = -(bounds.y_min as f64) * self.k(sz);
                    }
                    if matches!(name.as_str(), "widehat" | "widetilde" | "vec" | "overrightarrow") && accent.w > 0.0 {
                        let ratio = (w / accent.w).max(1.0);
                        for item in &mut accent.items {
                            if let Item::Glyph { k, sy, .. } = item {
                                *k *= ratio;
                                *sy /= ratio;
                            }
                        }
                        accent.w *= ratio;
                    }
                    let x = (w - accent.w) / 2.0;
                    let y = -asc - gap - accent.desc;
                    out.put(accent, x, y);
                }
                out
            }
            N::Sqrt(idx, body) => {
                let bb = self.layout(body, size, level, display);
                let t = self.cst(|c| c.radical_rule_thickness().value, 0.05, sz);
                let gap = self.cst(
                    |c| {
                        if display {
                            c.radical_display_style_vertical_gap().value
                        } else {
                            c.radical_vertical_gap().value
                        }
                    },
                    0.1,
                    sz,
                );
                let target = bb.asc + bb.desc + gap + t;
                let mut rad = self.glyph('√', sz, false);
                let rh = (rad.asc + rad.desc).max(1e-6);
                let sy = (target / rh).max(1.0);
                if let Some(Item::Glyph { sy: s, y, .. }) = rad.items.first_mut() {
                    *s = sy;
                    // radical top meets the rule above the body
                    *y = -(bb.asc + gap + t) + rad.asc * sy;
                }
                let rw = rad.w;
                let mut b = B::default();
                let mut x = 0.0;
                if let Some(ix) = idx {
                    let ib = self.layout(ix, size, level + 2, false);
                    let iw = ib.w;
                    b.put(ib, 0.0, -(bb.asc * 0.6));
                    x = (iw - rw * 0.4).max(0.0);
                }
                let asc_rad = bb.asc + gap + t;
                b.put(B { w: rw, asc: asc_rad, desc: bb.desc, items: rad.items }, x, 0.0);
                let bw = bb.w;
                b.put(bb, x + rw, 0.0);
                b.items.push(Item::Rule { x: x + rw, y: -(asc_rad), w: bw + sz * 0.05, h: t, part: None });
                b.w = x + rw + bw + sz * 0.05;
                b.asc = b.asc.max(asc_rad + t);
                b
            }
            N::Scripts(base, sub, sup) => {
                let limits = display && matches!(**base, N::Big(_, true) | N::Fn(_, true));
                let bb = self.layout(base, size, level, display);
                let lv = level + 1;
                let sb = sub.as_ref().map(|s| self.layout(s, size, lv, false));
                let pb = sup.as_ref().map(|s| self.layout(s, size, lv, false));
                let mut b = B::default();
                if limits {
                    let w = bb.w.max(sb.as_ref().map_or(0.0, |x| x.w)).max(pb.as_ref().map_or(0.0, |x| x.w));
                    let gap_u = self.cst(|c| c.upper_limit_gap_min().value, 0.1, sz);
                    let gap_l = self.cst(|c| c.lower_limit_gap_min().value, 0.1, sz);
                    let (ba, bd, bw) = (bb.asc, bb.desc, bb.w);
                    b.put(bb, (w - bw) * 0.5, 0.0);
                    if let Some(p) = pb {
                        let (pw, pd) = (p.w, p.desc);
                        b.put(p, (w - pw) * 0.5, -(ba + gap_u + pd));
                    }
                    if let Some(s) = sb {
                        let (sw, sa) = (s.w, s.asc);
                        b.put(s, (w - sw) * 0.5, bd + gap_l + sa);
                    }
                    b.w = w;
                    return b;
                }
                let (ba, bd, bw) = (bb.asc, bb.desc, bb.w);
                b.put(bb, 0.0, 0.0);
                let up = self
                    .cst(|c| c.superscript_shift_up().value, 0.36, sz)
                    .max(ba - self.cst(|c| c.superscript_baseline_drop_max().value, 0.25, sz));
                let down = self
                    .cst(|c| c.subscript_shift_down().value, 0.2, sz)
                    .max(bd + self.cst(|c| c.subscript_baseline_drop_min().value, 0.05, sz));
                let mut w = bw;
                let space = self.cst(|c| c.space_after_script().value, 0.05, sz);
                if let Some(p) = pb {
                    let pw = p.w;
                    b.put(p, bw, -up);
                    w = w.max(bw + pw);
                }
                if let Some(s) = sb {
                    let sw = s.w;
                    b.put(s, bw, down);
                    w = w.max(bw + sw);
                }
                b.w = w + space;
                b
            }
            N::Delim(l, body, r) => {
                let bb = self.layout(body, size, level, display);
                let target = 2.0 * (bb.asc - axis).max(bb.desc + axis) * 1.05;
                let mut b = B::default();
                if *l != ' ' {
                    let lb = self.stretched(*l, sz, target, axis);
                    let w = lb.w;
                    b.put(lb, 0.0, 0.0);
                    b.w = w;
                }
                let bw = bb.w;
                b.put(bb, b.w, 0.0);
                b.w += bw;
                if *r != ' ' {
                    let rb = self.stretched(*r, sz, target, axis);
                    let w = rb.w;
                    b.put(rb, b.w, 0.0);
                    b.w += w;
                }
                b
            }
        }
    }
}

fn math_italic(c: char) -> char {
    let off = |base: u32, from: char| char::from_u32(base + (c as u32 - from as u32));
    match c {
        'h' => 'ℎ',
        'a'..='z' => off(0x1D44E, 'a').unwrap_or(c),
        'A'..='Z' => off(0x1D434, 'A').unwrap_or(c),
        'α'..='ω' => off(0x1D6FC, 'α').unwrap_or(c),
        _ => c,
    }
}

/// The layout of `tex` at `size` px, with the math face it is set in.
fn typeset(lib: &mut FontLib, tex: &str, size: f64) -> Result<(usize, B), String> {
    if tex.contains("\\part") {
        crate::parts::parts(tex)?;
    }
    let chars: Vec<char> = tex.chars().collect();
    let mut parser = Parser { s: &chars, i: 0, depth: 0, parts: 0 };
    let ast = parser.row(None)?;
    if parser.i != chars.len() {
        return Err("unexpected alignment or closing delimiter".into());
    }
    let face = lib.math_face().ok_or("no font with an OpenType MATH table is installed")?;
    let b = lib
        .with_face(face, &[], |f| {
            let ctx = Ctx { face: f, upem: f.units_per_em() as f64, c: f.tables().math.and_then(|m| m.constants) };
            ctx.layout(&ast, size, 0, true)
        })
        .ok_or("math font does not parse")?;
    Ok((face, b))
}

/// The width and height of `tex` typeset at `size` px: the box [`draw`] draws it in at its natural size.
pub fn measure(lib: &mut FontLib, tex: &str, size: f64) -> Result<[f64; 2], String> {
    let (_, b) = typeset(lib, tex, size)?;
    Ok([b.w, b.asc + b.desc])
}

/// Typesets `tex` at `size` px, centred in a `size_box` box and scaled down to fit.
pub fn draw(
    lib: &mut FontLib,
    tex: &str,
    size: f64,
    size_box: [f64; 2],
    color: Paint,
    tol: f64,
) -> Result<Drawing, String> {
    let (face, b) = typeset(lib, tex, size)?;
    let place = placement(&b, size_box, fit_of(&b, size_box));
    let mut path = Path::default();
    for it in &b.items {
        path.extend(&item_path(lib, face, it, &place));
    }
    let mut d = Drawing::default();
    d.scene.fill(&path, FillRule::NonZero, color, 1.0, tol);
    Ok(d)
}

/// The scale that fits the layout in a `size_box` box, at most 1.
fn fit_of(b: &B, size_box: [f64; 2]) -> f64 {
    (size_box[0] / b.w.max(1e-9)).min(size_box[1] / (b.asc + b.desc).max(1e-9)).min(1.0)
}

/// The layout centred in a `size_box` box and scaled by `fit`.
fn placement(b: &B, size_box: [f64; 2], fit: f64) -> Xf {
    let (w, h) = (b.w, b.asc + b.desc);
    Xf::translate(size_box[0] * 0.5, size_box[1] * 0.5)
        .mul(&Xf::scale(fit, fit))
        // the layout's box spans [−asc, desc] about its baseline (y down): its middle is (desc − asc)/2
        .mul(&Xf::translate(-w * 0.5, b.asc - h * 0.5))
}

fn item_path(lib: &mut FontLib, face: usize, it: &Item, place: &Xf) -> Path {
    match *it {
        Item::Glyph { gid, x, y, k, sy, .. } => {
            lib.outline(face, gid, &[]).transform(&place.mul(&Xf([k, 0.0, 0.0, -k * sy, x, y])))
        }
        Item::Rule { x, y, w, h, .. } => shapes::rect(x, y, w, h, [0.0; 4]).transform(place),
    }
}

/// A glyph or rule of a formula, in the formula image's pixels after the fit, with the innermost part that produced
/// it (an index into [`Parted::parts`]) or `None`.
#[derive(Debug, Clone)]
pub struct Piece {
    pub part: Option<usize>,
    pub path: Path,
}

/// A formula typeset by part (the semantic-tex draft, Semantics 1): its parts in order of their opening tag, the
/// part that contains each, every glyph and rule as a [`Piece`] in layout order, and each part's box.
#[derive(Debug, Clone)]
pub struct Parted {
    pub parts: Vec<Part>,
    /// The innermost part that contains each part.
    pub parent: Vec<Option<usize>>,
    pub pieces: Vec<Piece>,
    /// The union of the flattened outlines of the part's pieces and its nested parts'; `None` when it produces
    /// nothing.
    pub boxes: Vec<Option<PartBox>>,
}

impl Parted {
    /// Whether piece `piece` belongs to part `part` or to a part nested in it.
    pub fn in_part(&self, piece: usize, part: usize) -> bool {
        let mut at = self.pieces[piece].part;
        while let Some(p) = at {
            if p == part {
                return true;
            }
            at = self.parent[p];
        }
        false
    }
}

/// Typesets `tex` at `size` px, centred in a `size_box` box and scaled down to fit as [`draw`] places it, and splits
/// it by `\part` (the semantic-tex draft, Semantics 1); `tol` is the flatness of the outlines the boxes are measured
/// on.
pub fn typeset_parts(lib: &mut FontLib, tex: &str, size: f64, size_box: [f64; 2], tol: f64) -> Result<Parted, String> {
    typeset_parts_at(lib, tex, size, size_box, None, tol)
}

/// [`typeset_parts`] centred in `size_box` and scaled by `fit` in place of the formula's own fit (a morph's two
/// formulas share the fit of their union box).
pub fn typeset_parts_fit(
    lib: &mut FontLib,
    tex: &str,
    size: f64,
    size_box: [f64; 2],
    fit: f64,
    tol: f64,
) -> Result<Parted, String> {
    typeset_parts_at(lib, tex, size, size_box, Some(fit), tol)
}

fn typeset_parts_at(
    lib: &mut FontLib,
    tex: &str,
    size: f64,
    size_box: [f64; 2],
    fit: Option<f64>,
    tol: f64,
) -> Result<Parted, String> {
    let parts = crate::parts::parts(tex)?;
    let (face, b) = typeset(lib, tex, size)?;
    let place = placement(&b, size_box, fit.unwrap_or_else(|| fit_of(&b, size_box)));
    let pieces: Vec<Piece> = b
        .items
        .iter()
        .map(|it| {
            let part = match it {
                Item::Glyph { part, .. } | Item::Rule { part, .. } => part.map(|p| p as usize),
            };
            Piece { part, path: item_path(lib, face, it, &place) }
        })
        .collect();
    // the container of a part is the last part opened before it one level out
    let parent = parts
        .iter()
        .map(|p| (p.depth > 0).then(|| parts[..p.index].iter().rposition(|q| q.depth + 1 == p.depth)).flatten())
        .collect();
    let mut parted = Parted { parts, parent, pieces, boxes: Vec::new() };
    let bounds: Vec<Rect> = parted
        .pieces
        .iter()
        .map(|pc| {
            let mut r = Rect::EMPTY;
            for poly in pc.path.flatten(tol) {
                for q in poly.pts {
                    r.add(q);
                }
            }
            r
        })
        .collect();
    parted.boxes = (0..parted.parts.len())
        .map(|i| {
            let r = (0..parted.pieces.len())
                .filter(|&k| parted.in_part(k, i) && !bounds[k].is_empty())
                .fold(Rect::EMPTY, |r, k| r.union(&bounds[k]));
            (!r.is_empty()).then(|| {
                let [x0, y0, x1, y1] = r.0;
                PartBox::new(x0, y0, x1 - x0, y1 - y0)
            })
        })
        .collect();
    Ok(parted)
}
