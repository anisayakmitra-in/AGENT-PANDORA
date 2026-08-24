use crate::builtin_genes;
use pandora_types::{
    Gene, Harness, HarnessKind, HarnessManifest, PackageKind, PackageManifest, PackageManifestError,
};
use std::fmt;

pub struct DeclarativeDomainHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

pub struct DeclarativeMetaHarness {
    manifest: HarnessManifest,
    genes: Vec<Box<dyn Gene>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainProfileError {
    UnsupportedPackageKind(PackageKind),
    InvalidPackage(PackageManifestError),
    MissingGeneImplementation { id: String, version: String },
    DuplicateGeneImplementation { id: String, version: String },
    NoExecutableGenes,
    InvalidManifest(pandora_types::harness::ManifestError),
}

impl fmt::Display for DomainProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPackageKind(kind) => {
                write!(
                    formatter,
                    "package kind {} is not a Domain Harness",
                    kind.as_str()
                )
            }
            Self::InvalidPackage(error) => error.fmt(formatter),
            Self::MissingGeneImplementation { id, version } => write!(
                formatter,
                "no Gene implementation is available for {id}@{version}"
            ),
            Self::DuplicateGeneImplementation { id, version } => {
                write!(
                    formatter,
                    "multiple Gene implementations match {id}@{version}"
                )
            }
            Self::NoExecutableGenes => {
                formatter.write_str("Domain Harness profile has no executable Genes")
            }
            Self::InvalidManifest(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DomainProfileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetaProfileError {
    UnsupportedPackageKind(PackageKind),
    InvalidPackage(PackageManifestError),
    InvalidManifest(pandora_types::harness::ManifestError),
}

impl fmt::Display for MetaProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPackageKind(kind) => {
                write!(
                    formatter,
                    "package kind {} is not a Meta Harness",
                    kind.as_str()
                )
            }
            Self::InvalidPackage(error) => error.fmt(formatter),
            Self::InvalidManifest(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MetaProfileError {}

impl DeclarativeDomainHarness {
    pub fn from_package(package: &PackageManifest) -> Result<Self, DomainProfileError> {
        Self::from_package_with_genes(package, Vec::new())
    }

    pub fn from_package_with_genes(
        package: &PackageManifest,
        genes: Vec<Box<dyn Gene>>,
    ) -> Result<Self, DomainProfileError> {
        package
            .validate()
            .map_err(DomainProfileError::InvalidPackage)?;
        if package.kind() != PackageKind::DomainHarness {
            return Err(DomainProfileError::UnsupportedPackageKind(package.kind()));
        }

        let mut available = builtin_genes();
        available.extend(genes);
        let mut genes = Vec::new();
        for dependency in package.dependencies() {
            let matches = available
                .iter()
                .enumerate()
                .filter_map(|(index, gene)| {
                    (gene.manifest().id().as_str() == dependency.id().as_str()
                        && gene.manifest().version() == dependency.version())
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [index] => genes.push(available.remove(*index)),
                [_, _, ..] => {
                    return Err(DomainProfileError::DuplicateGeneImplementation {
                        id: dependency.id().as_str().to_owned(),
                        version: dependency.version().to_owned(),
                    });
                }
                [] if dependency.optional() => {}
                [] => {
                    return Err(DomainProfileError::MissingGeneImplementation {
                        id: dependency.id().as_str().to_owned(),
                        version: dependency.version().to_owned(),
                    });
                }
            }
        }
        if genes.is_empty() {
            return Err(DomainProfileError::NoExecutableGenes);
        }

        let owned_genes = genes
            .iter()
            .map(|gene| gene.manifest().id().clone())
            .collect();
        let manifest = HarnessManifest::new(
            package.id().as_str(),
            package.version(),
            package.id().as_str(),
            HarnessKind::Domain,
            None,
            owned_genes,
        )
        .map_err(DomainProfileError::InvalidManifest)?;
        Ok(Self { manifest, genes })
    }
}

impl Harness for DeclarativeDomainHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}

impl DeclarativeMetaHarness {
    pub fn from_package(package: &PackageManifest) -> Result<Self, MetaProfileError> {
        package
            .validate()
            .map_err(MetaProfileError::InvalidPackage)?;
        if package.kind() != PackageKind::MetaHarness {
            return Err(MetaProfileError::UnsupportedPackageKind(package.kind()));
        }
        let manifest = HarnessManifest::new_meta(
            package.id().as_str(),
            package.version(),
            package.id().as_str(),
            package
                .meta_composition()
                .expect("validated Meta Harness package has a composition")
                .clone(),
        )
        .map_err(MetaProfileError::InvalidManifest)?;
        Ok(Self {
            manifest,
            genes: Vec::new(),
        })
    }
}

impl Harness for DeclarativeMetaHarness {
    fn manifest(&self) -> &HarnessManifest {
        &self.manifest
    }

    fn genes(&self) -> &[Box<dyn Gene>] {
        &self.genes
    }
}
