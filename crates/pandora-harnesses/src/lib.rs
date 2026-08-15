#![forbid(unsafe_code)]

pub mod genes;
pub mod harness;
pub mod manifest;

pub use genes::{CodingAction, CodingGene, CodingGeneRole, CodingRequest, PlanningContext};
pub use harness::{CodingHarness, CoreSourceHarness};

pub fn builtin_harnesses() -> Vec<Box<dyn pandora_types::Harness>> {
    vec![
        Box::new(CoreSourceHarness::new()),
        Box::new(CodingHarness::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::HarnessKind;

    #[test]
    fn builtin_catalog_includes_bound_core_source_harness() {
        let core = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "core-source")
            .expect("the built-in catalog should include the core source harness");

        assert_eq!(core.manifest().kind(), HarnessKind::Source);
        assert_eq!(
            core.manifest().constitutional_service(),
            Some("pandora-runtime")
        );
        assert!(core.genes().is_empty());
    }
}
