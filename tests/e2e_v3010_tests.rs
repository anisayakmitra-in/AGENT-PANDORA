#![forbid(unsafe_code)]

#[path = "../crates/pandora-cli/tests/e2e_v3010_tests.rs"]
mod e2e_v3010_tests;

pub use e2e_v3010_tests::*;
