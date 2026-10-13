//! A property draws the same cases on every run, from a seed its name fixes, and writes no failure files.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};

use sr_testkit::property::{self, proptest};

use proptest::prelude::*;
use proptest::test_runner::{RngAlgorithm, RngSeed};

fn draws(name: &str, cases: u32) -> Vec<u64> {
    let seen = RefCell::new(Vec::new());
    property::check(name, cases, any::<u64>(), |v| {
        seen.borrow_mut().push(v);
        Ok(())
    });
    seen.into_inner()
}

#[test]
fn a_property_draws_the_same_cases_on_every_run() {
    let first = draws("same", 16);
    assert_eq!(first.len(), 16);
    assert_eq!(first, draws("same", 16));
    assert_ne!(first, draws("other", 16));
}

#[test]
fn the_seed_follows_the_name_alone() {
    assert_eq!(property::seed("frame rates"), property::seed("frame rates"));
    assert_ne!(property::seed("frame rates"), property::seed("frame rate"));
    assert_eq!(property::seed(""), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn the_config_is_fixed_and_writes_no_failure_files() {
    let config = property::config("bounded", 32);
    assert_eq!(config.cases, 32);
    assert!(config.failure_persistence.is_none());
    assert_eq!(config.rng_algorithm, RngAlgorithm::ChaCha);
    assert_eq!(config.rng_seed, RngSeed::Fixed(property::seed("bounded")));
    assert_eq!(config.max_shrink_iters, property::MAX_SHRINK_ITERS);
    assert_eq!(config.max_shrink_time, 0);
    assert_eq!(config.verbose, 0);
}

#[test]
fn a_failing_property_panics_with_its_minimal_input() {
    let failure = panic::catch_unwind(AssertUnwindSafe(|| {
        property::check("below ten", 256, 0u32..1_000_000, |v| {
            prop_assert!(v < 10);
            Ok(())
        });
    }))
    .expect_err("a property that fails for most inputs passed");
    let message = failure.downcast_ref::<String>().expect("the failure message is a String");
    assert!(message.starts_with("property below ten: Test failed"), "{message}");
    assert!(message.ends_with("minimal failing input: 10"), "{message}");
}
