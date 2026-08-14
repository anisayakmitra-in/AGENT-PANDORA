use crate::genes::CodingGene;
use crate::manifest::coding_manifest;
use pandora_types::{Gene, Harness, HarnessManifest};

pub struct CodingHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl CodingHarness {
    pub fn new() -> Self {
        Self {
            manifest: coding_manifest().expect("built-in Coding Harness manifest is valid"),
            genes: CodingGene::all(),
        }
    }
}

impl Default for CodingHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for CodingHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}
