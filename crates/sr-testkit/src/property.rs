//! Property tests that draw the same cases on every run.
//!
//! [`check`] draws a bounded number of inputs from a strategy with a ChaCha generator seeded from the property's
//! name, shrinks a failure at most [`MAX_SHRINK_ITERS`] steps and panics with the minimal failing input. It writes no
//! failure files and no `PROPTEST_*` variable changes what it draws, so a failure reproduces from the source alone.
//! Strategies and the `prop_*` macros come from the re-exported [`proptest`].

pub use proptest;

use proptest::strategy::Strategy;
use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestCaseResult, TestRunner};

/// Shrinking steps a failure may take before the smallest input found so far is reported.
pub const MAX_SHRINK_ITERS: u32 = 4096;

/// The seed of the property `name`: the 64-bit FNV-1a hash of its bytes.
pub fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The configuration [`check`] runs `name` with: `cases` inputs, the seed of `name`, no failure persistence, and every
/// setting a `PROPTEST_*` variable could change set here instead.
pub fn config(name: &str, cases: u32) -> Config {
    Config {
        cases,
        max_local_rejects: 65_536,
        max_global_rejects: 1024,
        max_flat_map_regens: 1_000_000,
        failure_persistence: None,
        max_shrink_time: 0,
        max_shrink_iters: MAX_SHRINK_ITERS,
        max_default_size_range: 100,
        verbose: 0,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(seed(name)),
        ..Config::default()
    }
}

/// Runs `test` on `cases` inputs drawn from `strategy` (see [`config`]).
///
/// # Panics
///
/// When `test` fails or panics for an input, with `property <name>: Test failed: <reason>.` and the minimal failing
/// input; or when the strategy rejects too many inputs.
#[track_caller]
pub fn check<S: Strategy>(name: &str, cases: u32, strategy: S, test: impl Fn(S::Value) -> TestCaseResult) {
    let mut runner = TestRunner::new(config(name, cases));
    if let Err(failure) = runner.run(&strategy, test) {
        panic!("property {name}: {failure}");
    }
}
