use crate::effect::Timestamp;
use crate::ids::{SessionId, TenantId, WorkspaceId};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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
            classification_boundary: self.classification_boundary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextFragment {
    id: String,
    source: ContextSource,
    trust: ContextTrust,
    classification: ContextClassification,
    priority: u8,
    content: String,
    token_cost: u32,
    expires_at: Option<Timestamp>,
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
        let id = validate_text("fragment id", id.into())?;
        let content = content.into();
        if content.contains('\0') {
            return Err(ContextContractError::ControlCharacter("content"));
        }
        Ok(Self {
            id,
            source,
            trust,
            classification,
            priority,
            content,
            token_cost,
            expires_at,
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

    pub const fn token_cost(&self) -> u32 {
        self.token_cost
    }

    pub const fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    pub const fn is_expired(&self, now: Timestamp) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at.as_unix_seconds() <= now.as_unix_seconds(),
            None => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextCacheKey {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    provider: String,
    model: String,
    policy_version: u32,
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

    pub const fn classification_boundary(&self) -> ContextClassification {
        self.classification_boundary
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextReceipt {
    included_ids: Vec<String>,
    dropped_ids: Vec<String>,
    token_cost: u32,
    cacheable: bool,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
