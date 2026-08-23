use crate::effect::{EffectOutcome, Timestamp};
use crate::events::EventType;
use crate::ids::{
    EventId, ExecutionId, GeneId, HarnessId, IdError, PermitId, ReceiptId, RequestDigest,
    SessionId, TenantId, WorkspaceId,
};
use sha2::{Digest, Sha256};
use std::fmt;

pub const ROLLOUT_PROJECTION_VERSION: u16 = 1;
pub const MAX_ROLLOUT_RECORDS: usize = 4096;
const MAX_LABEL_BYTES: usize = 256;
const MAX_REASON_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RolloutContractError {
    InvalidId(IdError),
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    InvalidCode(&'static str),
    InvalidDigest,
    EmptyRecords,
    TooManyRecords,
    InvalidSequence,
    ScopeMismatch,
    PreviousDigestMismatch,
    RecordDigestMismatch,
    FinalDigestMismatch,
}

impl fmt::Display for RolloutContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::InvalidCode(field) => write!(formatter, "{field} is not a bounded code"),
            Self::InvalidDigest => formatter.write_str("rollout digest is invalid"),
            Self::EmptyRecords => formatter.write_str("rollout requires at least one record"),
            Self::TooManyRecords => formatter.write_str("rollout exceeds its record limit"),
            Self::InvalidSequence => formatter.write_str("rollout record sequence is invalid"),
            Self::ScopeMismatch => formatter.write_str("rollout record scope does not match"),
            Self::PreviousDigestMismatch => {
                formatter.write_str("rollout record linkage does not match")
            }
            Self::RecordDigestMismatch => {
                formatter.write_str("rollout record digest does not match")
            }
            Self::FinalDigestMismatch => formatter.write_str("rollout final digest does not match"),
        }
    }
}

impl std::error::Error for RolloutContractError {}

impl From<IdError> for RolloutContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolloutDigest(String);

impl RolloutDigest {
    pub fn new(value: impl Into<String>) -> Result<Self, RolloutContractError> {
        let value = value.into();
        if !is_sha256_digest(&value) {
            return Err(RolloutContractError::InvalidDigest);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolloutScope {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    execution_id: ExecutionId,
}

impl RolloutScope {
    pub fn new(
        tenant_id: impl Into<String>,
        workspace_id: impl Into<String>,
        session_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Result<Self, RolloutContractError> {
        Ok(Self {
            tenant_id: TenantId::new(tenant_id)?,
            workspace_id: WorkspaceId::new(workspace_id)?,
            session_id: SessionId::new(session_id)?,
            execution_id: ExecutionId::new(execution_id)?,
        })
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

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn digest(&self) -> RolloutDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"pandora-rollout-scope");
        digest_text(&mut hasher, self.tenant_id.as_str());
        digest_text(&mut hasher, self.workspace_id.as_str());
        digest_text(&mut hasher, self.session_id.as_str());
        digest_text(&mut hasher, self.execution_id.as_str());
        finish_digest(hasher)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutRecordKind {
    ContextManifest,
    RuntimeEvent,
    EffectReceipt,
}

impl RolloutRecordKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ContextManifest => "context_manifest",
            Self::RuntimeEvent => "runtime_event",
            Self::EffectReceipt => "effect_receipt",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RolloutEventEvidence {
    Empty,
    Effect {
        capability: String,
        request_digest: RequestDigest,
    },
    Policy {
        reason_digest: RolloutDigest,
    },
    Failure {
        code: String,
    },
    ProviderCall {
        provider: String,
        request_digest: RequestDigest,
    },
    McpEra {
        server: String,
        era: String,
        downgraded: bool,
    },
}

impl RolloutEventEvidence {
    pub fn effect(
        capability: impl Into<String>,
        request_digest: RequestDigest,
    ) -> Result<Self, RolloutContractError> {
        Ok(Self::Effect {
            capability: validate_code("capability", capability.into())?,
            request_digest,
        })
    }

    pub fn policy(reason: &str) -> Result<Self, RolloutContractError> {
        Ok(Self::Policy {
            reason_digest: digest_sensitive("pandora-rollout-policy-reason", reason)?,
        })
    }

    pub fn failure(code: impl Into<String>) -> Result<Self, RolloutContractError> {
        Ok(Self::Failure {
            code: validate_code("failure code", code.into())?,
        })
    }

    pub fn provider_call(
        provider: impl Into<String>,
        request_digest: RequestDigest,
    ) -> Result<Self, RolloutContractError> {
        Ok(Self::ProviderCall {
            provider: validate_label("provider", provider.into())?,
            request_digest,
        })
    }

    pub fn mcp_era(
        server: impl Into<String>,
        era: impl Into<String>,
        downgraded: bool,
    ) -> Result<Self, RolloutContractError> {
        Ok(Self::McpEra {
            server: validate_label("MCP server", server.into())?,
            era: validate_code("MCP era", era.into())?,
            downgraded,
        })
    }

    fn validate(&self) -> Result<(), RolloutContractError> {
        match self {
            Self::Empty => Ok(()),
            Self::Effect {
                capability,
                request_digest,
            } => {
                validate_code("capability", capability.clone())?;
                validate_identifier("request digest", request_digest.as_str())
            }
            Self::Policy { reason_digest } => {
                RolloutDigest::new(reason_digest.as_str()).map(|_| ())
            }
            Self::Failure { code } => validate_code("failure code", code.clone()).map(|_| ()),
            Self::ProviderCall {
                provider,
                request_digest,
            } => {
                validate_label("provider", provider.clone())?;
                validate_identifier("request digest", request_digest.as_str())
            }
            Self::McpEra {
                server,
                era,
                downgraded: _,
            } => {
                validate_label("MCP server", server.clone())?;
                validate_code("MCP era", era.clone()).map(|_| ())
            }
        }
    }

    fn digest_into(&self, hasher: &mut Sha256) {
        match self {
            Self::Empty => digest_text(hasher, "empty"),
            Self::Effect {
                capability,
                request_digest,
            } => {
                digest_text(hasher, "effect");
                digest_text(hasher, capability);
                digest_text(hasher, request_digest.as_str());
            }
            Self::Policy { reason_digest } => {
                digest_text(hasher, "policy");
                digest_text(hasher, reason_digest.as_str());
            }
            Self::Failure { code } => {
                digest_text(hasher, "failure");
                digest_text(hasher, code);
            }
            Self::ProviderCall {
                provider,
                request_digest,
            } => {
                digest_text(hasher, "provider_call");
                digest_text(hasher, provider);
                digest_text(hasher, request_digest.as_str());
            }
            Self::McpEra {
                server,
                era,
                downgraded,
            } => {
                digest_text(hasher, "mcp_era");
                digest_text(hasher, server);
                digest_text(hasher, era);
                hasher.update([u8::from(*downgraded)]);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RolloutEffectOutcome {
    Succeeded,
    Failed { code: String },
    Denied { reason_digest: RolloutDigest },
}

impl RolloutEffectOutcome {
    pub fn from_effect_outcome(outcome: &EffectOutcome) -> Result<Self, RolloutContractError> {
        match outcome {
            EffectOutcome::Succeeded => Ok(Self::Succeeded),
            EffectOutcome::Failed { code } => Ok(Self::Failed {
                code: validate_code("effect failure code", code.clone())?,
            }),
            EffectOutcome::Denied { reason } => Ok(Self::Denied {
                reason_digest: digest_sensitive("pandora-rollout-denial-reason", reason)?,
            }),
        }
    }

    fn validate(&self) -> Result<(), RolloutContractError> {
        match self {
            Self::Succeeded => Ok(()),
            Self::Failed { code } => validate_code("effect failure code", code.clone()).map(|_| ()),
            Self::Denied { reason_digest } => {
                RolloutDigest::new(reason_digest.as_str()).map(|_| ())
            }
        }
    }

    fn digest_into(&self, hasher: &mut Sha256) {
        match self {
            Self::Succeeded => digest_text(hasher, "succeeded"),
            Self::Failed { code } => {
                digest_text(hasher, "failed");
                digest_text(hasher, code);
            }
            Self::Denied { reason_digest } => {
                digest_text(hasher, "denied");
                digest_text(hasher, reason_digest.as_str());
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RolloutEvidence {
    ContextManifest {
        manifest_digest: RolloutDigest,
    },
    RuntimeEvent {
        event_id: EventId,
        event_type: EventType,
        harness_id: Option<HarnessId>,
        gene_id: Option<GeneId>,
        policy_version: Option<u32>,
        receipt_id: Option<ReceiptId>,
        payload: RolloutEventEvidence,
    },
    EffectReceipt {
        receipt_id: ReceiptId,
        permit_id: PermitId,
        request_digest: RequestDigest,
        completed_at: Timestamp,
        outcome: RolloutEffectOutcome,
    },
}

impl RolloutEvidence {
    pub const fn kind(&self) -> RolloutRecordKind {
        match self {
            Self::ContextManifest { .. } => RolloutRecordKind::ContextManifest,
            Self::RuntimeEvent { .. } => RolloutRecordKind::RuntimeEvent,
            Self::EffectReceipt { .. } => RolloutRecordKind::EffectReceipt,
        }
    }

    fn validate(&self) -> Result<(), RolloutContractError> {
        match self {
            Self::ContextManifest { manifest_digest } => {
                RolloutDigest::new(manifest_digest.as_str()).map(|_| ())
            }
            Self::RuntimeEvent {
                event_id,
                harness_id,
                gene_id,
                receipt_id,
                payload,
                ..
            } => {
                validate_identifier("event ID", event_id.as_str())?;
                validate_optional_identifier(
                    "harness ID",
                    harness_id.as_ref().map(HarnessId::as_str),
                )?;
                validate_optional_identifier("gene ID", gene_id.as_ref().map(GeneId::as_str))?;
                validate_optional_identifier(
                    "receipt ID",
                    receipt_id.as_ref().map(ReceiptId::as_str),
                )?;
                payload.validate()
            }
            Self::EffectReceipt {
                receipt_id,
                permit_id,
                request_digest,
                outcome,
                ..
            } => {
                validate_identifier("receipt ID", receipt_id.as_str())?;
                validate_identifier("permit ID", permit_id.as_str())?;
                validate_identifier("request digest", request_digest.as_str())?;
                outcome.validate()
            }
        }
    }

    fn digest_into(&self, hasher: &mut Sha256) {
        digest_text(hasher, self.kind().as_str());
        match self {
            Self::ContextManifest { manifest_digest } => {
                digest_text(hasher, manifest_digest.as_str());
            }
            Self::RuntimeEvent {
                event_id,
                event_type,
                harness_id,
                gene_id,
                policy_version,
                receipt_id,
                payload,
            } => {
                digest_text(hasher, event_id.as_str());
                digest_text(hasher, event_type_code(*event_type));
                digest_optional_text(hasher, harness_id.as_ref().map(HarnessId::as_str));
                digest_optional_text(hasher, gene_id.as_ref().map(GeneId::as_str));
                digest_optional_u32(hasher, *policy_version);
                digest_optional_text(hasher, receipt_id.as_ref().map(ReceiptId::as_str));
                payload.digest_into(hasher);
            }
            Self::EffectReceipt {
                receipt_id,
                permit_id,
                request_digest,
                completed_at,
                outcome,
            } => {
                digest_text(hasher, receipt_id.as_str());
                digest_text(hasher, permit_id.as_str());
                digest_text(hasher, request_digest.as_str());
                hasher.update(completed_at.as_unix_seconds().to_be_bytes());
                outcome.digest_into(hasher);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolloutRecord {
    sequence: u32,
    scope_digest: RolloutDigest,
    previous_digest: Option<RolloutDigest>,
    evidence: RolloutEvidence,
    digest: RolloutDigest,
}

impl RolloutRecord {
    pub fn link(
        scope: &RolloutScope,
        sequence: u32,
        previous_digest: Option<RolloutDigest>,
        evidence: RolloutEvidence,
    ) -> Result<Self, RolloutContractError> {
        evidence.validate()?;
        let scope_digest = scope.digest();
        let digest = digest_record(sequence, &scope_digest, previous_digest.as_ref(), &evidence);
        Ok(Self {
            sequence,
            scope_digest,
            previous_digest,
            evidence,
            digest,
        })
    }

    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    pub fn scope_digest(&self) -> &RolloutDigest {
        &self.scope_digest
    }

    pub fn previous_digest(&self) -> Option<&RolloutDigest> {
        self.previous_digest.as_ref()
    }

    pub const fn kind(&self) -> RolloutRecordKind {
        self.evidence.kind()
    }

    pub fn evidence(&self) -> &RolloutEvidence {
        &self.evidence
    }

    pub fn digest(&self) -> &RolloutDigest {
        &self.digest
    }

    fn verify(&self) -> Result<(), RolloutContractError> {
        self.evidence.validate()?;
        let expected = digest_record(
            self.sequence,
            &self.scope_digest,
            self.previous_digest.as_ref(),
            &self.evidence,
        );
        if expected != self.digest {
            return Err(RolloutContractError::RecordDigestMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rollout {
    projection_version: u16,
    scope: RolloutScope,
    records: Vec<RolloutRecord>,
    final_digest: RolloutDigest,
}

impl Rollout {
    pub fn new(
        scope: RolloutScope,
        records: Vec<RolloutRecord>,
    ) -> Result<Self, RolloutContractError> {
        let final_digest = records
            .last()
            .map(|record| record.digest.clone())
            .ok_or(RolloutContractError::EmptyRecords)?;
        Self::verify_records(&scope, &records, &final_digest)?;
        Ok(Self {
            projection_version: ROLLOUT_PROJECTION_VERSION,
            scope,
            records,
            final_digest,
        })
    }

    pub const fn projection_version(&self) -> u16 {
        self.projection_version
    }

    pub const fn scope(&self) -> &RolloutScope {
        &self.scope
    }

    pub fn records(&self) -> &[RolloutRecord] {
        &self.records
    }

    pub fn final_digest(&self) -> &RolloutDigest {
        &self.final_digest
    }

    pub fn verify(&self) -> Result<(), RolloutContractError> {
        if self.projection_version != ROLLOUT_PROJECTION_VERSION {
            return Err(RolloutContractError::RecordDigestMismatch);
        }
        Self::verify_records(&self.scope, &self.records, &self.final_digest)
    }

    pub fn verify_records(
        scope: &RolloutScope,
        records: &[RolloutRecord],
        final_digest: &RolloutDigest,
    ) -> Result<(), RolloutContractError> {
        if records.is_empty() {
            return Err(RolloutContractError::EmptyRecords);
        }
        if records.len() > MAX_ROLLOUT_RECORDS {
            return Err(RolloutContractError::TooManyRecords);
        }
        let scope_digest = scope.digest();
        let mut previous: Option<&RolloutDigest> = None;
        for (index, record) in records.iter().enumerate() {
            if record.sequence
                != u32::try_from(index).map_err(|_| RolloutContractError::TooManyRecords)?
            {
                return Err(RolloutContractError::InvalidSequence);
            }
            if record.scope_digest != scope_digest {
                return Err(RolloutContractError::ScopeMismatch);
            }
            if record.previous_digest.as_ref() != previous {
                return Err(RolloutContractError::PreviousDigestMismatch);
            }
            record.verify()?;
            previous = Some(&record.digest);
        }
        if previous != Some(final_digest) {
            return Err(RolloutContractError::FinalDigestMismatch);
        }
        Ok(())
    }
}

fn digest_record(
    sequence: u32,
    scope_digest: &RolloutDigest,
    previous_digest: Option<&RolloutDigest>,
    evidence: &RolloutEvidence,
) -> RolloutDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-rollout-record");
    hasher.update(ROLLOUT_PROJECTION_VERSION.to_be_bytes());
    hasher.update(sequence.to_be_bytes());
    digest_text(&mut hasher, scope_digest.as_str());
    digest_optional_text(&mut hasher, previous_digest.map(RolloutDigest::as_str));
    evidence.digest_into(&mut hasher);
    finish_digest(hasher)
}

fn digest_sensitive(domain: &str, value: &str) -> Result<RolloutDigest, RolloutContractError> {
    if value.trim().is_empty() {
        return Err(RolloutContractError::EmptyField("redacted reason"));
    }
    if value.len() > MAX_REASON_BYTES {
        return Err(RolloutContractError::FieldTooLong("redacted reason"));
    }
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    digest_text(&mut hasher, value);
    Ok(finish_digest(hasher))
}

fn finish_digest(hasher: Sha256) -> RolloutDigest {
    RolloutDigest(format!("sha256:{:x}", hasher.finalize()))
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

fn digest_optional_u32(hasher: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), RolloutContractError> {
    if value.trim().is_empty() {
        return Err(RolloutContractError::EmptyField(field));
    }
    if value.len() > MAX_LABEL_BYTES {
        return Err(RolloutContractError::FieldTooLong(field));
    }
    Ok(())
}

fn validate_optional_identifier(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), RolloutContractError> {
    value.map_or(Ok(()), |value| validate_identifier(field, value))
}

fn validate_label(field: &'static str, value: String) -> Result<String, RolloutContractError> {
    validate_identifier(field, &value)?;
    if value.chars().any(char::is_control) {
        return Err(RolloutContractError::InvalidCode(field));
    }
    Ok(value)
}

fn validate_code(field: &'static str, value: String) -> Result<String, RolloutContractError> {
    validate_identifier(field, &value)?;
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
    }) {
        return Err(RolloutContractError::InvalidCode(field));
    }
    Ok(value)
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

const fn event_type_code(event_type: EventType) -> &'static str {
    match event_type {
        EventType::SessionStarted => "session_started",
        EventType::EffectRequested => "effect_requested",
        EventType::EffectCompleted => "effect_completed",
        EventType::PolicyApproved => "policy_approved",
        EventType::PolicyDenied => "policy_denied",
        EventType::ApprovalRequired => "approval_required",
        EventType::ExecutionFailed => "execution_failed",
        EventType::ProviderCall => "provider_call",
        EventType::McpEraSelected => "mcp_era_selected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventId, EventType, RequestDigest};

    fn scope() -> RolloutScope {
        RolloutScope::new("tenant-a", "workspace-a", "session-a", "execution-a").unwrap()
    }

    #[test]
    fn record_hash_is_stable_for_the_same_redacted_evidence() {
        let evidence = RolloutEvidence::RuntimeEvent {
            event_id: EventId::new("event-a").unwrap(),
            event_type: EventType::EffectRequested,
            harness_id: None,
            gene_id: None,
            policy_version: Some(3),
            receipt_id: None,
            payload: RolloutEventEvidence::effect(
                "filesystem_read",
                RequestDigest::new("pandora-request-v1:sha256:abc").unwrap(),
            )
            .unwrap(),
        };

        let first = RolloutRecord::link(&scope(), 0, None, evidence.clone()).unwrap();
        let second = RolloutRecord::link(&scope(), 0, None, evidence).unwrap();

        assert_eq!(first.digest(), second.digest());
    }

    #[test]
    fn policy_reason_is_represented_only_by_digest() {
        let reason = "operator denied access to a private credential";
        let evidence = RolloutEventEvidence::policy(reason).unwrap();

        assert!(!format!("{evidence:?}").contains(reason));
        let RolloutEventEvidence::Policy { reason_digest } = evidence else {
            panic!("policy evidence must remain typed");
        };
        assert!(reason_digest.as_str().starts_with("sha256:"));
    }

    #[test]
    fn record_validation_rejects_empty_scope_or_digest() {
        assert!(matches!(
            RolloutScope::new("", "workspace-a", "session-a", "execution-a"),
            Err(RolloutContractError::InvalidId(_))
        ));
        assert!(matches!(
            RolloutDigest::new(""),
            Err(RolloutContractError::InvalidDigest)
        ));
    }
}
