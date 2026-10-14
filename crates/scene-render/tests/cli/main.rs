//! The integration tests of the `cli` area, one module for each file they came in.

mod aac_ceiling;
mod cli;
#[path = "../common/mod.rs"]
mod common;
mod debug_gpu_flag;
mod eval_json_warnings;
mod loudness_report;
mod program_findings;
mod solver_failures;
