use crate::harness::{
    CodingHarness, CoordinationMetaHarness, CoreSourceHarness, DataHarness, DebuggingHarness,
    DesignHarness, OperationsHarness, ResearchHarness, SecurityHarness,
};
use crate::profile::{
    DeclarativeDomainHarness, DeclarativeMetaHarness, DomainProfileError, MetaProfileError,
};
use pandora_types::{DomainRoutingProfile, Gene, Harness, HarnessId, HarnessKind, PackageManifest};
use std::collections::BTreeMap;
use std::fmt;

pub fn replaceable_builtin_harness_kind(id: &str) -> Option<HarnessKind> {
    match id {
        "coding-domain" | "research-domain" | "design-domain" | "operations-domain"
        | "security-domain" | "debugging-domain" | "data-domain" => Some(HarnessKind::Domain),
        "coordination-meta" => Some(HarnessKind::Meta),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessCatalogError {
    NotReplaceable { id: String },
    ReplacementTargetMissing { id: String },
    Domain(DomainProfileError),
    Meta(MetaProfileError),
}

impl fmt::Display for HarnessCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReplaceable { id } => {
                write!(
                    formatter,
                    "Harness {id} is not an optional built-in replacement target"
                )
            }
            Self::ReplacementTargetMissing { id } => {
                write!(formatter, "built-in replacement target {id} is unavailable")
            }
            Self::Domain(error) => error.fmt(formatter),
            Self::Meta(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HarnessCatalogError {}

pub struct HarnessCatalog {
    harnesses: Vec<Box<dyn Harness>>,
    domain_routing: BTreeMap<HarnessId, DomainRoutingProfile>,
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
                Box::new(DataHarness::new()),
                Box::new(CoordinationMetaHarness::new()),
            ],
            domain_routing: BTreeMap::new(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Harness> {
        self.harnesses.iter().map(Box::as_ref)
    }

    pub fn find(&self, id: &HarnessId) -> Option<&dyn Harness> {
        self.iter().find(|harness| harness.manifest().id() == id)
    }

    pub fn domain_routing(&self) -> impl Iterator<Item = (&HarnessId, &DomainRoutingProfile)> {
        self.domain_routing.iter()
    }

    pub fn with_declarative_domain(
        mut self,
        package: &PackageManifest,
    ) -> Result<Self, DomainProfileError> {
        self.harnesses
            .push(Box::new(DeclarativeDomainHarness::from_package(package)?));
        self.bind_domain_routing(package);
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
        self.bind_domain_routing(package);
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

    pub fn replace_declarative_domain_genes(
        mut self,
        package: &PackageManifest,
        genes: Vec<Box<dyn Gene>>,
    ) -> Result<Self, HarnessCatalogError> {
        let id = package.id().as_str();
        if replaceable_builtin_harness_kind(id) != Some(HarnessKind::Domain) {
            return Err(HarnessCatalogError::NotReplaceable { id: id.to_owned() });
        }
        let replacement = DeclarativeDomainHarness::from_package_with_genes(package, genes)
            .map_err(HarnessCatalogError::Domain)?;
        let index = self
            .harnesses
            .iter()
            .position(|harness| harness.manifest().id().as_str() == id)
            .ok_or_else(|| HarnessCatalogError::ReplacementTargetMissing { id: id.to_owned() })?;
        self.harnesses[index] = Box::new(replacement);
        self.bind_domain_routing(package);
        Ok(self)
    }

    pub fn replace_declarative_meta(
        mut self,
        package: &PackageManifest,
    ) -> Result<Self, HarnessCatalogError> {
        let id = package.id().as_str();
        if replaceable_builtin_harness_kind(id) != Some(HarnessKind::Meta) {
            return Err(HarnessCatalogError::NotReplaceable { id: id.to_owned() });
        }
        let replacement =
            DeclarativeMetaHarness::from_package(package).map_err(HarnessCatalogError::Meta)?;
        let index = self
            .harnesses
            .iter()
            .position(|harness| harness.manifest().id().as_str() == id)
            .ok_or_else(|| HarnessCatalogError::ReplacementTargetMissing { id: id.to_owned() })?;
        self.harnesses[index] = Box::new(replacement);
        Ok(self)
    }

    pub fn into_harnesses(self) -> Vec<Box<dyn Harness>> {
        self.harnesses
    }

    fn bind_domain_routing(&mut self, package: &PackageManifest) {
        self.domain_routing.remove(
            &HarnessId::new(package.id().as_str())
                .expect("validated package IDs are valid Harness IDs"),
        );
        if let Some(routing) = package.domain_routing() {
            self.domain_routing.insert(
                HarnessId::new(package.id().as_str())
                    .expect("validated package IDs are valid Harness IDs"),
                routing.clone(),
            );
        }
    }
}
