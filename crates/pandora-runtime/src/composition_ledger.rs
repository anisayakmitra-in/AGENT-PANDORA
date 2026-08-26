use pandora_types::hash_artifact;
use serde::Serialize;
use std::fmt;

pub const COMPOSITION_LEDGER_VERSION: u16 = 1;
pub const MAX_COMPOSITION_BINDINGS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompositionSource {
    CleanSource,
    GeneratedSource,
    ThirdParty,
    Native,
    ArtifactFallback,
}

impl CompositionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CleanSource => "clean-source",
            Self::GeneratedSource => "generated-source",
            Self::ThirdParty => "third-party",
            Self::Native => "native",
            Self::ArtifactFallback => "artifact-fallback",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionLedgerError {
    Empty,
    TooManyBindings,
    EmptyComponent,
    EmptyVersion,
    InvalidField(&'static str),
    InvalidDigest,
    DuplicateBinding { component: String, version: String },
    MissingRequiredComponent(String),
}

impl fmt::Display for CompositionLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("composition ledger cannot be empty"),
            Self::TooManyBindings => {
                formatter.write_str("composition ledger has too many bindings")
            }
            Self::EmptyComponent => formatter.write_str("composition component cannot be empty"),
            Self::EmptyVersion => {
                formatter.write_str("composition component version cannot be empty")
            }
            Self::InvalidField(field) => write!(formatter, "invalid composition {field}"),
            Self::InvalidDigest => formatter.write_str("composition identity digest is invalid"),
            Self::DuplicateBinding { component, version } => {
                write!(
                    formatter,
                    "composition binding {component}@{version} is duplicated"
                )
            }
            Self::MissingRequiredComponent(component) => {
                write!(formatter, "composition ledger is missing {component}")
            }
        }
    }
}

impl std::error::Error for CompositionLedgerError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompositionBinding {
    component: String,
    version: String,
    source: CompositionSource,
    identity_digest: String,
}

impl CompositionBinding {
    pub fn new(
        component: impl Into<String>,
        version: impl Into<String>,
        source: CompositionSource,
        identity_digest: impl Into<String>,
    ) -> Result<Self, CompositionLedgerError> {
        let component = component.into();
        let version = version.into();
        let identity_digest = identity_digest.into();
        validate_identifier(&component, "component")?;
        validate_identifier(&version, "version")?;
        if !valid_digest(&identity_digest) {
            return Err(CompositionLedgerError::InvalidDigest);
        }
        Ok(Self {
            component,
            version,
            source,
            identity_digest,
        })
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub const fn source(&self) -> CompositionSource {
        self.source
    }

    pub fn identity_digest(&self) -> &str {
        &self.identity_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompositionLedger {
    version: u16,
    bindings: Vec<CompositionBinding>,
    #[serde(rename = "composition_digest")]
    digest: String,
}

impl CompositionLedger {
    pub fn new(mut bindings: Vec<CompositionBinding>) -> Result<Self, CompositionLedgerError> {
        if bindings.is_empty() {
            return Err(CompositionLedgerError::Empty);
        }
        if bindings.len() > MAX_COMPOSITION_BINDINGS {
            return Err(CompositionLedgerError::TooManyBindings);
        }
        bindings.sort_by(|left, right| {
            left.component
                .cmp(&right.component)
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.source.cmp(&right.source))
                .then_with(|| left.identity_digest.cmp(&right.identity_digest))
        });
        if let Some(pair) = bindings.windows(2).find(|pair| {
            pair[0].component == pair[1].component && pair[0].version == pair[1].version
        }) {
            return Err(CompositionLedgerError::DuplicateBinding {
                component: pair[0].component.clone(),
                version: pair[0].version.clone(),
            });
        }
        let digest = ledger_digest(&bindings);
        Ok(Self {
            version: COMPOSITION_LEDGER_VERSION,
            bindings,
            digest,
        })
    }

    pub fn for_execution(
        runtime_version: &str,
        executor_id: &str,
        executor_version: &str,
        executor_digest: &str,
        containment_digest: &str,
    ) -> Result<Self, CompositionLedgerError> {
        let executor_component = format!("executor/{executor_id}");
        let ledger = Self::new(vec![
            CompositionBinding::new(
                "pandora-runtime",
                runtime_version,
                CompositionSource::CleanSource,
                hash_artifact(format!("pandora-runtime\0{runtime_version}").as_bytes()),
            )?,
            CompositionBinding::new(
                executor_component.clone(),
                executor_version,
                CompositionSource::Native,
                executor_digest,
            )?,
            CompositionBinding::new(
                "containment",
                "1",
                CompositionSource::CleanSource,
                containment_digest,
            )?,
        ])?;
        ledger.require_components(&[
            "pandora-runtime",
            "containment",
            executor_component.as_str(),
        ])?;
        Ok(ledger)
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub fn bindings(&self) -> &[CompositionBinding] {
        &self.bindings
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn has_component(&self, component: &str) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.component == component)
    }

    fn require_components(&self, components: &[&str]) -> Result<(), CompositionLedgerError> {
        for component in components {
            if !self.has_component(component) {
                return Err(CompositionLedgerError::MissingRequiredComponent(
                    (*component).to_owned(),
                ));
            }
        }
        Ok(())
    }
}

fn ledger_digest(bindings: &[CompositionBinding]) -> String {
    let mut canonical = String::from("pandora-composition-ledger-v1\n");
    for binding in bindings {
        canonical.push_str(binding.component());
        canonical.push('\0');
        canonical.push_str(binding.version());
        canonical.push('\0');
        canonical.push_str(binding.source().as_str());
        canonical.push('\0');
        canonical.push_str(binding.identity_digest());
        canonical.push('\n');
    }
    hash_artifact(canonical.as_bytes())
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), CompositionLedgerError> {
    if value.trim().is_empty() {
        return Err(if field == "component" {
            CompositionLedgerError::EmptyComponent
        } else {
            CompositionLedgerError::EmptyVersion
        });
    }
    if value.len() > 256 || value.contains('\0') {
        return Err(CompositionLedgerError::InvalidField(field));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'+')
    }) {
        return Err(CompositionLedgerError::InvalidField(field));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(value: u8) -> String {
        format!("sha256:{}", format!("{value:02x}").repeat(32))
    }

    fn binding(component: &str, version: &str, source: CompositionSource) -> CompositionBinding {
        CompositionBinding::new(component, version, source, digest(1)).unwrap()
    }

    #[test]
    fn ledger_is_sorted_and_deterministic() {
        let first = CompositionLedger::new(vec![
            binding("gene", "1.0.0", CompositionSource::ThirdParty),
            binding("runtime", "1.0.0", CompositionSource::CleanSource),
        ])
        .unwrap();
        let second = CompositionLedger::new(vec![
            binding("runtime", "1.0.0", CompositionSource::CleanSource),
            binding("gene", "1.0.0", CompositionSource::ThirdParty),
        ])
        .unwrap();

        assert_eq!(first, second);
        assert!(first.digest().starts_with("sha256:"));
        assert_eq!(first.bindings()[0].component(), "gene");
    }

    #[test]
    fn duplicate_identity_is_rejected() {
        assert_eq!(
            CompositionLedger::new(vec![
                binding("gene", "1.0.0", CompositionSource::ThirdParty),
                binding("gene", "1.0.0", CompositionSource::Native),
            ]),
            Err(CompositionLedgerError::DuplicateBinding {
                component: "gene".to_owned(),
                version: "1.0.0".to_owned(),
            })
        );
    }

    #[test]
    fn execution_ledger_requires_runtime_executor_and_containment() {
        let ledger = CompositionLedger::for_execution(
            "2.0.0-beta.7",
            "filesystem",
            "2.0.0",
            &digest(2),
            &digest(3),
        )
        .unwrap();

        assert!(ledger.has_component("pandora-runtime"));
        assert!(ledger.has_component("executor/filesystem"));
        assert!(ledger.has_component("containment"));
        assert_eq!(ledger.version(), COMPOSITION_LEDGER_VERSION);
    }

    #[test]
    fn source_and_identity_change_the_ledger_digest() {
        let clean = CompositionLedger::new(vec![binding(
            "dependency",
            "1.0.0",
            CompositionSource::CleanSource,
        )])
        .unwrap();
        let native = CompositionLedger::new(vec![binding(
            "dependency",
            "1.0.0",
            CompositionSource::Native,
        )])
        .unwrap();
        let changed = CompositionLedger::new(vec![
            CompositionBinding::new(
                "dependency",
                "1.0.0",
                CompositionSource::CleanSource,
                digest(2),
            )
            .unwrap(),
        ])
        .unwrap();

        assert_ne!(clean.digest(), native.digest());
        assert_ne!(clean.digest(), changed.digest());
    }

    #[test]
    fn invalid_identity_digest_is_rejected() {
        assert_eq!(
            CompositionBinding::new("runtime", "1.0.0", CompositionSource::CleanSource, "bad"),
            Err(CompositionLedgerError::InvalidDigest)
        );
    }
}
