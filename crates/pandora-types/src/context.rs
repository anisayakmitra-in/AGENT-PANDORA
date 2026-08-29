use crate::effect::Timestamp;
use crate::ids::{SessionId, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const CONTEXT_PROJECTION_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ContextClassification {
    Public,
    Internal,
    Sensitive,
    Secret,
}

impl ContextClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Secret => "secret",
        }
    }

    pub const fn cacheable(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ContextSource {
    Constitutional,
    ActivePlan,
    L1Evidence,
    Retrieved,
    Conversation,
}

impl ContextSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Constitutional => "constitutional",
            Self::ActivePlan => "active_plan",
            Self::L1Evidence => "l1_evidence",
            Self::Retrieved => "retrieved",
            Self::Conversation => "conversation",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ContextTrust {
    Constitutional,
    Verified,
    Admitted,
    Unverified,
}

impl ContextTrust {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Constitutional => "constitutional",
            Self::Verified => "verified",
            Self::Admitted => "admitted",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOriginKind {
    Runtime,
    Memory,
    Skill,
    UserSelection,
    Tool,
    Mcp,
    Package,
    Repository,
    Document,
    Issue,
    Design,
    AgentHandoff,
    #[default]
    External,
}

impl ContextOriginKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Memory => "memory",
            Self::Skill => "skill",
            Self::UserSelection => "user_selection",
            Self::Tool => "tool",
            Self::Mcp => "mcp",
            Self::Package => "package",
            Self::Repository => "repository",
            Self::Document => "document",
            Self::Issue => "issue",
            Self::Design => "design",
            Self::AgentHandoff => "agent_handoff",
            Self::External => "external",
        }
    }

    pub const fn is_external(self) -> bool {
        !matches!(self, Self::Runtime | Self::Memory | Self::Skill)
    }

    fn infer(producer: &str) -> Self {
        let producer = producer.to_ascii_lowercase();
        if producer.starts_with("pandora-runtime") {
            Self::Runtime
        } else if producer.starts_with("pandora-memory") {
            Self::Memory
        } else if producer.starts_with("pandora-skill") {
            Self::Skill
        } else if producer.contains("user-selection") || producer.contains("attachment") {
            Self::UserSelection
        } else if producer.contains("handoff") {
            Self::AgentHandoff
        } else if producer.contains("mcp") {
            Self::Mcp
        } else if producer.contains("package") || producer.contains("gene") {
            Self::Package
        } else if producer.contains("repository") || producer.contains("repo") {
            Self::Repository
        } else if producer.contains("document") || producer.contains("file") {
            Self::Document
        } else if producer.contains("issue") {
            Self::Issue
        } else if producer.contains("design") {
            Self::Design
        } else if producer.contains("tool") {
            Self::Tool
        } else {
            Self::External
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextOrigin {
    producer: Option<String>,
    reference: Option<String>,
    #[serde(default)]
    kind: ContextOriginKind,
}

impl ContextOrigin {
    pub fn new(
        producer: impl Into<String>,
        reference: impl Into<String>,
    ) -> Result<Self, ContextContractError> {
        let producer = producer.into();
        Self::new_with_kind(
            producer.clone(),
            reference,
            ContextOriginKind::infer(&producer),
        )
    }

    pub fn new_with_kind(
        producer: impl Into<String>,
        reference: impl Into<String>,
        kind: ContextOriginKind,
    ) -> Result<Self, ContextContractError> {
        Ok(Self {
            producer: Some(validate_origin_text("origin producer", producer.into())?),
            reference: Some(validate_origin_text("origin reference", reference.into())?),
            kind,
        })
    }

    pub const fn incomplete() -> Self {
        Self {
            producer: None,
            reference: None,
            kind: ContextOriginKind::External,
        }
    }

    pub fn producer(&self) -> Option<&str> {
        self.producer.as_deref()
    }

    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    pub const fn kind(&self) -> ContextOriginKind {
        self.kind
    }

    pub const fn is_complete(&self) -> bool {
        self.producer.is_some() && self.reference.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
}

impl fmt::Display for ContextContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
        }
    }
}

impl std::error::Error for ContextContractError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextRequest {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    provider: String,
    model: String,
    policy_version: u32,
    token_budget: u32,
    now: Timestamp,
    classification_boundary: ContextClassification,
}

impl ContextRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        provider: impl Into<String>,
        model: impl Into<String>,
        policy_version: u32,
        token_budget: u32,
        now: Timestamp,
    ) -> Result<Self, ContextContractError> {
        Ok(Self {
            tenant_id,
            workspace_id,
            session_id,
            provider: validate_text("provider", provider.into())?,
            model: validate_text("model", model.into())?,
            policy_version,
            token_budget,
            now,
            classification_boundary: ContextClassification::Internal,
        })
    }

    pub fn with_classification_boundary(mut self, boundary: ContextClassification) -> Self {
        self.classification_boundary = boundary;
        self
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn token_budget(&self) -> u32 {
        self.token_budget
    }

    pub const fn now(&self) -> Timestamp {
        self.now
    }

    pub const fn classification_boundary(&self) -> ContextClassification {
        self.classification_boundary
    }

    pub fn cache_key(&self) -> ContextCacheKey {
        ContextCacheKey {
            tenant_id: self.tenant_id.clone(),
            workspace_id: self.workspace_id.clone(),
            session_id: self.session_id.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            policy_version: self.policy_version,
            projection_version: CONTEXT_PROJECTION_VERSION,
            token_budget: self.token_budget,
            classification_boundary: self.classification_boundary,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextFragment {
    id: String,
    source: ContextSource,
    trust: ContextTrust,
    classification: ContextClassification,
    priority: u8,
    content: String,
    content_digest: String,
    token_cost: u32,
    expires_at: Option<Timestamp>,
    origin: ContextOrigin,
}

impl ContextFragment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        source: ContextSource,
        trust: ContextTrust,
        classification: ContextClassification,
        priority: u8,
        content: impl Into<String>,
        token_cost: u32,
        expires_at: Option<Timestamp>,
    ) -> Result<Self, ContextContractError> {
        Self::new_with_origin(
            id,
            source,
            trust,
            classification,
            priority,
            content,
            token_cost,
            expires_at,
            ContextOrigin::incomplete(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_origin(
        id: impl Into<String>,
        source: ContextSource,
        trust: ContextTrust,
        classification: ContextClassification,
        priority: u8,
        content: impl Into<String>,
        token_cost: u32,
        expires_at: Option<Timestamp>,
        origin: ContextOrigin,
    ) -> Result<Self, ContextContractError> {
        let id = validate_text("fragment id", id.into())?;
        let content = content.into();
        if content.contains('\0') {
            return Err(ContextContractError::ControlCharacter("content"));
        }
        let content_digest = sha256_digest(content.as_bytes());
        Ok(Self {
            id,
            source,
            trust,
            classification,
            priority,
            content,
            content_digest,
            token_cost,
            expires_at,
            origin,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn source(&self) -> ContextSource {
        self.source
    }

    pub const fn trust(&self) -> ContextTrust {
        self.trust
    }

    pub const fn classification(&self) -> ContextClassification {
        self.classification
    }

    pub const fn priority(&self) -> u8 {
        self.priority
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    pub const fn token_cost(&self) -> u32 {
        self.token_cost
    }

    pub const fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    pub const fn origin(&self) -> &ContextOrigin {
        &self.origin
    }

    pub const fn is_expired(&self, now: Timestamp) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at.as_unix_seconds() <= now.as_unix_seconds(),
            None => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextFragmentManifest {
    id: String,
    source: ContextSource,
    trust: ContextTrust,
    classification: ContextClassification,
    priority: u8,
    content_digest: String,
    token_cost: u32,
    expires_at: Option<Timestamp>,
    origin: ContextOrigin,
}

impl ContextFragmentManifest {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    pub const fn origin(&self) -> &ContextOrigin {
        &self.origin
    }
}

impl From<&ContextFragment> for ContextFragmentManifest {
    fn from(fragment: &ContextFragment) -> Self {
        Self {
            id: fragment.id().to_owned(),
            source: fragment.source(),
            trust: fragment.trust(),
            classification: fragment.classification(),
            priority: fragment.priority(),
            content_digest: fragment.content_digest().to_owned(),
            token_cost: fragment.token_cost(),
            expires_at: fragment.expires_at(),
            origin: fragment.origin().clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextManifest {
    fragments: Vec<ContextFragmentManifest>,
    digest: String,
    provenance_complete: bool,
}

impl ContextManifest {
    pub fn from_fragments(fragments: &[ContextFragment]) -> Self {
        let fragments = fragments
            .iter()
            .map(ContextFragmentManifest::from)
            .collect::<Vec<_>>();
        let provenance_complete = fragments
            .iter()
            .all(|fragment| fragment.origin.is_complete());
        let digest = digest_manifest(&fragments);
        Self {
            fragments,
            digest,
            provenance_complete,
        }
    }

    pub fn fragments(&self) -> &[ContextFragmentManifest] {
        &self.fragments
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub const fn provenance_complete(&self) -> bool {
        self.provenance_complete
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextCacheKey {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    provider: String,
    model: String,
    policy_version: u32,
    projection_version: u32,
    token_budget: u32,
    classification_boundary: ContextClassification,
}

impl ContextCacheKey {
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn projection_version(&self) -> u32 {
        self.projection_version
    }

    pub const fn token_budget(&self) -> u32 {
        self.token_budget
    }

    pub const fn classification_boundary(&self) -> ContextClassification {
        self.classification_boundary
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextEntry {
    id: String,
    source: ContextSource,
    trust: ContextTrust,
    classification: ContextClassification,
    content: String,
    token_cost: u32,
    compressed: bool,
}

impl ContextEntry {
    pub fn new(
        id: impl Into<String>,
        source: ContextSource,
        trust: ContextTrust,
        classification: ContextClassification,
        content: impl Into<String>,
        token_cost: u32,
        compressed: bool,
    ) -> Self {
        Self {
            id: id.into(),
            source,
            trust,
            classification,
            content: content.into(),
            token_cost,
            compressed,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn source(&self) -> ContextSource {
        self.source
    }

    pub const fn trust(&self) -> ContextTrust {
        self.trust
    }

    pub const fn classification(&self) -> ContextClassification {
        self.classification
    }

    pub const fn token_cost(&self) -> u32 {
        self.token_cost
    }

    pub const fn compressed(&self) -> bool {
        self.compressed
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ContextCacheDisposition {
    Hit,
    Miss,
    Bypass,
}

impl ContextCacheDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Bypass => "bypass",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextReceipt {
    included_ids: Vec<String>,
    dropped_ids: Vec<String>,
    token_cost: u32,
    cacheable: bool,
    manifest_digest: Option<String>,
    projection_version: u32,
    provenance_complete: bool,
    cache_disposition: ContextCacheDisposition,
}

impl ContextReceipt {
    pub fn new(
        included_ids: Vec<String>,
        dropped_ids: Vec<String>,
        token_cost: u32,
        cacheable: bool,
    ) -> Self {
        Self {
            included_ids,
            dropped_ids,
            token_cost,
            cacheable,
            manifest_digest: None,
            projection_version: CONTEXT_PROJECTION_VERSION,
            provenance_complete: false,
            cache_disposition: ContextCacheDisposition::Bypass,
        }
    }

    pub fn new_with_evidence(
        included_ids: Vec<String>,
        dropped_ids: Vec<String>,
        token_cost: u32,
        cacheable: bool,
        manifest: &ContextManifest,
        cache_disposition: ContextCacheDisposition,
    ) -> Self {
        Self {
            included_ids,
            dropped_ids,
            token_cost,
            cacheable,
            manifest_digest: Some(manifest.digest().to_owned()),
            projection_version: CONTEXT_PROJECTION_VERSION,
            provenance_complete: manifest.provenance_complete(),
            cache_disposition,
        }
    }

    pub fn included_ids(&self) -> &[String] {
        &self.included_ids
    }

    pub fn dropped_ids(&self) -> &[String] {
        &self.dropped_ids
    }

    pub const fn token_cost(&self) -> u32 {
        self.token_cost
    }

    pub const fn cacheable(&self) -> bool {
        self.cacheable
    }

    pub fn manifest_digest(&self) -> Option<&str> {
        self.manifest_digest.as_deref()
    }

    pub const fn projection_version(&self) -> u32 {
        self.projection_version
    }

    pub const fn provenance_complete(&self) -> bool {
        self.provenance_complete
    }

    pub const fn cache_disposition(&self) -> ContextCacheDisposition {
        self.cache_disposition
    }

    pub fn with_cache_disposition(mut self, disposition: ContextCacheDisposition) -> Self {
        self.cache_disposition = disposition;
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextAssembly {
    entries: Vec<ContextEntry>,
    text: String,
    receipt: ContextReceipt,
    cache_key: ContextCacheKey,
}

impl ContextAssembly {
    pub fn new(
        entries: Vec<ContextEntry>,
        text: String,
        receipt: ContextReceipt,
        cache_key: ContextCacheKey,
    ) -> Self {
        Self {
            entries,
            text,
            receipt,
            cache_key,
        }
    }

    pub fn entries(&self) -> &[ContextEntry] {
        &self.entries
    }

    pub fn item_ids(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.id.clone()).collect()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn receipt(&self) -> &ContextReceipt {
        &self.receipt
    }

    pub fn cache_key(&self) -> &ContextCacheKey {
        &self.cache_key
    }
}

fn validate_text(field: &'static str, value: String) -> Result<String, ContextContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ContextContractError::EmptyField(field));
    }
    if value.len() > 4096 {
        return Err(ContextContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(ContextContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

fn validate_origin_text(
    field: &'static str,
    value: String,
) -> Result<String, ContextContractError> {
    let value = validate_text(field, value)?;
    if value.len() > 512 {
        return Err(ContextContractError::FieldTooLong(field));
    }
    Ok(value)
}

fn digest_manifest(fragments: &[ContextFragmentManifest]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-context-manifest");
    hasher.update(CONTEXT_PROJECTION_VERSION.to_be_bytes());
    hasher.update(
        u64::try_from(fragments.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for fragment in fragments {
        digest_text(&mut hasher, &fragment.id);
        digest_text(&mut hasher, fragment.source.as_str());
        digest_text(&mut hasher, fragment.trust.as_str());
        digest_text(&mut hasher, fragment.classification.as_str());
        hasher.update([fragment.priority]);
        digest_text(&mut hasher, &fragment.content_digest);
        hasher.update(fragment.token_cost.to_be_bytes());
        match fragment.expires_at {
            Some(expires_at) => {
                hasher.update([1]);
                hasher.update(expires_at.as_unix_seconds().to_be_bytes());
            }
            None => hasher.update([0]),
        }
        digest_optional_text(&mut hasher, fragment.origin.producer());
        digest_optional_text(&mut hasher, fragment.origin.reference());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn sha256_digest(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn digest_optional_text(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            digest_text(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(token_budget: u32) -> ContextRequest {
        ContextRequest::new(
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            "provider-a",
            "model-a",
            7,
            token_budget,
            Timestamp::from_unix_seconds(100),
        )
        .unwrap()
    }

    fn fragment(origin: ContextOrigin) -> ContextFragment {
        ContextFragment::new_with_origin(
            "constitution",
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            100,
            "constitutional rules",
            4,
            None,
            origin,
        )
        .unwrap()
    }

    #[test]
    fn fragment_origin_and_content_digest_are_stable() {
        let origin = ContextOrigin::new("pandora-runtime", "agent.system-prompt").unwrap();
        let first = fragment(origin.clone());
        let second = fragment(origin);

        assert_eq!(first.origin(), second.origin());
        assert!(first.origin().is_complete());
        assert_eq!(first.content_digest(), second.content_digest());
        assert!(first.content_digest().starts_with("sha256:"));
    }

    #[test]
    fn origin_kind_is_inferred_for_external_sources() {
        assert_eq!(
            ContextOrigin::new("pandora-mcp", "server.result")
                .unwrap()
                .kind(),
            ContextOriginKind::Mcp
        );
        assert_eq!(
            ContextOrigin::new("pandora-package", "gene.wasm")
                .unwrap()
                .kind(),
            ContextOriginKind::Package
        );
        assert_eq!(
            ContextOrigin::new("pandora-repository", "issue-42")
                .unwrap()
                .kind(),
            ContextOriginKind::Repository
        );
        assert_eq!(
            ContextOrigin::new("pandora-document", "README.md")
                .unwrap()
                .kind(),
            ContextOriginKind::Document
        );
        assert!(ContextOriginKind::Mcp.is_external());
        assert!(!ContextOriginKind::Runtime.is_external());
        let encoded =
            serde_json::to_value(ContextOrigin::new("pandora-mcp", "server.result").unwrap())
                .unwrap();
        assert_eq!(encoded["kind"], "mcp");
    }

    #[test]
    fn manifest_digest_changes_when_provenance_changes() {
        let first = ContextManifest::from_fragments(&[fragment(
            ContextOrigin::new("pandora-runtime", "agent.system-prompt").unwrap(),
        )]);
        let second = ContextManifest::from_fragments(&[fragment(
            ContextOrigin::new("pandora-runtime", "agent.revised-system-prompt").unwrap(),
        )]);

        assert!(first.provenance_complete());
        assert!(second.provenance_complete());
        assert_ne!(first.digest(), second.digest());
    }

    #[test]
    fn cache_key_includes_projection_version_and_token_budget() {
        let first = request(20).cache_key();
        let second = request(21).cache_key();

        assert_eq!(first.projection_version(), CONTEXT_PROJECTION_VERSION);
        assert_eq!(first.token_budget(), 20);
        assert_eq!(second.token_budget(), 21);
        assert_ne!(first, second);
    }
}
