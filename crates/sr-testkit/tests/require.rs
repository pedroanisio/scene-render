//! The environment decides: each requirement, when missing, skips a test with a `SKIPPED` line, or fails it
//! with `SR_REQUIRE_<NAME>=1`. Each case reruns this test binary on one `missing_*` test, which acts only
//! when `SR_TESTKIT_CHILD=1` marks the rerun.

use std::process::Command;

const CHILD: &str = "SR_TESTKIT_CHILD";
const NO_TOOL: &str = "/nonexistent/sr-testkit-tool";

fn child() -> bool {
    std::env::var(CHILD).as_deref() == Ok("1")
}

#[test]
fn missing_gpu() {
    if child() {
        let _: () = sr_testkit::require_gpu!(Err::<(), _>("no adapter"), "missing_gpu");
        panic!("require_gpu! went on without a GPU");
    }
}

#[test]
fn missing_ffmpeg() {
    if child() {
        sr_testkit::require_ffmpeg!("missing_ffmpeg", &[NO_TOOL]);
        panic!("require_ffmpeg! went on without FFmpeg");
    }
}

#[test]
fn missing_heic() {
    if child() {
        let _: () = sr_testkit::require_heic!(Err::<(), _>("no HEVC plugin"), "missing_heic");
        panic!("require_heic! went on without a HEIC decoder");
    }
}

/// The exit status and standard error of `test` rerun with `var` set to `value`, or unset.
fn rerun(test: &str, var: &str, value: Option<&str>) -> (bool, String) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", test, "--nocapture", "--test-threads=1"]).env(CHILD, "1");
    for other in ["SR_REQUIRE_GPU", "SR_REQUIRE_FFMPEG", "SR_REQUIRE_HEIC"] {
        command.env_remove(other);
    }
    if let Some(value) = value {
        command.env(var, value);
    }
    let out = command.output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const CASES: [(&str, &str, &str, &str); 3] = [
    ("missing_gpu", "SR_REQUIRE_GPU", "no adapter", "a GPU adapter"),
    ("missing_ffmpeg", "SR_REQUIRE_FFMPEG", "cannot run /nonexistent/sr-testkit-tool", "FFmpeg"),
    ("missing_heic", "SR_REQUIRE_HEIC", "no HEVC plugin", "a HEIC decoder"),
];

#[test]
fn a_missing_requirement_skips_the_test_with_a_reason() {
    for (test, var, reason, _) in CASES {
        for value in [None, Some("0"), Some("")] {
            let (passed, stderr) = rerun(test, var, value);
            assert!(passed, "{test} with {var}={value:?}: {stderr}");
            assert!(stderr.contains(&format!("SKIPPED ({test}): {reason}\n")), "{test}: {stderr}");
        }
    }
}

#[test]
fn a_missing_requirement_fails_the_test_where_it_is_required() {
    for (test, var, reason, needs) in CASES {
        let (passed, stderr) = rerun(test, var, Some("1"));
        assert!(!passed, "{test} with {var}=1 passed: {stderr}");
        assert!(stderr.contains(&format!("{test}: {reason} ({var}=1 requires {needs})")), "{test}: {stderr}");
        assert!(!stderr.contains("SKIPPED"), "{test}: {stderr}");
        assert!(!stderr.contains("went on"), "{test}: {stderr}");
    }
}
