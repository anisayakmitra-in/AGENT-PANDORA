#![forbid(unsafe_code)]

pub mod genes;
pub mod harness;
pub mod manifest;

pub use genes::{CodingAction, CodingGene, CodingGeneRole, CodingRequest, PlanningContext};
pub use harness::CodingHarness;

pub fn builtin_harnesses() -> Vec<Box<dyn pandora_types::Harness>> {
    vec![Box::new(CodingHarness::new())]
}
