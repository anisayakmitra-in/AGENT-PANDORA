use crate::harness::{
    CodingHarness, CoordinationMetaHarness, CoreSourceHarness, DebuggingHarness, DesignHarness,
    OperationsHarness, ResearchHarness, SecurityHarness,
};
use crate::profile::{
    DeclarativeDomainHarness, DeclarativeMetaHarness, DomainProfileError, MetaProfileError,
};
use pandora_types::{Gene, Harness, HarnessId, PackageManifest};

pub struct HarnessCatalog {
    harnesses: Vec<Box<dyn Harness>>,
}

impl HarnessCatalog {
    pub fn builtins() -> Self {
        Self {
            harnesses: vec![
                Box::new(CoreSourceHarness::new()),
                Box::new(CodingHarness::new()),
                Box::new(ResearchHarness::new()),
                Box::new(DesignHarness::new()),
                Box::new(OperationsHarness::new()),
                Box::new(SecurityHarness::new()),
                Box::new(DebuggingHarness::new()),
                Box::new(CoordinationMetaHarness::new()),
            ],
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Harness> {
        self.harnesses.iter().map(Box::as_ref)
    }

    pub fn find(&self, id: &HarnessId) -> Option<&dyn Harness> {
        self.iter().find(|harness| harness.manifest().id() == id)
    }

    pub fn with_declarative_domain(
        mut self,
        package: &PackageManifest,
    ) -> Result<Self, DomainProfileError> {
        self.harnesses
            .push(Box::new(DeclarativeDomainHarness::from_package(package)?));
        Ok(self)
    }

    pub fn with_declarative_domain_genes(
        mut self,
        package: &PackageManifest,
        genes: Vec<Box<dyn Gene>>,
    ) -> Result<Self, DomainProfileError> {
        self.harnesses
            .push(Box::new(DeclarativeDomainHarness::from_package_with_genes(
                package, genes,
            )?));
        Ok(self)
    }

    pub fn with_declarative_meta(
        mut self,
        package: &PackageManifest,
    ) -> Result<Self, MetaProfileError> {
        self.harnesses
            .push(Box::new(DeclarativeMetaHarness::from_package(package)?));
        Ok(self)
    }

    pub fn into_harnesses(self) -> Vec<Box<dyn Harness>> {
        self.harnesses
    }
}
