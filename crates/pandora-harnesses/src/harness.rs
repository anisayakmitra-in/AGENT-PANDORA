use crate::data::DataGene;
use crate::design::DesignGene;
use crate::genes::CodingGene;
use crate::manifest::{
    coding_manifest, coordination_meta_manifest, core_source_manifest, data_manifest,
    debugging_manifest, design_manifest, operations_manifest, research_manifest, security_manifest,
};
use crate::operations::OperationsGene;
use crate::research::ResearchGene;
use crate::security::SecurityGene;
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

pub struct OperationsHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl OperationsHarness {
    pub fn new() -> Self {
        Self {
            manifest: operations_manifest().expect("built-in Operations Harness manifest is valid"),
            genes: OperationsGene::all(),
        }
    }
}

impl Default for OperationsHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for OperationsHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

pub struct SecurityHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

pub struct DebuggingHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

pub struct DataHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

impl DataHarness {
    pub fn new() -> Self {
        Self {
            manifest: data_manifest().expect("built-in Data Harness manifest is valid"),
            genes: DataGene::all(),
        }
    }
}

impl Default for DataHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for DataHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

impl DebuggingHarness {
    pub fn new() -> Self {
        Self {
            manifest: debugging_manifest().expect("built-in Debugging Harness manifest is valid"),
            genes: crate::debugging::DebuggingGene::all(),
        }
    }
}

impl Default for DebuggingHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for DebuggingHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

impl SecurityHarness {
    pub fn new() -> Self {
        Self {
            manifest: security_manifest().expect("built-in Security Harness manifest is valid"),
            genes: SecurityGene::all(),
        }
    }
}

impl Default for SecurityHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness for SecurityHarness {
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
