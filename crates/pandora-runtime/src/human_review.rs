use pandora_types::{OperationRequest, RequestDigest, Timestamp};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HumanMode {
    HumanInTheLoop,
    HumanOnTheLoop,
    HumanOutOfTheLoop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewState {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewDecision {
    Approve,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewError {
    InvalidSubject,
    InvalidTimeout,
    NotFound,
    AlreadyResolved,
    Expired,
    InvalidActor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSubject {
    request_digest: RequestDigest,
    summary: String,
}

impl ReviewSubject {
    pub fn new(request: &OperationRequest, summary: impl Into<String>) -> Self {
        Self {
            request_digest: request.request_digest().clone(),
            summary: summary.into().trim().to_owned(),
        }
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRecord {
    id: String,
    mode: HumanMode,
    subject: ReviewSubject,
    confidence: u8,
    irreversible: bool,
    state: ReviewState,
    created_at: Timestamp,
    expires_at: Timestamp,
}

impl ReviewRecord {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn mode(&self) -> HumanMode {
        self.mode
    }

    pub fn subject(&self) -> &ReviewSubject {
        &self.subject
    }

    pub const fn confidence(&self) -> u8 {
        self.confidence
    }

    pub const fn irreversible(&self) -> bool {
        self.irreversible
    }

    pub const fn state(&self) -> ReviewState {
        self.state
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewReceipt {
    review_id: String,
    subject_digest: RequestDigest,
    state: ReviewState,
    actor: Option<String>,
    at: Timestamp,
}

impl ReviewReceipt {
    pub fn review_id(&self) -> &str {
        &self.review_id
    }

    pub fn subject_digest(&self) -> &RequestDigest {
        &self.subject_digest
    }

    pub const fn state(&self) -> ReviewState {
        self.state
    }

    pub fn actor(&self) -> Option<&str> {
        self.actor.as_deref()
    }

    pub const fn at(&self) -> Timestamp {
        self.at
    }
}

pub struct HumanReviewEngine {
    confidence_threshold: u8,
    next_id: AtomicU64,
    records: Mutex<HashMap<String, ReviewRecord>>,
    audit: Mutex<Vec<ReviewReceipt>>,
}

impl HumanReviewEngine {
    pub fn new(confidence_threshold: u8) -> Self {
        Self {
            confidence_threshold,
            next_id: AtomicU64::new(1),
            records: Mutex::new(HashMap::new()),
            audit: Mutex::new(Vec::new()),
        }
    }

    pub fn submit(
        &self,
        mode: HumanMode,
        subject: ReviewSubject,
        confidence: u8,
        irreversible: bool,
        created_at: Timestamp,
        timeout_seconds: u64,
    ) -> Result<ReviewRecord, ReviewError> {
        if subject.summary().is_empty() || subject.summary().len() > 4096 {
            return Err(ReviewError::InvalidSubject);
        }
        if timeout_seconds == 0 {
            return Err(ReviewError::InvalidTimeout);
        }
        let expires_at = created_at
            .as_unix_seconds()
            .checked_add(timeout_seconds)
            .map(Timestamp::from_unix_seconds)
            .ok_or(ReviewError::InvalidTimeout)?;
        let state = if irreversible {
            ReviewState::Pending
        } else {
            match mode {
                HumanMode::HumanInTheLoop => ReviewState::Pending,
                HumanMode::HumanOnTheLoop if confidence < self.confidence_threshold => {
                    ReviewState::Pending
                }
                HumanMode::HumanOnTheLoop | HumanMode::HumanOutOfTheLoop => ReviewState::Approved,
            }
        };
        let id = self
            .next_id
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ReviewError::InvalidTimeout)?;
        let record = ReviewRecord {
            id: format!("review-{id}"),
            mode,
            subject,
            confidence,
            irreversible,
            state,
            created_at,
            expires_at,
        };
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(record.id.clone(), record.clone());
        if state == ReviewState::Approved {
            self.audit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(receipt(&record, state, None, created_at));
        }
        Ok(record)
    }

    pub fn resolve(
        &self,
        id: &str,
        decision: ReviewDecision,
        actor: &str,
        at: Timestamp,
    ) -> Result<ReviewReceipt, ReviewError> {
        let actor = validate_actor(actor)?;
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = records.get_mut(id).ok_or(ReviewError::NotFound)?;
        if record.state != ReviewState::Pending {
            return Err(ReviewError::AlreadyResolved);
        }
        if at >= record.expires_at {
            record.state = ReviewState::Expired;
            let receipt = receipt(record, ReviewState::Expired, None, at);
            self.audit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(receipt.clone());
            return Err(ReviewError::Expired);
        }
        record.state = match decision {
            ReviewDecision::Approve => ReviewState::Approved,
            ReviewDecision::Reject => ReviewState::Rejected,
        };
        let receipt = receipt(record, record.state, Some(actor), at);
        self.audit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(receipt.clone());
        Ok(receipt)
    }

    pub fn status(&self, id: &str, at: Timestamp) -> Result<ReviewState, ReviewError> {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = records.get_mut(id).ok_or(ReviewError::NotFound)?;
        if record.state == ReviewState::Pending && at >= record.expires_at {
            record.state = ReviewState::Expired;
            let receipt = receipt(record, ReviewState::Expired, None, at);
            self.audit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(receipt);
        }
        Ok(record.state)
    }

    pub fn can_resume(&self, id: &str, at: Timestamp) -> bool {
        self.status(id, at)
            .is_ok_and(|state| state == ReviewState::Approved)
    }

    pub fn audit(&self, id: &str) -> Vec<ReviewReceipt> {
        self.audit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|receipt| receipt.review_id() == id)
            .cloned()
            .collect()
    }
}

impl Default for HumanReviewEngine {
    fn default() -> Self {
        Self::new(70)
    }
}

fn receipt(
    record: &ReviewRecord,
    state: ReviewState,
    actor: Option<String>,
    at: Timestamp,
) -> ReviewReceipt {
    ReviewReceipt {
        review_id: record.id.clone(),
        subject_digest: record.subject.request_digest.clone(),
        state,
        actor,
        at,
    }
}

fn validate_actor(actor: &str) -> Result<String, ReviewError> {
    let actor = actor.trim();
    if actor.is_empty() || actor.chars().any(char::is_control) {
        return Err(ReviewError::InvalidActor);
    }
    Ok(actor.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        Capability, EffectTarget, ExecutionId, GeneId, Operation, PrincipalId, ResourceScope,
        SessionId,
    };

    fn subject() -> ReviewSubject {
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("patch.apply").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        ReviewSubject::new(&request, "write src/lib.rs")
    }

    #[test]
    fn irreversible_human_in_the_loop_action_blocks_until_approval() {
        let engine = HumanReviewEngine::new(70);
        let review = engine
            .submit(
                HumanMode::HumanInTheLoop,
                subject(),
                95,
                true,
                Timestamp::from_unix_seconds(1),
                30,
            )
            .unwrap();
        assert_eq!(review.state(), ReviewState::Pending);
        assert!(!engine.can_resume(review.id(), Timestamp::from_unix_seconds(2)));

        let receipt = engine
            .resolve(
                review.id(),
                ReviewDecision::Approve,
                "operator-1",
                Timestamp::from_unix_seconds(2),
            )
            .unwrap();
        assert_eq!(receipt.state(), ReviewState::Approved);
        assert!(engine.can_resume(review.id(), Timestamp::from_unix_seconds(2)));
        assert_eq!(receipt.subject_digest(), review.subject().request_digest());
    }

    #[test]
    fn low_confidence_human_on_the_loop_review_escalates() {
        let review = HumanReviewEngine::new(70)
            .submit(
                HumanMode::HumanOnTheLoop,
                subject(),
                69,
                false,
                Timestamp::from_unix_seconds(1),
                30,
            )
            .unwrap();
        assert_eq!(review.state(), ReviewState::Pending);
    }

    #[test]
    fn review_can_be_rejected_asynchronously() {
        let engine = HumanReviewEngine::new(70);
        let review = engine
            .submit(
                HumanMode::HumanInTheLoop,
                subject(),
                90,
                false,
                Timestamp::from_unix_seconds(1),
                30,
            )
            .unwrap();
        let receipt = engine
            .resolve(
                review.id(),
                ReviewDecision::Reject,
                "operator-1",
                Timestamp::from_unix_seconds(2),
            )
            .unwrap();
        assert_eq!(receipt.state(), ReviewState::Rejected);
        assert!(!engine.can_resume(review.id(), Timestamp::from_unix_seconds(2)));
    }

    #[test]
    fn timeout_expires_pending_review_and_is_auditable() {
        let engine = HumanReviewEngine::new(70);
        let review = engine
            .submit(
                HumanMode::HumanInTheLoop,
                subject(),
                90,
                false,
                Timestamp::from_unix_seconds(1),
                1,
            )
            .unwrap();
        assert_eq!(
            engine.status(review.id(), Timestamp::from_unix_seconds(2)),
            Ok(ReviewState::Expired)
        );
        assert!(
            engine
                .audit(review.id())
                .iter()
                .any(|receipt| { receipt.state() == ReviewState::Expired })
        );
    }
}
