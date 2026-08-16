use crate::gene::Gene;
use crate::ids::{GeneId, HarnessId, IdError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HarnessKind {
    Source,
    Meta,
    Domain,
}

impl HarnessKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Meta => "meta",
            Self::Domain => "domain",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestError {
    InvalidId(IdError),
    EmptyField(&'static str),
    MissingConstitutionalService,
    UnexpectedConstitutionalService,
    MissingMetaComposition,
    UnexpectedMetaComposition,
    EmptyMetaDomainHarnesses,
    DuplicateMetaDomainHarness,
    InvalidMetaHandoffLimit,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::MissingConstitutionalService => {
                formatter.write_str("source harness requires one constitutional service")
            }
            Self::UnexpectedConstitutionalService => {
                formatter.write_str("only source harnesses may bind a constitutional service")
            }
            Self::MissingMetaComposition => {
                formatter.write_str("meta harness requires a composition declaration")
            }
            Self::UnexpectedMetaComposition => {
                formatter.write_str("only meta harnesses may declare a composition")
            }
            Self::EmptyMetaDomainHarnesses => {
                formatter.write_str("meta composition requires at least one domain harness")
            }
            Self::DuplicateMetaDomainHarness => {
                formatter.write_str("meta composition domain harnesses must be unique")
            }
            Self::InvalidMetaHandoffLimit => {
                formatter.write_str("meta composition handoff limit must be greater than zero")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<IdError> for ManifestError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetaComposition {
    allowed_domains: Vec<HarnessId>,
    max_handoffs: u32,
}

impl MetaComposition {
    pub fn new(allowed_domains: Vec<HarnessId>, max_handoffs: u32) -> Result<Self, ManifestError> {
        let composition = Self {
            allowed_domains,
            max_handoffs,
        };
        composition.validate()?;
        Ok(composition)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.allowed_domains.is_empty() {
            return Err(ManifestError::EmptyMetaDomainHarnesses);
        }
        if self.max_handoffs == 0 {
            return Err(ManifestError::InvalidMetaHandoffLimit);
        }

        let mut unique = BTreeSet::new();
        if self.allowed_domains.iter().any(|id| !unique.insert(id)) {
            return Err(ManifestError::DuplicateMetaDomainHarness);
        }

        Ok(())
    }

    pub fn allowed_domains(&self) -> &[HarnessId] {
        &self.allowed_domains
    }

    pub fn allows_domain(&self, harness_id: &HarnessId) -> bool {
        self.allowed_domains.iter().any(|id| id == harness_id)
    }

    pub const fn max_handoffs(&self) -> u32 {
        self.max_handoffs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessManifest {
    id: HarnessId,
    version: String,
    name: String,
    kind: HarnessKind,
    constitutional_service: Option<String>,
    owned_genes: Vec<GeneId>,
    meta_composition: Option<MetaComposition>,
}

impl HarnessManifest {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        name: impl Into<String>,
        kind: HarnessKind,
        constitutional_service: Option<String>,
        owned_genes: Vec<GeneId>,
    ) -> Result<Self, ManifestError> {
        Self::build(
            id,
            version,
            name,
            kind,
            constitutional_service,
            owned_genes,
            None,
        )
    }

    pub fn new_meta(
        id: impl Into<String>,
        version: impl Into<String>,
        name: impl Into<String>,
        composition: MetaComposition,
    ) -> Result<Self, ManifestError> {
        Self::build(
            id,
            version,
            name,
            HarnessKind::Meta,
            None,
            Vec::new(),
            Some(composition),
        )
    }

    fn build(
        id: impl Into<String>,
        version: impl Into<String>,
        name: impl Into<String>,
        kind: HarnessKind,
        constitutional_service: Option<String>,
        owned_genes: Vec<GeneId>,
        meta_composition: Option<MetaComposition>,
    ) -> Result<Self, ManifestError> {
        let id = HarnessId::new(id)?;
        let version = version.into();
        let name = name.into();
        validate_text("version", &version)?;
        validate_text("name", &name)?;

        match (kind, constitutional_service.as_deref()) {
            (HarnessKind::Source, None | Some("")) => {
                return Err(ManifestError::MissingConstitutionalService);
            }
            (HarnessKind::Meta | HarnessKind::Domain, Some(_)) => {
                return Err(ManifestError::UnexpectedConstitutionalService);
            }
            _ => {}
        }
        if kind == HarnessKind::Meta && meta_composition.is_none() {
            return Err(ManifestError::MissingMetaComposition);
        }
        if kind != HarnessKind::Meta && meta_composition.is_some() {
            return Err(ManifestError::UnexpectedMetaComposition);
        }

        Ok(Self {
            id,
            version,
            name,
            kind,
            constitutional_service,
            owned_genes,
            meta_composition,
        })
    }

    pub fn id(&self) -> &HarnessId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> HarnessKind {
        self.kind
    }

    pub fn constitutional_service(&self) -> Option<&str> {
        self.constitutional_service.as_deref()
    }

    pub fn owned_genes(&self) -> &[GeneId] {
        &self.owned_genes
    }

    pub fn meta_composition(&self) -> Option<&MetaComposition> {
        self.meta_composition.as_ref()
    }
}

pub trait Harness: Send + Sync {
    fn manifest(&self) -> &HarnessManifest;
    fn genes(&self) -> &[Box<dyn Gene>];
}

fn validate_text(field: &'static str, value: &str) -> Result<(), ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::EmptyField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HarnessKind, HarnessManifest, ManifestError, MetaComposition};
    use crate::HarnessId;

    #[test]
    fn meta_harness_requires_an_explicit_composition() {
        let result = HarnessManifest::new(
            "coordination-meta",
            "0.1.0",
            "Coordination Meta",
            HarnessKind::Meta,
            None,
            Vec::new(),
        );

        assert_eq!(result, Err(ManifestError::MissingMetaComposition));
    }

    #[test]
    fn meta_composition_rejects_duplicate_domain_members() {
        let domain = HarnessId::new("coding-domain").unwrap();

        assert_eq!(
            MetaComposition::new(vec![domain.clone(), domain], 8),
            Err(ManifestError::DuplicateMetaDomainHarness)
        );
    }

    #[test]
    fn meta_manifest_exposes_only_declared_domain_members() {
        let domain = HarnessId::new("coding-domain").unwrap();
        let composition = MetaComposition::new(vec![domain.clone()], 8).unwrap();
        let manifest = HarnessManifest::new_meta(
            "coordination-meta",
            "0.1.0",
            "Coordination Meta",
            composition,
        )
        .unwrap();

        let meta = manifest.meta_composition().expect("meta composition");
        assert!(meta.allows_domain(&domain));
        assert!(!meta.allows_domain(&HarnessId::new("research-domain").unwrap()));
        assert_eq!(meta.max_handoffs(), 8);
    }
}
