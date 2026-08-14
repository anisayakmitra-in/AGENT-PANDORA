use crate::context_recovery::ContextRecovery;
use pandora_types::{
    ContextAssembly, ContextClassification, ContextEntry, ContextFragment, ContextReceipt,
    ContextRequest, ContextSource, ContextTrust,
};
use std::collections::HashSet;
use std::fmt;

const COMPRESSION_CHARS_PER_TOKEN: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextError {
    TokenBudgetOverflow,
}

impl fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenBudgetOverflow => formatter.write_str("context token budget overflowed"),
        }
    }
}

impl std::error::Error for ContextError {}

pub struct ContextEngine {
    recovery: ContextRecovery,
}

impl ContextEngine {
    pub const fn new() -> Self {
        Self {
            recovery: ContextRecovery::new(),
        }
    }

    pub fn assemble(
        &self,
        request: &ContextRequest,
        fragments: Vec<ContextFragment>,
    ) -> Result<ContextAssembly, ContextError> {
        let mut candidates = fragments;
        candidates.sort_by(|left, right| {
            source_rank(left.source())
                .cmp(&source_rank(right.source()))
                .then_with(|| right.priority().cmp(&left.priority()))
                .then_with(|| left.id().cmp(right.id()))
        });

        let mut seen = HashSet::new();
        let mut dropped_ids = Vec::new();
        let mut entries = Vec::new();
        let mut token_cost = 0_u32;
        let mut cacheable = true;

        for fragment in candidates {
            if fragment.is_expired(request.now()) || fragment.trust() == ContextTrust::Unverified {
                dropped_ids.push(fragment.id().to_owned());
                continue;
            }
            if !seen.insert(fragment.id().to_owned()) {
                dropped_ids.push(fragment.id().to_owned());
                continue;
            }

            let remaining = request
                .token_budget()
                .checked_sub(token_cost)
                .ok_or(ContextError::TokenBudgetOverflow)?;
            if remaining == 0 {
                dropped_ids.push(fragment.id().to_owned());
                continue;
            }

            let (content, cost, compressed) = if fragment.token_cost() <= remaining {
                (redact(&fragment), fragment.token_cost(), false)
            } else {
                let compressed = compress(&fragment, remaining);
                if compressed.1 == 0 {
                    dropped_ids.push(fragment.id().to_owned());
                    continue;
                }
                (compressed.0, compressed.1, true)
            };
            token_cost = token_cost
                .checked_add(cost)
                .ok_or(ContextError::TokenBudgetOverflow)?;
            cacheable &= fragment.classification().cacheable();
            entries.push(ContextEntry::new(
                fragment.id(),
                fragment.source(),
                fragment.trust(),
                fragment.classification(),
                content,
                cost,
                compressed,
            ));
        }

        let text = entries
            .iter()
            .map(ContextEntry::content)
            .collect::<Vec<_>>()
            .join("\n");
        let included_ids = entries.iter().map(|entry| entry.id().to_owned()).collect();
        let receipt = ContextReceipt::new(included_ids, dropped_ids, token_cost, cacheable);
        let cache_key = request.cache_key();
        Ok(ContextAssembly::new(entries, text, receipt, cache_key))
    }

    pub fn recovery(&self) -> &ContextRecovery {
        &self.recovery
    }
}

impl Default for ContextEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn source_rank(source: ContextSource) -> u8 {
    match source {
        ContextSource::Constitutional => 0,
        ContextSource::ActivePlan => 1,
        ContextSource::L1Evidence => 2,
        ContextSource::Retrieved => 3,
        ContextSource::Conversation => 4,
    }
}

fn redact(fragment: &ContextFragment) -> String {
    if fragment.classification() == ContextClassification::Secret {
        "[redacted]".to_owned()
    } else {
        fragment.content().to_owned()
    }
}

fn compress(fragment: &ContextFragment, remaining: u32) -> (String, u32) {
    if remaining == 0 {
        return (String::new(), 0);
    }
    let max_chars = remaining as usize * COMPRESSION_CHARS_PER_TOKEN;
    let content = redact(fragment);
    let mut shortened = content.chars().take(max_chars).collect::<String>();
    if shortened.len() < content.len() {
        shortened.push('…');
    }
    if shortened.is_empty() {
        (shortened, 0)
    } else {
        (shortened, remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_recovery::{RecoveryInput, RecoveryStep};
    use pandora_types::{
        ContextClassification, ContextFragment, ContextRequest, ContextSource, ContextTrust,
        SessionId, TenantId, Timestamp, WorkspaceId,
    };

    fn request(budget: u32) -> ContextRequest {
        ContextRequest::new(
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            "provider-a",
            "model-a",
            7,
            budget,
            Timestamp::from_unix_seconds(100),
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn fragment(
        id: &str,
        source: ContextSource,
        trust: ContextTrust,
        classification: ContextClassification,
        priority: u8,
        content: &str,
        token_cost: u32,
        expires_at: Option<u64>,
    ) -> ContextFragment {
        ContextFragment::new(
            id,
            source,
            trust,
            classification,
            priority,
            content,
            token_cost,
            expires_at.map(Timestamp::from_unix_seconds),
        )
        .unwrap()
    }

    #[test]
    fn assembly_keeps_constitution_and_plan_before_lower_priority_evidence() {
        let fragments = vec![
            fragment(
                "evidence",
                ContextSource::L1Evidence,
                ContextTrust::Verified,
                ContextClassification::Internal,
                10,
                "verified evidence",
                8,
                None,
            ),
            fragment(
                "plan",
                ContextSource::ActivePlan,
                ContextTrust::Verified,
                ContextClassification::Internal,
                80,
                "active plan",
                4,
                None,
            ),
            fragment(
                "constitution",
                ContextSource::Constitutional,
                ContextTrust::Constitutional,
                ContextClassification::Internal,
                100,
                "constitutional rules",
                4,
                None,
            ),
        ];

        let assembly = ContextEngine::new()
            .assemble(&request(8), fragments)
            .unwrap();

        assert_eq!(assembly.item_ids(), ["constitution", "plan"]);
        assert!(!assembly.text().contains("verified evidence"));
    }

    #[test]
    fn assembly_drops_expired_untrusted_and_duplicate_fragments() {
        let fragments = vec![
            fragment(
                "duplicate",
                ContextSource::Conversation,
                ContextTrust::Unverified,
                ContextClassification::Internal,
                5,
                "untrusted copy",
                2,
                None,
            ),
            fragment(
                "duplicate",
                ContextSource::L1Evidence,
                ContextTrust::Verified,
                ContextClassification::Internal,
                20,
                "verified copy",
                2,
                None,
            ),
            fragment(
                "expired",
                ContextSource::Retrieved,
                ContextTrust::Verified,
                ContextClassification::Internal,
                90,
                "stale evidence",
                2,
                Some(100),
            ),
        ];

        let assembly = ContextEngine::new()
            .assemble(&request(20), fragments)
            .unwrap();

        assert_eq!(assembly.item_ids(), ["duplicate"]);
        assert_eq!(assembly.text(), "verified copy");
        assert!(
            assembly
                .receipt()
                .dropped_ids()
                .contains(&"expired".to_owned())
        );
    }

    #[test]
    fn secret_content_is_redacted_and_excluded_from_cache() {
        let fragments = vec![fragment(
            "credential",
            ContextSource::Retrieved,
            ContextTrust::Verified,
            ContextClassification::Secret,
            100,
            "sk-live-secret",
            2,
            None,
        )];

        let assembly = ContextEngine::new()
            .assemble(&request(20), fragments)
            .unwrap();

        assert!(!assembly.text().contains("sk-live-secret"));
        assert!(assembly.text().contains("[redacted]"));
        assert!(!assembly.receipt().cacheable());
    }

    #[test]
    fn cache_key_keeps_sessions_providers_models_and_policy_isolated() {
        let first = ContextEngine::new()
            .assemble(&request(20), Vec::new())
            .unwrap();
        let second_request = ContextRequest::new(
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            SessionId::new("session-2").unwrap(),
            "provider-b",
            "model-b",
            8,
            20,
            Timestamp::from_unix_seconds(100),
        )
        .unwrap();
        let second = ContextEngine::new()
            .assemble(&second_request, Vec::new())
            .unwrap();

        assert_ne!(first.cache_key(), second.cache_key());
    }

    #[test]
    fn recovery_uses_the_fixed_order_and_pauses_when_safety_cannot_be_preserved() {
        let recovery = ContextRecovery::new();
        let decision = recovery.plan(RecoveryInput {
            has_verified_l1: false,
            has_fresh_evidence: false,
            fresh_evidence_is_trusted: false,
            can_reduce_scope: false,
        });

        assert_eq!(
            decision.steps(),
            [
                RecoveryStep::PruneLowValue,
                RecoveryStep::RestoreCore,
                RecoveryStep::RebuildFromL1,
                RecoveryStep::RetrieveFreshEvidence,
                RecoveryStep::ReduceScope,
                RecoveryStep::Pause,
            ]
        );
        assert!(decision.is_paused());
    }
}
