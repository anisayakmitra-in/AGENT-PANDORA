use crate::capability::Capability;
use crate::effect::{RequestError, Timestamp};
use crate::ids::{ExecutionId, GeneId, HarnessId, RequestDigest, SessionId};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AdaptationTarget {
    Harness(HarnessId),
    Gene(GeneId),
    Skill(String),
    Provider(String),
    Workflow(String),
    Recovery(String),
    CapabilityReduction(Capability),
}

impl AdaptationTarget {
    pub fn skill(value: impl Into<String>) -> Result<Self, AdaptationContractError> {
        Ok(Self::Skill(validate_label("skill", value.into())?))
    }

    pub fn provider(value: impl Into<String>) -> Result<Self, AdaptationContractError> {
        Ok(Self::Provider(validate_label("provider", value.into())?))
    }

    pub fn workflow(value: impl Into<String>) -> Result<Self, AdaptationContractError> {
        Ok(Self::Workflow(validate_label("workflow", value.into())?))
    }

    pub fn recovery(value: impl Into<String>) -> Result<Self, AdaptationContractError> {
        Ok(Self::Recovery(validate_label("recovery", value.into())?))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Harness(id) => id.as_str(),
            Self::Gene(id) => id.as_str(),
            Self::Skill(value)
            | Self::Provider(value)
            | Self::Workflow(value)
            | Self::Recovery(value) => value,
            Self::CapabilityReduction(capability) => capability.as_str(),
        }
    }

    fn validate(&self) -> Result<(), AdaptationContractError> {
        match self {
            Self::Harness(id) if id.as_str().trim().is_empty() => {
                Err(AdaptationContractError::InvalidTarget)
            }
            Self::Gene(id) if id.as_str().trim().is_empty() => {
                Err(AdaptationContractError::InvalidTarget)
            }
            Self::Skill(value)
            | Self::Provider(value)
            | Self::Workflow(value)
            | Self::Recovery(value)
                if value.trim().is_empty()
                    || value.len() > 256
                    || value.chars().any(char::is_control) =>
            {
                Err(AdaptationContractError::InvalidTarget)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdaptationContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    InvalidTarget,
    InvalidLimit(&'static str),
    DuplicateCandidate(String),
    AuthorityExpansion,
    Request(RequestError),
}

impl fmt::Display for AdaptationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::InvalidTarget => formatter.write_str("adaptation target is invalid"),
            Self::InvalidLimit(field) => write!(formatter, "{field} must be greater than zero"),
            Self::DuplicateCandidate(id) => {
                write!(formatter, "adaptation candidate {id} is duplicated")
            }
            Self::AuthorityExpansion => formatter.write_str("adaptation cannot expand authority"),
            Self::Request(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AdaptationContractError {}

impl From<RequestError> for AdaptationContractError {
    fn from(error: RequestError) -> Self {
        Self::Request(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptationCandidate {
    id: String,
    target: AdaptationTarget,
    score: i32,
    approved: bool,
    cost_micros: u64,
    latency_ms: u64,
}

impl AdaptationCandidate {
    pub fn new(
        id: impl Into<String>,
        target: AdaptationTarget,
        score: i32,
        approved: bool,
        expands_authority: bool,
        cost_micros: u64,
        latency_ms: u64,
    ) -> Result<Self, AdaptationContractError> {
        let id = validate_label("candidate ID", id.into())?;
        if expands_authority {
            return Err(AdaptationContractError::AuthorityExpansion);
        }
        target.validate()?;
        Ok(Self {
            id,
            target,
            score,
            approved,
            cost_micros,
            latency_ms,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn target(&self) -> &AdaptationTarget {
        &self.target
    }

    pub const fn score(&self) -> i32 {
        self.score
    }

    pub const fn approved(&self) -> bool {
        self.approved
    }

    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }

    pub const fn latency_ms(&self) -> u64 {
        self.latency_ms
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptationPolicy {
    policy_version: u32,
    max_candidates: usize,
    max_cost_micros: u64,
    max_latency_ms: u64,
}

impl AdaptationPolicy {
    pub fn new(
        policy_version: u32,
        max_candidates: usize,
        max_cost_micros: u64,
        max_latency_ms: u64,
    ) -> Result<Self, AdaptationContractError> {
        if max_candidates == 0 {
            return Err(AdaptationContractError::InvalidLimit("max candidates"));
        }
        Ok(Self {
            policy_version,
            max_candidates,
            max_cost_micros,
            max_latency_ms,
        })
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn max_candidates(&self) -> usize {
        self.max_candidates
    }

    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }

    pub const fn max_latency_ms(&self) -> u64 {
        self.max_latency_ms
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptationRequest {
    execution_id: ExecutionId,
    session_id: SessionId,
    request_digest: RequestDigest,
    current: Option<AdaptationTarget>,
    candidates: Vec<AdaptationCandidate>,
}

impl AdaptationRequest {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        request_digest: RequestDigest,
        current: Option<AdaptationTarget>,
        candidates: Vec<AdaptationCandidate>,
    ) -> Result<Self, AdaptationContractError> {
        if let Some(current) = &current {
            current.validate()?;
        }
        let mut ids = BTreeSet::new();
        for candidate in &candidates {
            if !ids.insert(candidate.id()) {
                return Err(AdaptationContractError::DuplicateCandidate(
                    candidate.id().to_owned(),
                ));
            }
        }
        Ok(Self {
            execution_id,
            session_id,
            request_digest,
            current,
            candidates,
        })
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn current(&self) -> Option<&AdaptationTarget> {
        self.current.as_ref()
    }

    pub fn candidates(&self) -> &[AdaptationCandidate] {
        &self.candidates
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdaptationDecision {
    Selected {
        target: AdaptationTarget,
        changed: bool,
        reason: String,
    },
    NoChange {
        degraded: bool,
        reason: String,
    },
}

impl AdaptationDecision {
    pub fn selected(&self) -> Option<&AdaptationTarget> {
        match self {
            Self::Selected { target, .. } => Some(target),
            Self::NoChange { .. } => None,
        }
    }

    pub const fn changed(&self) -> bool {
        matches!(self, Self::Selected { changed: true, .. })
    }

    pub const fn degraded(&self) -> bool {
        matches!(self, Self::NoChange { degraded: true, .. })
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Selected { reason, .. } | Self::NoChange { reason, .. } => reason,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptationReceipt {
    execution_id: ExecutionId,
    session_id: SessionId,
    request_digest: RequestDigest,
    policy_version: u32,
    selected: Option<AdaptationTarget>,
    changed: bool,
    degraded: bool,
    reason: String,
    recorded_at: Timestamp,
}

impl AdaptationReceipt {
    pub fn new(
        request: &AdaptationRequest,
        policy_version: u32,
        decision: &AdaptationDecision,
        recorded_at: Timestamp,
    ) -> Self {
        Self {
            execution_id: request.execution_id.clone(),
            session_id: request.session_id.clone(),
            request_digest: request.request_digest.clone(),
            policy_version,
            selected: decision.selected().cloned(),
            changed: decision.changed(),
            degraded: decision.degraded(),
            reason: decision.reason().to_owned(),
            recorded_at,
        }
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn selected(&self) -> Option<&AdaptationTarget> {
        self.selected.as_ref()
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn degraded(&self) -> bool {
        self.degraded
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub const fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }
}

fn validate_label(field: &'static str, value: String) -> Result<String, AdaptationContractError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(AdaptationContractError::EmptyField(field));
    }
    if value.len() > 256 {
        return Err(AdaptationContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(AdaptationContractError::ControlCharacter(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, ExecutionId, GeneId, HarnessId, RequestDigest, SessionId};

    fn candidate(
        id: &str,
        target: AdaptationTarget,
        score: i32,
        approved: bool,
    ) -> AdaptationCandidate {
        AdaptationCandidate::new(id, target, score, approved, false, 100, 10).unwrap()
    }

    #[test]
    fn candidate_targets_cover_harness_gene_and_safe_reduction() {
        assert_eq!(
            candidate(
                "coding",
                AdaptationTarget::Harness(HarnessId::new("coding-domain").unwrap()),
                10,
                true
            )
            .target()
            .label(),
            "coding-domain"
        );
        assert_eq!(
            candidate(
                "read",
                AdaptationTarget::Gene(GeneId::new("workspace.read").unwrap()),
                10,
                true
            )
            .target()
            .label(),
            "workspace.read"
        );
        assert_eq!(
            candidate(
                "reduce",
                AdaptationTarget::CapabilityReduction(Capability::FilesystemWrite),
                10,
                true
            )
            .target()
            .label(),
            "filesystem.write"
        );
    }

    #[test]
    fn requests_reject_duplicate_candidate_ids() {
        let result = AdaptationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            RequestDigest::new("pandora-request-v1:sha256:test").unwrap(),
            None,
            vec![
                candidate(
                    "same",
                    AdaptationTarget::Workflow("one".to_owned()),
                    1,
                    true,
                ),
                candidate(
                    "same",
                    AdaptationTarget::Workflow("two".to_owned()),
                    2,
                    true,
                ),
            ],
        );

        assert!(matches!(
            result,
            Err(AdaptationContractError::DuplicateCandidate(_))
        ));
    }

    #[test]
    fn authority_expansion_is_rejected_at_candidate_construction() {
        assert_eq!(
            AdaptationCandidate::new(
                "unsafe",
                AdaptationTarget::Recovery("grant-all".to_owned()),
                100,
                true,
                true,
                1,
                1,
            ),
            Err(AdaptationContractError::AuthorityExpansion)
        );
    }
}
