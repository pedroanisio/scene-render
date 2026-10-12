//! Plain-old-data structures shared with `shaders.wgsl`.

use bytemuck::{Pod, Zeroable};

/// Per-draw parameters (`Draw` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Draw {
    pub color: [f32; 4],
    pub uv_rect: [f32; 4],
    pub box_rect: [f32; 4],
    pub target_size: [f32; 2],
    pub opacity: f32,
    pub blend: u32,
    pub src_kind: u32,
    pub matte_mode: u32,
    pub mask_off: u32,
    pub mask_count: u32,
    pub paint: u32,
    pub lod: f32,
    pub seed: u32,
    pub flags: u32,
}

/// Source kinds.
pub mod src {
    pub const TEXTURE: u32 = 0;
    pub const SOLID: u32 = 1;
    pub const PAINT: u32 = 2;
    pub const TEXTURE_LOD: u32 = 3;
    pub const PATTERN: u32 = 4;
    /// A texture magnified with Catmull-Rom (`layer/@resample="bicubic"`); minified samples stay trilinear.
    pub const BICUBIC: u32 = 5;
    /// A texture magnified with Mitchell-Netravali, B = C = 1/3 (`layer/@resample="mitchell"`).
    pub const MITCHELL: u32 = 6;
}

/// Draw flags.
pub mod flag {
    /// Accessibility probe: replace RGB with `color.r`, preserving sampled alpha.
    pub const CONTRAST_INK: u32 = 16;
    pub const MATTE: u32 = 2;
    /// Antialias the edges of `box_rect`.
    pub const EDGE: u32 = 8;
    /// The texture lands texel for pixel on whole target pixels: read texels instead of sampling. `uv_rect` is then
    /// the target-pixel rectangle the content covers (x0, y0, x1, y1) and `box_rect.xy` the texel at its top left.
    pub const ALIGNED: u32 = 32;
}

/// A mask (`Mask` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Mask {
    pub rect: [f32; 4],
    pub kind: u32,
    pub mode: u32,
    pub invert: u32,
    pub fill_rule: u32,
    pub edge_off: u32,
    pub edge_count: u32,
    pub feather: f32,
    pub expansion: f32,
    pub radius: f32,
    pub opacity: f32,
    pub pad: [f32; 2],
}

/// A paint (`PaintDesc` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct PaintDesc {
    pub xform0: [f32; 4],
    pub xform1: [f32; 4],
    pub p0: [f32; 4],
    pub p1: [f32; 4],
    pub kind: u32,
    pub spread: u32,
    pub space: u32,
    pub stop_off: u32,
    pub stop_count: u32,
    pub dither: u32,
    pub pad: [u32; 2],
}

/// A gradient stop or mesh point colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Stop {
    pub color: [f32; 4],
    pub offset: f32,
    pub midpoint: f32,
    pub pad: [f32; 2],
}

/// Frame-wide constants.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Globals {
    pub linear_light: u32,
    pub seed: u32,
    pub pad: [u32; 2],
    pub to_srgb: [[f32; 4]; 3],
    pub from_srgb: [[f32; 4]; 3],
}

/// Generator parameters (`Gen` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Gen {
    pub size: [f32; 2],
    pub kind: u32,
    pub octaves: u32,
    pub scale: f32,
    pub evolution: f32,
    pub contrast: f32,
    pub angle: f32,
    /// The generator's seed as a u64 (low, high words: `seed`, `seed_hi`).
    pub seed: u32,
    pub paint_a: u32,
    pub paint_b: u32,
    pub seed_hi: u32,
    /// Film grain: this frame's seed (low, high); for a grid, the line width in pixels (f32 bits) and whether one was
    /// given (1) or the line follows the scale (0).
    pub grain: [u32; 4],
    /// Perlin permutation of 0..255 (seeded 64-bit hash draws), four per vector.
    pub perm: [[u32; 4]; 64],
}

impl Default for Gen {
    fn default() -> Gen {
        bytemuck::Zeroable::zeroed()
    }
}

/// One vertex of a draw quad.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Vertex {
    pub clip: [f32; 4],
    pub uv: [f32; 2],
    pub local: [f32; 2],
}

/// Converts a row-major 3×3 matrix to WGSL's column-major, vec4-padded layout.
pub fn mat3(m: &crate::color::M3) -> [[f32; 4]; 3] {
    [0, 1, 2].map(|c| [m[0][c] as f32, m[1][c] as f32, m[2][c] as f32, 0.0])
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn draw_matches_wgsl_draw() {
        let fields = rust_fields!(super::Draw; color, uv_rect, box_rect, target_size, opacity, blend, src_kind, matte_mode, mask_off, mask_count, paint, lod, seed, flags);
        assert_agree(include_str!("shaders.wgsl"), "Draw", std::mem::size_of::<super::Draw>(), &fields);
    }

    #[test]
    fn mask_matches_wgsl_mask() {
        let fields = rust_fields!(super::Mask; rect, kind, mode, invert, fill_rule, edge_off, edge_count, feather, expansion, radius, opacity, pad);
        assert_agree(include_str!("shaders.wgsl"), "Mask", std::mem::size_of::<super::Mask>(), &fields);
    }

    #[test]
    fn paint_desc_matches_wgsl_paint_desc() {
        let fields = rust_fields!(super::PaintDesc; xform0, xform1, p0, p1, kind, spread, space, stop_off, stop_count, dither, pad);
        assert_agree(include_str!("common.wgsl"), "PaintDesc", std::mem::size_of::<super::PaintDesc>(), &fields);
    }

    #[test]
    fn stop_matches_wgsl_stop() {
        let fields = rust_fields!(super::Stop; color, offset, midpoint, pad);
        assert_agree(include_str!("common.wgsl"), "Stop", std::mem::size_of::<super::Stop>(), &fields);
    }

    #[test]
    fn globals_matches_wgsl_globals() {
        let fields = rust_fields!(super::Globals; linear_light, seed, pad, to_srgb, from_srgb);
        assert_agree(include_str!("common.wgsl"), "Globals", std::mem::size_of::<super::Globals>(), &fields);
    }

    #[test]
    fn gen_matches_wgsl_gen() {
        let fields = rust_fields!(super::Gen; size, kind, octaves, scale, evolution, contrast, angle, seed, paint_a, paint_b, seed_hi, grain, perm);
        assert_agree(include_str!("shaders.wgsl"), "Gen", std::mem::size_of::<super::Gen>(), &fields);
    }
}
