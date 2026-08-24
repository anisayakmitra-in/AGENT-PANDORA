use crate::design::DesignGene;
use crate::genes::CodingGene;
use crate::manifest::{
    coding_manifest, coordination_meta_manifest, core_source_manifest, design_manifest,
    research_manifest,
};
use crate::research::ResearchGene;
use pandora_types::{Gene, Harness, HarnessManifest};

pub struct CoreSourceHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl CoreSourceHarness {
    pub fn new() -> Self {
        Self {
            manifest: core_source_manifest()
                .expect("built-in Core Source Harness manifest is valid"),
            genes: Vec::new(),
        }
    }
}

impl Default for CoreSourceHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for CoreSourceHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

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

pub struct ResearchHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl ResearchHarness {
    pub fn new() -> Self {
        Self {
            manifest: research_manifest().expect("built-in Research Harness manifest is valid"),
            genes: ResearchGene::all(),
        }
    }
}

impl Default for ResearchHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for ResearchHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

pub struct DesignHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl DesignHarness {
    pub fn new() -> Self {
        Self {
            manifest: design_manifest().expect("built-in Design Harness manifest is valid"),
            genes: DesignGene::all(),
        }
    }
}

impl Default for DesignHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for DesignHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

pub struct CoordinationMetaHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl CoordinationMetaHarness {
    pub fn new() -> Self {
        Self {
            manifest: coordination_meta_manifest()
                .expect("built-in Coordination Meta Harness manifest is valid"),
            genes: Vec::new(),
        }
    }
}

impl Default for CoordinationMetaHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for CoordinationMetaHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}
