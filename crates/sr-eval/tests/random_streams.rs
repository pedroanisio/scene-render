//! The numeric table of the random-streams SREP (sr-core `srep/srep-0000-random-streams.md`, Conformance), every
//! row against the functions the engine draws with: sections 1 to 5 on `sr_vector::d24`, Appendix A's legacy rows
//! on `sr_sim::rng` and `sr_eval::rng`. Integers compare exactly; binary64 results to 1e-12 relative, the Perlin
//! row to 1e-9, as the SREP states.

use sr_vector::d24::{
    crc32, d24_hash, d24_unit, element_seed, gaussian, noise, pack, perlin3, permutation, stream, stream_normal,
    stream_signed, unpack, weighted_octaves,
};
use sr_vector::modifiers::smooth_noise;

fn close(got: f64, want: f64, rel: f64) {
    assert!((got - want).abs() <= rel * want.abs().max(1.0), "{got} != {want}");
}

const NEG5: u64 = -5i64 as u64;
const H0: u64 = 14325487974692486532;

#[test]
fn hashes_and_unit_draws() {
    assert_eq!(d24_hash(1, 2, 3), 0xd62a7f6e291fef64);
    assert_eq!(d24_hash(42, 7, NEG5), 0x2bf4c61d835a19bb);
    assert_eq!(d24_hash(0, 0, 0), 0x238275bc38fcbe91);
    assert_eq!(d24_hash(H0, 0, 0), 0);
    close(d24_unit(0, 0, 0), 0.13870941014555427, 1e-12);
    close(d24_unit(7, 1, 0), 0.47670922732307197, 1e-12);
    close(d24_unit(7, 2, 5), 0.356758657594873, 1e-12);
}

#[test]
fn normal_draws_permutations_and_lattice_indices() {
    close(gaussian(1, 2, 3), 0.3180452816872121, 1e-12);
    close(gaussian(0, 0, 0), 0.34042346108851657, 1e-12);
    close(gaussian(H0, 0, 0), 6.763705635001895, 1e-12);
    assert_eq!(permutation(5, 0, 10), [9, 8, 3, 7, 2, 6, 5, 0, 1, 4]);
    assert_eq!(permutation(1, 7, 8), [4, 3, 5, 7, 2, 0, 6, 1]);
    assert_eq!(pack(-1, 2, 3), 0xc00005fffff);
    assert_eq!(pack(1048575, -1048576, 0), 0x200000fffff);
    assert_eq!(unpack(0xC00005FFFFF), (-1, 2, 3));
}

#[test]
fn element_seeds() {
    assert_eq!(crc32(b"fontSize"), 1178744372);
    assert_eq!(element_seed(1, "", Some(42), "order"), 3996963903);
    assert_eq!(element_seed(0, "p1", None, "stream"), 856627436);
    assert_eq!(element_seed(7, "fx", Some(0), "7.0"), 2350236182);
}

#[test]
fn gradient_value_octave_and_perlin_noise() {
    close(noise(0, 0, 0.5), -0.554802728864675, 1e-12);
    close(noise(42, 7, -1.3), -0.2219594810158777, 1e-12);
    assert_eq!(noise(0, 0, 2.0), 0.0);
    close(weighted_octaves(1, 0, 0.37, 4, 0.5), -0.11429312869138866, 1e-12);
    close(weighted_octaves(42, 3, -2.6, 3, 0.5), 0.13784387603697962, 1e-12);
    close(weighted_octaves(3, 1, 0.25, 2, 0.7), -0.08983855650259293, 1e-12);
    close(smooth_noise(5, 2, 0.5), -0.529593365289099, 1e-12);
    close(smooth_noise(5, 2, -1.25), -0.23344113072768796, 1e-12);
    let p = permutation(1, 7, 256);
    close(perlin3(&p, 0.3, 0.7, 0.0), 0.28006378512, 1e-9);
    assert_eq!(perlin3(&p, 1.0, 2.0, -5.0), 0.0);
}

#[test]
fn stream_draws_address_purpose_step_and_index() {
    let e = element_seed(0, "p1", None, "stream");
    close(stream(e, 11, 0, 4), 0.5170957861079158, 1e-12);
    close(stream(e, 11, 3, 4), 0.3488340232764445, 1e-12);
    close(stream_signed(e, 2, 0, 0), 0.28862525370006553, 1e-12);
    // S is U at channel p + t · 2³², S_normal is G there (section 5)
    assert_eq!(stream(e, 11, 3, 4), d24_unit(e, 11 + (3 << 32), 4));
    assert_eq!(stream_normal(e, 11, 3, 4), gaussian(e, 11 + (3 << 32), 4));
}

#[test]
fn legacy_simulation_draws_of_appendix_a() {
    assert_eq!(sr_sim::rng::hash(3, 4, 5), 0xfe4e081fb3b6bf64);
    close(sr_sim::rng::unit(7, 0, 11), 0.10948806701760105, 1e-12);
    assert_eq!(sr_eval::rng::hash_str(""), 0xf52a15e9a9b5e89b);
    assert_eq!(sr_eval::rng::hash_str("p1"), 0x5b050ef57e3b95bc);
    assert_eq!(sr_eval::rng::hash(&[1, 2, 3]), 0xd56d0264932a233f);
}

#[test]
fn unpack_inverts_pack_across_the_lattice_range() {
    for (i, j, k) in [(0, 0, 0), (-1048576, 1048575, -7), (1048575, -1048576, 1048575), (-1, -1, -1), (123, -456, 789)]
    {
        assert_eq!(unpack(pack(i, j, k)), (i, j, k));
    }
}
