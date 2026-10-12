//! Test support: the byte layout of a WGSL struct, computed from its source with the WGSL rules for host-shareable
//! types, compared with the `#[repr(C)]` Rust struct that fills the buffer it is bound to.
//!
//! The two are written by hand. A field added, removed, reordered or retyped on one side only changes what the shader
//! reads without an error whenever the total size stays the same; this makes every such change fail in `cargo test`,
//! on a machine with no GPU.

use std::any::type_name;

/// One field of a Rust struct: its name, byte offset, size and type.
pub struct RustField {
    pub name: &'static str,
    pub offset: usize,
    pub size: usize,
    pub ty: &'static str,
}

/// The type name and size of the field a projection selects.
pub fn describe<T, F>(_: fn(&T) -> &F) -> (&'static str, usize) {
    (type_name::<F>(), std::mem::size_of::<F>())
}

/// `rust_fields!(Type; a, b, c)`: the `RustField` list of `Type`.
macro_rules! rust_fields {
    ($t:ty; $($f:ident),+ $(,)?) => {
        vec![$({
            let (ty, size) = $crate::wgsl_layout::describe::<$t, _>(|s| &s.$f);
            $crate::wgsl_layout::RustField { name: stringify!($f), offset: std::mem::offset_of!($t, $f), size, ty }
        }),+]
    };
}
pub(crate) use rust_fields;

/// A member of a WGSL struct with its computed place.
#[derive(Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    /// `f32`, `u32` or `i32`: the scalar every component of the member has.
    pub scalar: &'static str,
}

#[derive(Debug, PartialEq)]
pub struct Layout {
    pub size: usize,
    pub align: usize,
    pub members: Vec<Member>,
}

fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

fn strip_comments(src: &str) -> String {
    src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

/// The member declarations of `struct name { ... }`, as `(member, type)` text.
fn declarations(src: &str, name: &str) -> Vec<(String, String)> {
    let src = strip_comments(src);
    let head = format!("struct {name} {{");
    let start = src.find(&head).unwrap_or_else(|| panic!("no `struct {name}` in the WGSL source")) + head.len();
    let end = start + src[start..].find('}').expect("an unclosed struct");
    // a comma inside <...> separates template arguments, not members
    let (mut depth, mut cur, mut out) = (0i32, String::new(), Vec::new());
    for c in src[start..end].chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .map(|d| {
            assert!(!d.contains('@'), "{name}: `{d}` has an attribute; only host-shareable structs are checked");
            let (m, t) = d.split_once(':').unwrap_or_else(|| panic!("{name}: `{d}` is not `member: type`"));
            (m.trim().to_string(), t.trim().to_string())
        })
        .collect()
}

/// `(align, size, scalar)` of a WGSL type.
fn type_layout(src: &str, ty: &str) -> (usize, usize, &'static str) {
    let ty = ty.trim();
    let scalar = |s: &str| match s {
        "f32" => Some("f32"),
        "u32" => Some("u32"),
        "i32" => Some("i32"),
        _ => None,
    };
    if let Some(s) = scalar(ty) {
        return (4, 4, s);
    }
    let args = |prefix: &str| ty.strip_prefix(prefix).and_then(|r| r.strip_suffix('>')).map(|r| r.to_string());
    if let Some(a) = args("vec2<") {
        return (8, 8, scalar(a.trim()).expect("vec2 of a scalar"));
    }
    if let Some(a) = args("vec3<") {
        return (16, 12, scalar(a.trim()).expect("vec3 of a scalar"));
    }
    if let Some(a) = args("vec4<") {
        return (16, 16, scalar(a.trim()).expect("vec4 of a scalar"));
    }
    for (name, cols, rows) in [("mat2x2<", 2, 2), ("mat3x3<", 3, 3), ("mat4x4<", 4, 4)] {
        if let Some(a) = args(name) {
            assert_eq!(a.trim(), "f32", "{ty}");
            let col_align = if rows == 2 { 8 } else { 16 };
            let col_size = if rows == 2 {
                8
            } else if rows == 3 {
                12
            } else {
                16
            };
            return (col_align, cols * round_up(col_size, col_align), "f32");
        }
    }
    if let Some(a) = args("array<") {
        let (elem, count) =
            a.rsplit_once(',').unwrap_or_else(|| panic!("{ty}: a runtime-sized array has no layout here"));
        let (align, size, s) = type_layout(src, elem);
        let n: usize = count.trim().parse().unwrap_or_else(|_| panic!("{ty}: the count must be a literal"));
        return (align, n * round_up(size, align), s);
    }
    // a struct of the same source
    let inner = layout(src, ty);
    let scalar = inner.members.first().map_or("f32", |m| m.scalar);
    (inner.align, inner.size, scalar)
}

/// The layout of `struct name` in `src`.
pub fn layout(src: &str, name: &str) -> Layout {
    let (mut offset, mut align, mut members) = (0usize, 1usize, Vec::new());
    for (member, ty) in declarations(src, name) {
        let (a, size, scalar) = type_layout(src, &ty);
        offset = round_up(offset, a);
        members.push(Member { name: member, offset, size, scalar });
        offset += size;
        align = align.max(a);
    }
    Layout { size: round_up(offset, align), align, members }
}

fn scalar_of(rust_type: &str) -> Option<&'static str> {
    ["f32", "u32", "i32"].into_iter().find(|s| rust_type.replace(['[', ']', ';', ' '], ",").split(',').any(|p| p == *s))
}

fn is_padding(name: &str) -> bool {
    name.starts_with("pad")
}

/// Every difference between the WGSL struct and the Rust struct; empty when they agree.
///
/// Members are matched by name. A member named `pad…` on either side is padding and is only constrained by the total
/// size. Matched members must start at the same offset, have the same size and hold the same scalar kind.
pub fn differences(src: &str, wgsl: &str, rust_size: usize, fields: &[RustField]) -> Vec<String> {
    let l = layout(src, wgsl);
    let mut out = Vec::new();
    if l.size != rust_size {
        out.push(format!("size: WGSL {} is {} bytes, the Rust struct {rust_size}", wgsl, l.size));
    }
    for m in l.members.iter().filter(|m| !is_padding(&m.name)) {
        match fields.iter().find(|f| f.name == m.name) {
            None => out.push(format!("{}: in WGSL {wgsl} and not in the Rust struct", m.name)),
            Some(f) => {
                if f.offset != m.offset {
                    out.push(format!("{}: WGSL offset {}, Rust offset {}", m.name, m.offset, f.offset));
                }
                if f.size != m.size {
                    out.push(format!("{}: WGSL size {}, Rust size {}", m.name, m.size, f.size));
                }
                if scalar_of(f.ty) != Some(m.scalar) {
                    out.push(format!("{}: WGSL scalar {}, Rust type {}", m.name, m.scalar, f.ty));
                }
            }
        }
    }
    for f in fields.iter().filter(|f| !is_padding(f.name)) {
        if !l.members.iter().any(|m| m.name == f.name) {
            out.push(format!("{}: in the Rust struct and not in WGSL {wgsl}", f.name));
        }
    }
    out
}

/// Asserts the two agree.
#[track_caller]
pub fn assert_agree(src: &str, wgsl: &str, rust_size: usize, fields: &[RustField]) {
    let d = differences(src, wgsl, rust_size, fields);
    assert!(d.is_empty(), "the Rust struct and WGSL {wgsl} disagree:\n  {}", d.join("\n  "));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "
        struct Inner { a: vec3<f32>, b: u32 }            // 16 bytes, align 16
        struct S {
          x: u32,
          v: vec3<f32>,      // aligned to 16
          w: f32,            // packs after the vec3
          m: mat3x3<f32>,
          arr: array<vec4<u32>, 2>,
          i: Inner,
          h: vec2<i32>,
        }";

    #[test]
    fn the_wgsl_rules_are_applied() {
        let l = layout(SRC, "S");
        let at: Vec<(&str, usize, usize)> = l.members.iter().map(|m| (m.name.as_str(), m.offset, m.size)).collect();
        assert_eq!(
            at,
            [("x", 0, 4), ("v", 16, 12), ("w", 28, 4), ("m", 32, 48), ("arr", 80, 32), ("i", 112, 16), ("h", 128, 8)]
        );
        assert_eq!((l.align, l.size), (16, 144));
        assert_eq!(layout(SRC, "Inner").size, 16);
    }

    #[test]
    fn a_comma_inside_a_template_does_not_split_a_member() {
        assert_eq!(layout("struct T { a: array<vec4<f32>, 3>, b: f32 }", "T").size, 64);
    }

    #[repr(C)]
    struct Good {
        x: u32,
        pad: [u32; 3],
        v: [f32; 3],
        w: f32,
    }

    fn good_fields() -> Vec<RustField> {
        rust_fields!(Good; x, pad, v, w)
    }

    const GOOD: &str = "struct G { x: u32, pad0: u32, pad1: u32, pad2: u32, v: vec3<f32>, w: f32 }";

    #[test]
    fn matching_structs_agree_and_padding_names_may_differ() {
        assert_agree(GOOD, "G", std::mem::size_of::<Good>(), &good_fields());
    }

    #[test]
    fn a_reordered_field_of_equal_size_is_found() {
        let src = "struct G { v: vec3<f32>, w: f32, x: u32, pad0: u32, pad1: u32, pad2: u32 }";
        let d = differences(src, "G", std::mem::size_of::<Good>(), &good_fields());
        assert!(d.iter().any(|m| m.starts_with("x: WGSL offset 16, Rust offset 0")), "{d:?}");
        assert!(d.iter().any(|m| m.starts_with("v: WGSL offset 0")), "{d:?}");
    }

    #[test]
    fn a_retyped_field_of_equal_size_is_found() {
        let src = GOOD.replace("w: f32", "w: u32");
        let d = differences(&src, "G", std::mem::size_of::<Good>(), &good_fields());
        assert_eq!(d, ["w: WGSL scalar u32, Rust type f32"]);
    }

    #[test]
    fn a_vec3_that_became_a_vec4_changes_the_size_and_offsets() {
        let src = GOOD.replace("v: vec3<f32>, w: f32", "v: vec4<f32>, w: f32");
        let d = differences(&src, "G", std::mem::size_of::<Good>(), &good_fields());
        assert!(d.iter().any(|m| m.starts_with("size:")), "{d:?}");
        assert!(d.iter().any(|m| m.starts_with("v: WGSL size 16")), "{d:?}");
    }

    #[test]
    fn a_member_on_one_side_only_is_found() {
        let d = differences(
            &GOOD.replace("w: f32", "w: f32, extra: f32"),
            "G",
            std::mem::size_of::<Good>(),
            &good_fields(),
        );
        assert!(d.iter().any(|m| m.starts_with("extra: in WGSL")), "{d:?}");
        let d = differences(&GOOD.replace(", w: f32", ""), "G", std::mem::size_of::<Good>(), &good_fields());
        assert!(d.iter().any(|m| m.starts_with("w: in the Rust struct")), "{d:?}");
    }
}
