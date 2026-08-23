use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

pub const EXECUTION_PROFILE_VERSION: u16 = 1;
const MAX_ID_BYTES: usize = 256;
const MAX_VERSION_BYTES: usize = 128;
const MAX_WORKSPACE_IDENTITY_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionProfileContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    InvalidField(&'static str),
    InvalidDigest(&'static str),
    MissingExecutor,
    MultipleExecutors,
    DuplicateBinding {
        kind: ExecutionProfileBindingKind,
        id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExecutionProfileDigest(String);

impl ExecutionProfileDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProfileBindingKind {
    Executor,
    Harness,
    Gene,
    Provider,
    Model,
    ToolCatalog,
    Artifact,
    Configuration,
}

impl ExecutionProfileBindingKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Executor => "executor",
            Self::Harness => "harness",
            Self::Gene => "gene",
            Self::Provider => "provider",
            Self::Model => "model",
            Self::ToolCatalog => "tool_catalog",
            Self::Artifact => "artifact",
            Self::Configuration => "configuration",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionProfileBinding {
    kind: ExecutionProfileBindingKind,
    id: String,
    version: Option<String>,
    digest: String,
}

impl ExecutionProfileBinding {
    pub fn new(
        kind: ExecutionProfileBindingKind,
        id: impl Into<String>,
        version: Option<&str>,
        digest: impl Into<String>,
    ) -> Result<Self, ExecutionProfileContractError> {
        Ok(Self {
            kind,
            id: validate_identifier("binding ID", id.into(), MAX_ID_BYTES)?,
            version: version
                .map(|value| {
                    validate_identifier("binding version", value.to_owned(), MAX_VERSION_BYTES)
                })
                .transpose()?,
            digest: validate_digest("binding digest", digest.into())?,
        })
    }

    pub const fn kind(&self) -> ExecutionProfileBindingKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionProfile {
    version: u16,
    authority: &'static str,
    runtime_version: String,
    platform_os: String,
    platform_architecture: String,
    policy_version: u32,
    workspace_digest: String,
    containment_digest: String,
    bindings: Vec<ExecutionProfileBinding>,
    #[serde(rename = "execution_profile_digest")]
    digest: ExecutionProfileDigest,
}

impl ExecutionProfile {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime_version: impl Into<String>,
        platform_os: impl Into<String>,
        platform_architecture: impl Into<String>,
        policy_version: u32,
        workspace_identity: &str,
        containment_digest: impl Into<String>,
        bindings: Vec<ExecutionProfileBinding>,
    ) -> Result<Self, ExecutionProfileContractError> {
        let workspace_identity = validate_text(
            "workspace identity",
            workspace_identity.to_owned(),
            MAX_WORKSPACE_IDENTITY_BYTES,
        )?;
        let workspace_digest = digest_value("pandora-workspace-identity-v1", &workspace_identity);
        Self::from_digests(
            runtime_version.into(),
            platform_os.into(),
            platform_architecture.into(),
            policy_version,
            workspace_digest,
            containment_digest.into(),
            bindings,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_digests(
        runtime_version: String,
        platform_os: String,
        platform_architecture: String,
        policy_version: u32,
        workspace_digest: String,
        containment_digest: String,
        mut bindings: Vec<ExecutionProfileBinding>,
    ) -> Result<Self, ExecutionProfileContractError> {
        let runtime_version =
            validate_identifier("runtime version", runtime_version, MAX_VERSION_BYTES)?;
        let platform_os = validate_identifier("platform OS", platform_os, MAX_ID_BYTES)?;
        let platform_architecture =
            validate_identifier("platform architecture", platform_architecture, MAX_ID_BYTES)?;
        let workspace_digest = validate_digest("workspace digest", workspace_digest)?;
        let containment_digest = validate_digest("containment digest", containment_digest)?;
        bindings.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
        if let Some(duplicate) = bindings.windows(2).find_map(|pair| {
            (pair[0].kind == pair[1].kind && pair[0].id == pair[1].id).then(|| {
                ExecutionProfileContractError::DuplicateBinding {
                    kind: pair[0].kind,
                    id: pair[0].id.clone(),
                }
            })
        }) {
            return Err(duplicate);
        }
        match bindings
            .iter()
            .filter(|binding| binding.kind == ExecutionProfileBindingKind::Executor)
            .count()
        {
            0 => return Err(ExecutionProfileContractError::MissingExecutor),
            1 => {}
            _ => return Err(ExecutionProfileContractError::MultipleExecutors),
        }
        let digest = profile_digest(
            &runtime_version,
            &platform_os,
            &platform_architecture,
            policy_version,
            &workspace_digest,
            &containment_digest,
            &bindings,
        );
        Ok(Self {
            version: EXECUTION_PROFILE_VERSION,
            authority: "evidence_only",
            runtime_version,
            platform_os,
            platform_architecture,
            policy_version,
            workspace_digest,
            containment_digest,
            bindings,
            digest,
        })
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub fn runtime_version(&self) -> &str {
        &self.runtime_version
    }

    pub fn platform_os(&self) -> &str {
        &self.platform_os
    }

    pub fn platform_architecture(&self) -> &str {
        &self.platform_architecture
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn workspace_digest(&self) -> &str {
        &self.workspace_digest
    }

    pub fn containment_digest(&self) -> &str {
        &self.containment_digest
    }

    pub fn bindings(&self) -> &[ExecutionProfileBinding] {
        &self.bindings
    }

    pub fn digest(&self) -> &ExecutionProfileDigest {
        &self.digest
    }
}

#[derive(Deserialize)]
struct SerializedExecutionProfile {
    version: u16,
    authority: String,
    runtime_version: String,
    platform_os: String,
    platform_architecture: String,
    policy_version: u32,
    workspace_digest: String,
    containment_digest: String,
    bindings: Vec<SerializedExecutionProfileBinding>,
    execution_profile_digest: String,
}

#[derive(Deserialize)]
struct SerializedExecutionProfileBinding {
    kind: SerializedExecutionProfileBindingKind,
    id: String,
    version: Option<String>,
    digest: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SerializedExecutionProfileBindingKind {
    Executor,
    Harness,
    Gene,
    Provider,
    Model,
    ToolCatalog,
    Artifact,
    Configuration,
}

impl From<SerializedExecutionProfileBindingKind> for ExecutionProfileBindingKind {
    fn from(value: SerializedExecutionProfileBindingKind) -> Self {
        match value {
            SerializedExecutionProfileBindingKind::Executor => Self::Executor,
            SerializedExecutionProfileBindingKind::Harness => Self::Harness,
            SerializedExecutionProfileBindingKind::Gene => Self::Gene,
            SerializedExecutionProfileBindingKind::Provider => Self::Provider,
            SerializedExecutionProfileBindingKind::Model => Self::Model,
            SerializedExecutionProfileBindingKind::ToolCatalog => Self::ToolCatalog,
            SerializedExecutionProfileBindingKind::Artifact => Self::Artifact,
            SerializedExecutionProfileBindingKind::Configuration => Self::Configuration,
        }
    }
}

impl<'de> Deserialize<'de> for ExecutionProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serialized = SerializedExecutionProfile::deserialize(deserializer)?;
        if serialized.version != EXECUTION_PROFILE_VERSION
            || serialized.authority != "evidence_only"
        {
            return Err(serde::de::Error::custom("unsupported execution profile"));
        }
        let bindings = serialized
            .bindings
            .into_iter()
            .map(|binding| {
                ExecutionProfileBinding::new(
                    binding.kind.into(),
                    binding.id,
                    binding.version.as_deref(),
                    binding.digest,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| serde::de::Error::custom("invalid execution profile binding"))?;
        let profile = Self::from_digests(
            serialized.runtime_version,
            serialized.platform_os,
            serialized.platform_architecture,
            serialized.policy_version,
            serialized.workspace_digest,
            serialized.containment_digest,
            bindings,
        )
        .map_err(|_| serde::de::Error::custom("invalid execution profile"))?;
        if profile.digest.as_str() != serialized.execution_profile_digest {
            return Err(serde::de::Error::custom(
                "execution profile digest mismatch",
            ));
        }
        Ok(profile)
    }
}

fn profile_digest(
    runtime_version: &str,
    platform_os: &str,
    platform_architecture: &str,
    policy_version: u32,
    workspace_digest: &str,
    containment_digest: &str,
    bindings: &[ExecutionProfileBinding],
) -> ExecutionProfileDigest {
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, "pandora-execution-profile-v1");
    digest_text(&mut hasher, runtime_version);
    digest_text(&mut hasher, platform_os);
    digest_text(&mut hasher, platform_architecture);
    hasher.update(policy_version.to_be_bytes());
    digest_text(&mut hasher, workspace_digest);
    digest_text(&mut hasher, containment_digest);
    for binding in bindings {
        digest_text(&mut hasher, binding.kind.as_str());
        digest_text(&mut hasher, &binding.id);
        match binding.version.as_deref() {
            Some(version) => {
                hasher.update([1]);
                digest_text(&mut hasher, version);
            }
            None => hasher.update([0]),
        }
        digest_text(&mut hasher, &binding.digest);
    }
    ExecutionProfileDigest(format!(
        "pandora-execution-profile-v1:sha256:{:x}",
        hasher.finalize()
    ))
}

fn digest_value(domain: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, domain);
    digest_text(&mut hasher, value);
    format!("sha256:{:x}", hasher.finalize())
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, ExecutionProfileContractError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(ExecutionProfileContractError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(ExecutionProfileContractError::FieldTooLong(field));
    }
    Ok(value)
}

fn validate_identifier(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, ExecutionProfileContractError> {
    let value = validate_text(field, value, max_bytes)?;
    let valid = value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'+')
    });
    valid
        .then_some(value)
        .ok_or(ExecutionProfileContractError::InvalidField(field))
}

fn validate_digest(
    field: &'static str,
    value: String,
) -> Result<String, ExecutionProfileContractError> {
    let valid = value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
    valid
        .then_some(value)
        .ok_or(ExecutionProfileContractError::InvalidDigest(field))
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind,
        ExecutionProfileContractError,
    };

    fn binding(
        kind: ExecutionProfileBindingKind,
        id: &str,
        version: Option<&str>,
        digest_suffix: char,
    ) -> ExecutionProfileBinding {
        ExecutionProfileBinding::new(
            kind,
            id,
            version,
            format!("sha256:{}", digest_suffix.to_string().repeat(64)),
        )
        .unwrap()
    }

    fn profile(
        workspace: &str,
        policy_version: u32,
        containment_suffix: char,
        bindings: Vec<ExecutionProfileBinding>,
    ) -> ExecutionProfile {
        ExecutionProfile::new(
            "2.0.0-alpha.6",
            "windows",
            "x86_64",
            policy_version,
            workspace,
            format!("sha256:{}", containment_suffix.to_string().repeat(64)),
            bindings,
        )
        .unwrap()
    }

    #[test]
    fn profile_digest_is_deterministic_across_binding_order() {
        let executor = binding(
            ExecutionProfileBindingKind::Executor,
            "filesystem",
            Some("2.0.0-alpha.6"),
            '1',
        );
        let gene = binding(
            ExecutionProfileBindingKind::Gene,
            "workspace.read",
            Some("0.1.0"),
            '2',
        );

        let first = profile(
            r"C:\work\pandora",
            1,
            '3',
            vec![gene.clone(), executor.clone()],
        );
        let second = profile(r"C:\work\pandora", 1, '3', vec![executor, gene]);

        assert_eq!(first.digest(), second.digest());
        assert_eq!(
            first.bindings()[0].kind(),
            ExecutionProfileBindingKind::Executor
        );
        assert_eq!(
            first.bindings()[1].kind(),
            ExecutionProfileBindingKind::Gene
        );
    }

    #[test]
    fn profile_digest_distinguishes_absent_and_literal_none_versions() {
        let without_version = profile(
            r"C:\work\pandora",
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                None,
                '1',
            )],
        );
        let literal_none_version = profile(
            r"C:\work\pandora",
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                Some("none"),
                '1',
            )],
        );

        assert_ne!(without_version.digest(), literal_none_version.digest());
    }

    #[test]
    fn profile_digest_covers_runtime_platform_policy_workspace_containment_and_bindings() {
        let baseline_binding = binding(
            ExecutionProfileBindingKind::Executor,
            "filesystem",
            Some("2.0.0-alpha.6"),
            '1',
        );
        let baseline = profile(r"C:\work\pandora", 1, '2', vec![baseline_binding.clone()]);
        let changed_runtime = ExecutionProfile::new(
            "2.0.0-alpha.7",
            "windows",
            "x86_64",
            1,
            r"C:\work\pandora",
            format!("sha256:{}", "2".repeat(64)),
            vec![baseline_binding.clone()],
        )
        .unwrap();
        let changed_platform = ExecutionProfile::new(
            "2.0.0-alpha.6",
            "linux",
            "x86_64",
            1,
            r"C:\work\pandora",
            format!("sha256:{}", "2".repeat(64)),
            vec![baseline_binding.clone()],
        )
        .unwrap();
        let changed_policy = profile(r"C:\work\pandora", 2, '2', vec![baseline_binding.clone()]);
        let changed_workspace = profile(r"C:\work\other", 1, '2', vec![baseline_binding.clone()]);
        let changed_containment = profile(r"C:\work\pandora", 1, '3', vec![baseline_binding]);
        let changed_binding = profile(
            r"C:\work\pandora",
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                Some("2.0.0-alpha.6"),
                '4',
            )],
        );

        for changed in [
            changed_runtime,
            changed_platform,
            changed_policy,
            changed_workspace,
            changed_containment,
            changed_binding,
        ] {
            assert_ne!(baseline.digest(), changed.digest());
        }
    }

    #[test]
    fn serialized_profile_does_not_expose_the_workspace_identity() {
        let raw_workspace = r"C:\Users\alice\secret-client";
        let profile = profile(
            raw_workspace,
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                Some("2.0.0-alpha.6"),
                '1',
            )],
        );

        let json = serde_json::to_string(&profile).unwrap();

        assert!(!json.contains(raw_workspace));
        assert!(json.contains("workspace_digest"));
        assert!(json.contains("execution_profile_digest"));
    }

    #[test]
    fn duplicate_binding_identity_is_rejected() {
        let executor = binding(
            ExecutionProfileBindingKind::Executor,
            "filesystem",
            Some("2.0.0-alpha.6"),
            '1',
        );

        assert_eq!(
            ExecutionProfile::new(
                "2.0.0-alpha.6",
                "windows",
                "x86_64",
                1,
                r"C:\work\pandora",
                format!("sha256:{}", "2".repeat(64)),
                vec![executor.clone(), executor],
            ),
            Err(ExecutionProfileContractError::DuplicateBinding {
                kind: ExecutionProfileBindingKind::Executor,
                id: "filesystem".to_owned(),
            })
        );
    }

    #[test]
    fn deserialization_rejects_fields_changed_behind_the_profile_digest() {
        let profile = profile(
            r"C:\work\pandora",
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                Some("2.0.0-alpha.6"),
                '1',
            )],
        );
        let mut value = serde_json::to_value(profile).unwrap();
        value["policy_version"] = serde_json::json!(99);

        assert!(serde_json::from_value::<ExecutionProfile>(value).is_err());
    }

    #[test]
    fn deserialization_rejects_absent_version_changed_to_literal_none() {
        let profile = profile(
            r"C:\work\pandora",
            1,
            '2',
            vec![binding(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                None,
                '1',
            )],
        );
        let mut value = serde_json::to_value(profile).unwrap();
        value["bindings"][0]["version"] = serde_json::json!("none");

        assert!(serde_json::from_value::<ExecutionProfile>(value).is_err());
    }

    #[test]
    fn noncanonical_digest_and_identifier_text_is_rejected() {
        assert_eq!(
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                None,
                format!("sha256:{}", "A".repeat(64)),
            ),
            Err(ExecutionProfileContractError::InvalidDigest(
                "binding digest"
            ))
        );
        assert_eq!(
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                "filesystem\nsecret",
                None,
                format!("sha256:{}", "a".repeat(64)),
            ),
            Err(ExecutionProfileContractError::InvalidField("binding ID"))
        );
    }
}
