use pandora_types::{PackageKind, SkillManifest};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageAdmissionBoundary {
    HarnessRegistry,
    ConstitutionalSource,
    DataOnly,
    ProviderConfiguration,
    SkillEngine,
}

impl PackageAdmissionBoundary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HarnessRegistry => "harness_registry",
            Self::ConstitutionalSource => "constitutional_source",
            Self::DataOnly => "data_only",
            Self::ProviderConfiguration => "provider_configuration",
            Self::SkillEngine => "skill_engine",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageAdmissionRule {
    kind: PackageKind,
    boundary: PackageAdmissionBoundary,
    executable_artifact: bool,
}

impl PackageAdmissionRule {
    pub const fn kind(self) -> PackageKind {
        self.kind
    }

    pub const fn boundary(self) -> PackageAdmissionBoundary {
        self.boundary
    }

    pub const fn executable_artifact(self) -> bool {
        self.executable_artifact
    }

    pub const fn grants_runtime_authority(self) -> bool {
        false
    }

    pub const fn allows_harness_registry(self) -> bool {
        matches!(self.boundary, PackageAdmissionBoundary::HarnessRegistry)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageAdmissionError {
    InvalidResource,
    WrongBoundary {
        kind: PackageKind,
        required: PackageAdmissionBoundary,
    },
}

impl fmt::Display for PackageAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResource => formatter.write_str("skill resource is invalid"),
            Self::WrongBoundary { kind, required } => write!(
                formatter,
                "{} packages require the {} admission boundary",
                kind.as_str(),
                required.as_str()
            ),
        }
    }
}

impl std::error::Error for PackageAdmissionError {}

pub struct PackageAdmission;

impl PackageAdmission {
    pub const fn rule_for(kind: PackageKind) -> PackageAdmissionRule {
        let (boundary, executable_artifact) = match kind {
            PackageKind::Gene => (PackageAdmissionBoundary::HarnessRegistry, true),
            PackageKind::DomainHarness | PackageKind::MetaHarness => {
                (PackageAdmissionBoundary::HarnessRegistry, false)
            }
            PackageKind::SourceHarness => (PackageAdmissionBoundary::ConstitutionalSource, false),
            PackageKind::Package => (PackageAdmissionBoundary::DataOnly, false),
            PackageKind::Provider => (PackageAdmissionBoundary::ProviderConfiguration, false),
            PackageKind::Skill => (PackageAdmissionBoundary::SkillEngine, false),
        };
        PackageAdmissionRule {
            kind,
            boundary,
            executable_artifact,
        }
    }

    pub fn validate_harness_registry_kind(
        kind: PackageKind,
    ) -> Result<PackageAdmissionRule, PackageAdmissionError> {
        let rule = Self::rule_for(kind);
        if !rule.allows_harness_registry() {
            return Err(PackageAdmissionError::WrongBoundary {
                kind,
                required: rule.boundary(),
            });
        }
        Ok(rule)
    }

    pub fn validate_skill(manifest: &SkillManifest) -> Result<(), PackageAdmissionError> {
        if manifest.resources().iter().any(|resource| {
            resource.is_empty()
                || resource.chars().any(char::is_control)
                || resource == "."
                || resource == ".."
                || resource.contains(['/', '\\'])
        }) {
            return Err(PackageAdmissionError::InvalidResource);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_labels_cannot_be_paths() {
        let manifest = SkillManifest::new(
            "alpha",
            "0.1.0",
            "Alpha",
            "A skill",
            None,
            vec!["../secrets".to_owned()],
        )
        .unwrap();

        assert_eq!(
            PackageAdmission::validate_skill(&manifest),
            Err(PackageAdmissionError::InvalidResource)
        );
    }

    #[test]
    fn package_kinds_have_distinct_fail_closed_boundaries() {
        for (kind, boundary) in [
            (
                PackageKind::SourceHarness,
                PackageAdmissionBoundary::ConstitutionalSource,
            ),
            (PackageKind::Package, PackageAdmissionBoundary::DataOnly),
            (
                PackageKind::Provider,
                PackageAdmissionBoundary::ProviderConfiguration,
            ),
            (PackageKind::Skill, PackageAdmissionBoundary::SkillEngine),
        ] {
            let rule = PackageAdmission::rule_for(kind);
            assert_eq!(rule.kind(), kind);
            assert_eq!(rule.boundary(), boundary);
            assert!(!rule.executable_artifact());
            assert!(!rule.grants_runtime_authority());
            assert_eq!(
                PackageAdmission::validate_harness_registry_kind(kind),
                Err(PackageAdmissionError::WrongBoundary {
                    kind,
                    required: boundary,
                })
            );
        }
    }

    #[test]
    fn harness_registry_rules_preserve_wasm_and_profile_boundaries() {
        let gene = PackageAdmission::validate_harness_registry_kind(PackageKind::Gene).unwrap();
        assert!(gene.executable_artifact());
        assert!(!gene.grants_runtime_authority());

        for kind in [PackageKind::DomainHarness, PackageKind::MetaHarness] {
            let profile = PackageAdmission::validate_harness_registry_kind(kind).unwrap();
            assert!(!profile.executable_artifact());
            assert!(!profile.grants_runtime_authority());
        }
    }
}
