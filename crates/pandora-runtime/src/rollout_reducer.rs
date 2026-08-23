use pandora_types::{
    EffectReceipt, EventContext, EventPayload, EventType, MAX_ROLLOUT_RECORDS, Rollout,
    RolloutContractError, RolloutDigest, RolloutEffectOutcome, RolloutEventEvidence,
    RolloutEvidence, RolloutRecord, RolloutScope, RuntimeEvent,
};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RolloutReducerError {
    Contract(RolloutContractError),
    EmptyEvents,
    MissingSessionScope,
    MissingExecutionScope,
    ScopeMismatch,
    DuplicateEventId,
    DuplicateReceiptId,
    AmbiguousReceiptLink,
    ReceiptNotLinked,
    ReceiptMissing,
    ReceiptDigestMismatch,
}

impl fmt::Display for RolloutReducerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::EmptyEvents => formatter.write_str("rollout reduction requires an event"),
            Self::MissingSessionScope => {
                formatter.write_str("runtime event is missing its session scope")
            }
            Self::MissingExecutionScope => {
                formatter.write_str("runtime event is missing its execution scope")
            }
            Self::ScopeMismatch => formatter.write_str("runtime event scope does not match"),
            Self::DuplicateEventId => formatter.write_str("runtime event ID is duplicated"),
            Self::DuplicateReceiptId => formatter.write_str("effect receipt ID is duplicated"),
            Self::AmbiguousReceiptLink => {
                formatter.write_str("effect receipt has more than one completion event")
            }
            Self::ReceiptNotLinked => {
                formatter.write_str("effect receipt has no matching completion event")
            }
            Self::ReceiptMissing => {
                formatter.write_str("effect completion event has no matching receipt")
            }
            Self::ReceiptDigestMismatch => {
                formatter.write_str("effect receipt request digest does not match its event")
            }
        }
    }
}

impl std::error::Error for RolloutReducerError {}

impl From<RolloutContractError> for RolloutReducerError {
    fn from(error: RolloutContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RolloutReducer;

impl RolloutReducer {
    pub const fn new() -> Self {
        Self
    }

    pub fn reduce(
        &self,
        context_manifest_digest: &str,
        events: &[RuntimeEvent],
        receipts: &[EffectReceipt],
    ) -> Result<Rollout, RolloutReducerError> {
        let first = events.first().ok_or(RolloutReducerError::EmptyEvents)?;
        let scope = scope_from_context(first.context())?;
        let record_count = required_record_count(events.len(), receipts.len())?;
        let mut records = Vec::with_capacity(record_count);
        push_record(
            &scope,
            &mut records,
            RolloutEvidence::ContextManifest {
                manifest_digest: RolloutDigest::new(context_manifest_digest)?,
            },
        )?;

        let mut event_ids = HashSet::new();
        let mut receipt_links = HashMap::new();
        for event in events {
            validate_scope(&scope, event.context())?;
            if !event_ids.insert(event.event_id().as_str()) {
                return Err(RolloutReducerError::DuplicateEventId);
            }
            if event.event_type() == EventType::EffectCompleted
                && let (Some(receipt_id), EventPayload::Effect { request_digest, .. }) =
                    (event.context().receipt_id(), event.payload())
                && receipt_links
                    .insert(receipt_id.as_str(), request_digest)
                    .is_some()
            {
                return Err(RolloutReducerError::AmbiguousReceiptLink);
            }
            push_record(&scope, &mut records, project_event(event)?)?;
        }

        let mut receipt_ids = HashSet::new();
        for receipt in receipts {
            if !receipt_ids.insert(receipt.receipt_id().as_str()) {
                return Err(RolloutReducerError::DuplicateReceiptId);
            }
            let linked_digest = receipt_links
                .remove(receipt.receipt_id().as_str())
                .ok_or(RolloutReducerError::ReceiptNotLinked)?;
            if linked_digest.as_str() != receipt.request_digest().as_str() {
                return Err(RolloutReducerError::ReceiptDigestMismatch);
            }
            push_record(
                &scope,
                &mut records,
                RolloutEvidence::EffectReceipt {
                    receipt_id: receipt.receipt_id().clone(),
                    permit_id: receipt.permit_id().clone(),
                    request_digest: receipt.request_digest().clone(),
                    completed_at: receipt.completed_at(),
                    outcome: RolloutEffectOutcome::from_effect_outcome(receipt.outcome())?,
                },
            )?;
        }
        if !receipt_links.is_empty() {
            return Err(RolloutReducerError::ReceiptMissing);
        }

        Ok(Rollout::new(scope, records)?)
    }

    pub fn verify(
        scope: &RolloutScope,
        records: &[RolloutRecord],
        final_digest: &RolloutDigest,
    ) -> Result<(), RolloutReducerError> {
        Rollout::verify_records(scope, records, final_digest).map_err(Into::into)
    }
}

fn required_record_count(
    event_count: usize,
    receipt_count: usize,
) -> Result<usize, RolloutReducerError> {
    let record_count = event_count
        .checked_add(receipt_count)
        .and_then(|count| count.checked_add(1))
        .ok_or(RolloutContractError::TooManyRecords)?;
    if record_count > MAX_ROLLOUT_RECORDS {
        return Err(RolloutContractError::TooManyRecords.into());
    }
    Ok(record_count)
}

fn scope_from_context(context: &EventContext) -> Result<RolloutScope, RolloutReducerError> {
    let session_id = context
        .session_id()
        .ok_or(RolloutReducerError::MissingSessionScope)?;
    let execution_id = context
        .execution_id()
        .ok_or(RolloutReducerError::MissingExecutionScope)?;
    Ok(RolloutScope::new(
        context.tenant_id().as_str(),
        context.workspace_id().as_str(),
        session_id.as_str(),
        execution_id.as_str(),
    )?)
}

fn validate_scope(scope: &RolloutScope, context: &EventContext) -> Result<(), RolloutReducerError> {
    let session_id = context
        .session_id()
        .ok_or(RolloutReducerError::MissingSessionScope)?;
    let execution_id = context
        .execution_id()
        .ok_or(RolloutReducerError::MissingExecutionScope)?;
    if context.tenant_id().as_str() != scope.tenant_id().as_str()
        || context.workspace_id().as_str() != scope.workspace_id().as_str()
        || session_id.as_str() != scope.session_id().as_str()
        || execution_id.as_str() != scope.execution_id().as_str()
    {
        return Err(RolloutReducerError::ScopeMismatch);
    }
    Ok(())
}

fn project_event(event: &RuntimeEvent) -> Result<RolloutEvidence, RolloutReducerError> {
    let payload = match event.payload() {
        EventPayload::Empty => RolloutEventEvidence::Empty,
        EventPayload::Effect {
            capability,
            request_digest,
        } => RolloutEventEvidence::effect(capability, request_digest.clone())?,
        EventPayload::Policy { reason } => RolloutEventEvidence::policy(reason)?,
        EventPayload::Failure { code } => RolloutEventEvidence::failure(code)?,
        EventPayload::ProviderCall {
            provider,
            credential: _,
            request_digest,
        } => RolloutEventEvidence::provider_call(provider, request_digest.clone())?,
        EventPayload::McpEra {
            server,
            era,
            downgraded,
        } => RolloutEventEvidence::mcp_era(server, era, *downgraded)?,
    };
    Ok(RolloutEvidence::RuntimeEvent {
        event_id: event.event_id().clone(),
        event_type: event.event_type(),
        harness_id: event.context().harness_id().cloned(),
        gene_id: event.context().gene_id().cloned(),
        policy_version: event.context().policy_version(),
        receipt_id: event.context().receipt_id().cloned(),
        payload,
    })
}

fn push_record(
    scope: &RolloutScope,
    records: &mut Vec<RolloutRecord>,
    evidence: RolloutEvidence,
) -> Result<(), RolloutReducerError> {
    let sequence =
        u32::try_from(records.len()).map_err(|_| RolloutContractError::TooManyRecords)?;
    let previous_digest = records.last().map(|record| record.digest().clone());
    records.push(RolloutRecord::link(
        scope,
        sequence,
        previous_digest,
        evidence,
    )?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        EffectOutcome, EffectReceipt, EventContext, EventId, EventPayload, EventType, ExecutionId,
        GeneId, HarnessId, PermitId, ReceiptId, RequestDigest, RuntimeEvent, SecretReference,
        SessionId, TenantId, Timestamp, WorkspaceId,
    };
    use serde::Deserialize;

    const CONTEXT_DIGEST: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn context(
        tenant: &str,
        workspace: &str,
        session: &str,
        execution: &str,
        receipt_id: Option<&str>,
    ) -> EventContext {
        let context = EventContext::new(
            TenantId::new(tenant).unwrap(),
            WorkspaceId::new(workspace).unwrap(),
        )
        .with_session(SessionId::new(session).unwrap())
        .with_execution(ExecutionId::new(execution).unwrap())
        .with_harness(HarnessId::new("coding-domain").unwrap())
        .with_gene(GeneId::new("daedalus-read").unwrap())
        .with_policy_version(7);
        match receipt_id {
            Some(receipt_id) => context.with_receipt(ReceiptId::new(receipt_id).unwrap()),
            None => context,
        }
    }

    fn evidence() -> (Vec<RuntimeEvent>, Vec<EffectReceipt>) {
        let request_digest = RequestDigest::new(
            "pandora-request-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        let events = vec![
            RuntimeEvent::new(
                EventId::new("event-requested").unwrap(),
                EventType::EffectRequested,
                context("tenant-a", "workspace-a", "session-a", "execution-a", None),
                EventPayload::Effect {
                    capability: "filesystem_read".to_owned(),
                    request_digest: request_digest.clone(),
                },
            ),
            RuntimeEvent::new(
                EventId::new("event-completed").unwrap(),
                EventType::EffectCompleted,
                context(
                    "tenant-a",
                    "workspace-a",
                    "session-a",
                    "execution-a",
                    Some("receipt-a"),
                ),
                EventPayload::Effect {
                    capability: "filesystem_read".to_owned(),
                    request_digest: request_digest.clone(),
                },
            ),
        ];
        let receipts = vec![EffectReceipt::new(
            ReceiptId::new("receipt-a").unwrap(),
            PermitId::new("permit-a").unwrap(),
            request_digest,
            Timestamp::from_unix_seconds(42),
            EffectOutcome::Succeeded,
        )];
        (events, receipts)
    }

    #[test]
    fn same_evidence_produces_the_same_rollout_hash() {
        let (events, receipts) = evidence();

        let first = RolloutReducer::new()
            .reduce(CONTEXT_DIGEST, &events, &receipts)
            .unwrap();
        let second = RolloutReducer::new()
            .reduce(CONTEXT_DIGEST, &events, &receipts)
            .unwrap();

        assert_eq!(first.final_digest(), second.final_digest());
    }

    #[test]
    fn rollout_contains_context_permit_and_receipt_linkage() {
        let (events, receipts) = evidence();
        let rollout = RolloutReducer::new()
            .reduce(CONTEXT_DIGEST, &events, &receipts)
            .unwrap();

        assert!(matches!(
            rollout.records()[0].evidence(),
            pandora_types::RolloutEvidence::ContextManifest { manifest_digest }
                if manifest_digest.as_str() == CONTEXT_DIGEST
        ));
        assert!(rollout.records().iter().any(|record| matches!(
            record.evidence(),
            pandora_types::RolloutEvidence::EffectReceipt {
                receipt_id,
                permit_id,
                ..
            } if receipt_id.as_str() == "receipt-a" && permit_id.as_str() == "permit-a"
        )));
    }

    #[test]
    fn reducer_rejects_cross_scope_events() {
        let (mut events, receipts) = evidence();
        events.push(RuntimeEvent::new(
            EventId::new("event-cross-scope").unwrap(),
            EventType::ExecutionFailed,
            context("tenant-a", "workspace-b", "session-a", "execution-a", None),
            EventPayload::Failure {
                code: "failed".to_owned(),
            },
        ));

        assert!(matches!(
            RolloutReducer::new().reduce(CONTEXT_DIGEST, &events, &receipts),
            Err(RolloutReducerError::ScopeMismatch)
        ));
    }

    #[test]
    fn reducer_omits_credentials_and_policy_text() {
        let policy_reason = "private policy explanation";
        let credential = "PANDORA_PRIVATE_PROVIDER_KEY";
        let request_digest = RequestDigest::new("provider-request").unwrap();
        let events = vec![
            RuntimeEvent::new(
                EventId::new("event-provider").unwrap(),
                EventType::ProviderCall,
                context("tenant-a", "workspace-a", "session-a", "execution-a", None),
                EventPayload::ProviderCall {
                    provider: "provider-a".to_owned(),
                    credential: SecretReference::new(credential).unwrap(),
                    request_digest,
                },
            ),
            RuntimeEvent::new(
                EventId::new("event-policy").unwrap(),
                EventType::PolicyDenied,
                context("tenant-a", "workspace-a", "session-a", "execution-a", None),
                EventPayload::Policy {
                    reason: policy_reason.to_owned(),
                },
            ),
        ];

        let rollout = RolloutReducer::new()
            .reduce(CONTEXT_DIGEST, &events, &[])
            .unwrap();
        let debug = format!("{rollout:?}");

        assert!(!debug.contains(credential));
        assert!(!debug.contains(policy_reason));
    }

    #[test]
    fn reducer_rejects_completion_without_its_receipt() {
        let (events, _) = evidence();

        assert!(matches!(
            RolloutReducer::new().reduce(CONTEXT_DIGEST, &events, &[]),
            Err(RolloutReducerError::ReceiptMissing)
        ));
    }

    #[test]
    fn record_count_rejects_overflow_and_inputs_above_the_projection_limit() {
        assert!(matches!(
            required_record_count(usize::MAX, 1),
            Err(RolloutReducerError::Contract(
                RolloutContractError::TooManyRecords
            ))
        ));
        assert!(matches!(
            required_record_count(pandora_types::MAX_ROLLOUT_RECORDS, 0),
            Err(RolloutReducerError::Contract(
                RolloutContractError::TooManyRecords
            ))
        ));
    }

    #[test]
    fn replay_rejects_reordered_removed_duplicated_or_tampered_records() {
        let (events, receipts) = evidence();
        let rollout = RolloutReducer::new()
            .reduce(CONTEXT_DIGEST, &events, &receipts)
            .unwrap();
        let scope = rollout.scope().clone();
        let final_digest = rollout.final_digest().clone();

        let mut reordered = rollout.records().to_vec();
        reordered.swap(1, 2);
        assert!(RolloutReducer::verify(&scope, &reordered, &final_digest).is_err());

        let mut removed = rollout.records().to_vec();
        removed.remove(1);
        assert!(RolloutReducer::verify(&scope, &removed, &final_digest).is_err());

        let mut duplicated = rollout.records().to_vec();
        duplicated.insert(2, duplicated[1].clone());
        assert!(RolloutReducer::verify(&scope, &duplicated, &final_digest).is_err());

        let mut tampered = rollout.records().to_vec();
        tampered[1] = pandora_types::RolloutRecord::link(
            &scope,
            1,
            Some(tampered[0].digest().clone()),
            pandora_types::RolloutEvidence::RuntimeEvent {
                event_id: EventId::new("event-tampered").unwrap(),
                event_type: EventType::ExecutionFailed,
                harness_id: None,
                gene_id: None,
                policy_version: None,
                receipt_id: None,
                payload: pandora_types::RolloutEventEvidence::failure("tampered").unwrap(),
            },
        )
        .unwrap();
        assert!(RolloutReducer::verify(&scope, &tampered, &final_digest).is_err());
    }

    #[derive(Deserialize)]
    struct RolloutFixture {
        context_manifest_digest: String,
        expected_final_digest: String,
    }

    #[test]
    fn fixture_replays_to_the_expected_final_hash() {
        let fixture: RolloutFixture =
            serde_json::from_str(include_str!("../tests/fixtures/rollout_reducer_v1.json"))
                .unwrap();
        let (events, receipts) = evidence();
        let rollout = RolloutReducer::new()
            .reduce(&fixture.context_manifest_digest, &events, &receipts)
            .unwrap();

        assert_eq!(
            rollout.final_digest().as_str(),
            fixture.expected_final_digest
        );
        RolloutReducer::verify(rollout.scope(), rollout.records(), rollout.final_digest()).unwrap();
    }
}
