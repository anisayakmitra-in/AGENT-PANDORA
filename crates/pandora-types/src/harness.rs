use crate::gene::Gene;
use crate::ids::{GeneId, HarnessId, IdError};
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
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<IdError> for ManifestError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
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

        Ok(Self {
            id,
            version,
            name,
            kind,
            constitutional_service,
            owned_genes,
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
}

pub struct SourceHarnessManifest {
    manifest: HarnessManifest,
}

impl TryFrom<HarnessManifest> for SourceHarnessManifest {
    type Error = ManifestError;

    fn try_from(manifest: HarnessManifest) -> Result<Self, Self::Error> {
        if manifest.kind != HarnessKind::Source {
            return Err(ManifestError::MissingConstitutionalService);
        }
        if manifest.constitutional_service.is_none() {
            return Err(ManifestError::MissingConstitutionalService);
        }
        Ok(Self { manifest })
    }
}

impl SourceHarnessManifest {
    pub fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    pub fn constitutional_service(&self) -> &str {
        self.manifest
            .constitutional_service()
            .expect("validated source harness has a service")
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
