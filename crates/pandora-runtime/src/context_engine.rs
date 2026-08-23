use crate::context_recovery::ContextRecovery;
use pandora_types::{
    ContextAssembly, ContextCacheDisposition, ContextCacheKey, ContextClassification, ContextEntry,
    ContextFragment, ContextManifest, ContextReceipt, ContextRequest, ContextSource, ContextTrust,
    Timestamp,
};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::sync::Mutex;

const COMPRESSION_CHARS_PER_TOKEN: usize = 4;
const MAX_CONTEXT_CACHE_ENTRIES: usize = 64;
const MAX_CONTEXT_CACHE_ENTRY_BYTES: usize = 64 * 1024;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextCacheStats {
    hits: u64,
    misses: u64,
    entries: usize,
}

impl ContextCacheStats {
    pub const fn hits(self) -> u64 {
        self.hits
    }

    pub const fn misses(self) -> u64 {
        self.misses
    }

    pub const fn entries(self) -> usize {
        self.entries
    }
}

struct ContextCacheEntry {
    key: ContextCacheKey,
    manifest_digest: String,
    assembled_at: Timestamp,
    valid_until: Option<Timestamp>,
    assembly: ContextAssembly,
}

struct ContextCache {
    entries: VecDeque<ContextCacheEntry>,
    hits: u64,
    misses: u64,
}

impl ContextCache {
    const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            hits: 0,
            misses: 0,
        }
    }
}

pub struct ContextEngine {
    recovery: ContextRecovery,
    cache: Mutex<ContextCache>,
}

impl ContextEngine {
    pub const fn new() -> Self {
        Self {
            recovery: ContextRecovery::new(),
            cache: Mutex::new(ContextCache::new()),
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
        let cache_key = request.cache_key();
        let manifest = ContextManifest::from_fragments(&candidates);
        let valid_until = candidates
            .iter()
            .filter_map(ContextFragment::expires_at)
            .filter(|expires_at| expires_at.as_unix_seconds() > request.now().as_unix_seconds())
            .min_by_key(|expires_at| expires_at.as_unix_seconds());
        let cache_candidate = candidates.iter().all(|fragment| {
            fragment.classification().cacheable()
                && matches!(
                    fragment.source(),
                    ContextSource::Constitutional | ContextSource::ActivePlan
                )
        }) && manifest.provenance_complete();
        if cache_candidate && let Ok(mut cache) = self.cache.lock() {
            if let Some(index) = cache.entries.iter().position(|entry| {
                entry.key == cache_key && entry.manifest_digest == manifest.digest()
            }) {
                let entry = &cache.entries[index];
                let request_time = request.now().as_unix_seconds();
                let valid = request_time >= entry.assembled_at.as_unix_seconds()
                    && entry
                        .valid_until
                        .is_none_or(|expires_at| request_time < expires_at.as_unix_seconds());
                if valid {
                    let cached = entry.assembly.clone();
                    let receipt = cached
                        .receipt()
                        .clone()
                        .with_cache_disposition(ContextCacheDisposition::Hit);
                    let assembly = ContextAssembly::new(
                        cached.entries().to_vec(),
                        cached.text().to_owned(),
                        receipt,
                        cache_key,
                    );
                    cache.hits = cache.hits.saturating_add(1);
                    return Ok(assembly);
                }
                cache.entries.remove(index);
            }
            cache.misses = cache.misses.saturating_add(1);
        }

        let mut seen = HashSet::new();
        let mut dropped_ids = Vec::new();
        let mut entries = Vec::new();
        let mut token_cost = 0_u32;
        let mut cacheable = cache_candidate;

        for fragment in candidates {
            if fragment.is_expired(request.now())
                || fragment.trust() == ContextTrust::Unverified
                || fragment.classification() > request.classification_boundary()
            {
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
        let cache_disposition = if cache_candidate {
            ContextCacheDisposition::Miss
        } else {
            ContextCacheDisposition::Bypass
        };
        let receipt = ContextReceipt::new_with_evidence(
            included_ids,
            dropped_ids,
            token_cost,
            cacheable,
            &manifest,
            cache_disposition,
        );
        let assembly = ContextAssembly::new(entries, text, receipt, cache_key.clone());
        if cache_candidate
            && assembly.receipt().cacheable()
            && cached_assembly_bytes(&assembly) <= MAX_CONTEXT_CACHE_ENTRY_BYTES
            && let Ok(mut cache) = self.cache.lock()
        {
            if cache.entries.len() >= MAX_CONTEXT_CACHE_ENTRIES {
                cache.entries.pop_front();
            }
            cache.entries.push_back(ContextCacheEntry {
                key: cache_key,
                manifest_digest: manifest.digest().to_owned(),
                assembled_at: request.now(),
                valid_until,
                assembly: assembly.clone(),
            });
        }
        Ok(assembly)
    }

    pub fn recovery(&self) -> &ContextRecovery {
        &self.recovery
    }

    pub fn cache_stats(&self) -> ContextCacheStats {
        let Ok(cache) = self.cache.lock() else {
            return ContextCacheStats::default();
        };
        ContextCacheStats {
            hits: cache.hits,
            misses: cache.misses,
            entries: cache.entries.len(),
        }
    }
}

impl Default for ContextEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn cached_assembly_bytes(assembly: &ContextAssembly) -> usize {
    let mut bytes = std::mem::size_of::<ContextAssembly>()
        .saturating_add(std::mem::size_of::<ContextCacheEntry>())
        .saturating_add(assembly.text().len());
    for entry in assembly.entries() {
        bytes = bytes
            .saturating_add(std::mem::size_of::<ContextEntry>())
            .saturating_add(entry.id().len())
            .saturating_add(entry.content().len());
    }
    for id in assembly
        .receipt()
        .included_ids()
        .iter()
        .chain(assembly.receipt().dropped_ids())
    {
        bytes = bytes
            .saturating_add(std::mem::size_of::<String>())
            .saturating_add(id.len());
    }
    if let Some(manifest_digest) = assembly.receipt().manifest_digest() {
        bytes = bytes.saturating_add(manifest_digest.len().saturating_mul(2));
    }
    let key = assembly.cache_key();
    let key_bytes = key
        .tenant_id()
        .as_str()
        .len()
        .saturating_add(key.workspace_id().as_str().len())
        .saturating_add(key.session_id().as_str().len())
        .saturating_add(key.provider().len())
        .saturating_add(key.model().len());
    bytes.saturating_add(key_bytes.saturating_mul(2))
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
        ContextCacheDisposition, ContextClassification, ContextFragment, ContextOrigin,
        ContextRequest, ContextSource, ContextTrust, SessionId, TenantId, Timestamp, WorkspaceId,
    };

    fn request(budget: u32) -> ContextRequest {
        request_at(budget, 100)
    }

    fn request_at(budget: u32, now: u64) -> ContextRequest {
        scoped_request(
            "tenant-1",
            "workspace-1",
            "session-1",
            "provider-a",
            "model-a",
            7,
            budget,
            now,
            ContextClassification::Internal,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn scoped_request(
        tenant: &str,
        workspace: &str,
        session: &str,
        provider: &str,
        model: &str,
        policy_version: u32,
        budget: u32,
        now: u64,
        classification_boundary: ContextClassification,
    ) -> ContextRequest {
        ContextRequest::new(
            TenantId::new(tenant).unwrap(),
            WorkspaceId::new(workspace).unwrap(),
            SessionId::new(session).unwrap(),
            provider,
            model,
            policy_version,
            budget,
            Timestamp::from_unix_seconds(now),
        )
        .unwrap()
        .with_classification_boundary(classification_boundary)
    }

    fn request_with_boundary(
        budget: u32,
        classification_boundary: ContextClassification,
    ) -> ContextRequest {
        request(budget).with_classification_boundary(classification_boundary)
    }

    #[test]
    fn context_engine_remains_const_constructible() {
        const fn build_context_engine() -> ContextEngine {
            ContextEngine::new()
        }
        let engine = build_context_engine();

        assert_eq!(engine.cache_stats(), ContextCacheStats::default());
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
        ContextFragment::new_with_origin(
            id,
            source,
            trust,
            classification,
            priority,
            content,
            token_cost,
            expires_at.map(Timestamp::from_unix_seconds),
            ContextOrigin::new("pandora-runtime-test", id).unwrap(),
        )
        .unwrap()
    }

    fn incomplete_fragment(id: &str, content: &str) -> ContextFragment {
        ContextFragment::new(
            id,
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            100,
            content,
            4,
            None,
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
            .assemble(
                &request_with_boundary(20, ContextClassification::Secret),
                fragments,
            )
            .unwrap();

        assert!(!assembly.text().contains("sk-live-secret"));
        assert!(assembly.text().contains("[redacted]"));
        assert!(!assembly.receipt().cacheable());
    }

    #[test]
    fn admitted_sensitive_context_is_included_without_cache_reuse() {
        let fragments = vec![fragment(
            "skill",
            ContextSource::Retrieved,
            ContextTrust::Admitted,
            ContextClassification::Sensitive,
            100,
            "locally admitted guidance",
            4,
            None,
        )];

        let assembly = ContextEngine::new()
            .assemble(
                &request_with_boundary(20, ContextClassification::Sensitive),
                fragments,
            )
            .unwrap();

        assert_eq!(assembly.item_ids(), ["skill"]);
        assert_eq!(assembly.text(), "locally admitted guidance");
        assert!(!assembly.receipt().cacheable());
    }

    #[test]
    fn assembly_drops_fragments_above_the_classification_boundary() {
        let fragments = vec![
            fragment(
                "internal",
                ContextSource::Constitutional,
                ContextTrust::Constitutional,
                ContextClassification::Internal,
                100,
                "constitutional rules",
                2,
                None,
            ),
            fragment(
                "sensitive",
                ContextSource::Retrieved,
                ContextTrust::Admitted,
                ContextClassification::Sensitive,
                90,
                "local guidance",
                2,
                None,
            ),
            fragment(
                "secret",
                ContextSource::Retrieved,
                ContextTrust::Verified,
                ContextClassification::Secret,
                80,
                "credential",
                2,
                None,
            ),
        ];

        let assembly = ContextEngine::new()
            .assemble(&request(20), fragments)
            .unwrap();

        assert_eq!(assembly.item_ids(), ["internal"]);
        assert!(!assembly.text().contains("local guidance"));
        assert!(!assembly.text().contains("credential"));
        assert!(
            assembly
                .receipt()
                .dropped_ids()
                .contains(&"sensitive".to_owned())
        );
        assert!(
            assembly
                .receipt()
                .dropped_ids()
                .contains(&"secret".to_owned())
        );
    }

    #[test]
    fn cache_key_keeps_sessions_providers_models_and_policy_isolated() {
        let engine = ContextEngine::new();
        let first = engine.assemble(&request(20), Vec::new()).unwrap();
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
        let second = engine.assemble(&second_request, Vec::new()).unwrap();

        assert_ne!(first.cache_key(), second.cache_key());
        assert_eq!(engine.cache_stats().hits(), 0);
        assert_eq!(engine.cache_stats().misses(), 2);
    }

    #[test]
    fn cache_lookup_isolates_each_request_dimension() {
        let base = request(20);
        let variants = [
            scoped_request(
                "tenant-2",
                "workspace-1",
                "session-1",
                "provider-a",
                "model-a",
                7,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-2",
                "session-1",
                "provider-a",
                "model-a",
                7,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-2",
                "provider-a",
                "model-a",
                7,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-1",
                "provider-b",
                "model-a",
                7,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-1",
                "provider-a",
                "model-b",
                7,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-1",
                "provider-a",
                "model-a",
                8,
                20,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-1",
                "provider-a",
                "model-a",
                7,
                19,
                100,
                ContextClassification::Internal,
            ),
            scoped_request(
                "tenant-1",
                "workspace-1",
                "session-1",
                "provider-a",
                "model-a",
                7,
                20,
                100,
                ContextClassification::Public,
            ),
        ];

        for variant in variants {
            let engine = ContextEngine::new();
            engine.assemble(&base, Vec::new()).unwrap();
            engine.assemble(&variant, Vec::new()).unwrap();

            assert_eq!(engine.cache_stats().hits(), 0);
            assert_eq!(engine.cache_stats().misses(), 2);
        }
    }

    #[test]
    fn cache_does_not_reuse_an_assembly_created_in_the_future() {
        let engine = ContextEngine::new();

        engine.assemble(&request_at(20, 110), Vec::new()).unwrap();
        engine.assemble(&request_at(20, 100), Vec::new()).unwrap();

        assert_eq!(engine.cache_stats().hits(), 0);
        assert_eq!(engine.cache_stats().misses(), 2);
    }

    #[test]
    fn repeated_safe_assembly_reuses_the_exact_cached_result() {
        let engine = ContextEngine::new();
        let fragments = vec![fragment(
            "constitution",
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            100,
            "constitutional rules",
            4,
            None,
        )];

        let first = engine.assemble(&request(20), fragments.clone()).unwrap();
        let second = engine.assemble(&request(20), fragments).unwrap();
        let stats = engine.cache_stats();

        assert_eq!(first.entries(), second.entries());
        assert_eq!(first.text(), second.text());
        assert_eq!(first.cache_key(), second.cache_key());
        assert_eq!(
            first.receipt().cache_disposition(),
            ContextCacheDisposition::Miss
        );
        assert_eq!(
            second.receipt().cache_disposition(),
            ContextCacheDisposition::Hit
        );
        assert_eq!(stats.hits(), 1);
        assert_eq!(stats.misses(), 1);
        assert_eq!(stats.entries(), 1);
    }

    #[test]
    fn cache_hit_returns_hit_receipt_without_changing_assembly_text() {
        let engine = ContextEngine::new();
        let fragments = vec![fragment(
            "constitution",
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            100,
            "constitutional rules",
            4,
            None,
        )];

        let first = engine.assemble(&request(20), fragments.clone()).unwrap();
        let second = engine.assemble(&request(20), fragments).unwrap();

        assert_eq!(first.text(), second.text());
        assert_eq!(
            first.receipt().cache_disposition(),
            ContextCacheDisposition::Miss
        );
        assert_eq!(
            second.receipt().cache_disposition(),
            ContextCacheDisposition::Hit
        );
        assert_eq!(
            first.receipt().manifest_digest(),
            second.receipt().manifest_digest()
        );
    }

    #[test]
    fn cache_miss_returns_manifest_evidence() {
        let assembly = ContextEngine::new()
            .assemble(
                &request(20),
                vec![fragment(
                    "plan",
                    ContextSource::ActivePlan,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    100,
                    "active plan",
                    4,
                    None,
                )],
            )
            .unwrap();

        assert_eq!(
            assembly.receipt().cache_disposition(),
            ContextCacheDisposition::Miss
        );
        assert!(assembly.receipt().provenance_complete());
        assert!(
            assembly
                .receipt()
                .manifest_digest()
                .is_some_and(|digest| digest.starts_with("sha256:"))
        );
        assert_eq!(
            assembly.receipt().projection_version(),
            pandora_types::CONTEXT_PROJECTION_VERSION
        );
    }

    #[test]
    fn retained_size_counts_both_manifest_digest_copies() {
        let assembly = ContextEngine::new()
            .assemble(
                &request(20),
                vec![fragment(
                    "plan",
                    ContextSource::ActivePlan,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    100,
                    "active plan",
                    4,
                    None,
                )],
            )
            .unwrap();
        let digest_bytes = assembly.receipt().manifest_digest().unwrap().len();
        let without_evidence = ContextAssembly::new(
            assembly.entries().to_vec(),
            assembly.text().to_owned(),
            ContextReceipt::new(
                assembly.receipt().included_ids().to_vec(),
                assembly.receipt().dropped_ids().to_vec(),
                assembly.receipt().token_cost(),
                assembly.receipt().cacheable(),
            ),
            assembly.cache_key().clone(),
        );

        assert_eq!(
            cached_assembly_bytes(&assembly) - cached_assembly_bytes(&without_evidence),
            digest_bytes * 2
        );
    }

    #[test]
    fn incomplete_or_dynamic_provenance_bypasses_cache() {
        let engine = ContextEngine::new();
        let incomplete = vec![incomplete_fragment("constitution", "constitutional rules")];

        let first = engine.assemble(&request(20), incomplete.clone()).unwrap();
        let second = engine.assemble(&request(20), incomplete).unwrap();

        assert_eq!(
            first.receipt().cache_disposition(),
            ContextCacheDisposition::Bypass
        );
        assert_eq!(
            second.receipt().cache_disposition(),
            ContextCacheDisposition::Bypass
        );
        assert!(!first.receipt().provenance_complete());
        assert_eq!(engine.cache_stats().entries(), 0);

        let dynamic = ContextEngine::new()
            .assemble(
                &request(20),
                vec![fragment(
                    "conversation",
                    ContextSource::Conversation,
                    ContextTrust::Admitted,
                    ContextClassification::Internal,
                    100,
                    "latest conversation",
                    4,
                    None,
                )],
            )
            .unwrap();
        assert_eq!(
            dynamic.receipt().cache_disposition(),
            ContextCacheDisposition::Bypass
        );
        assert!(dynamic.receipt().provenance_complete());
    }

    #[test]
    fn changed_fragment_content_does_not_reuse_stale_context() {
        let engine = ContextEngine::new();
        let original = vec![fragment(
            "plan",
            ContextSource::ActivePlan,
            ContextTrust::Verified,
            ContextClassification::Internal,
            100,
            "original plan",
            4,
            None,
        )];
        let revised = vec![fragment(
            "plan",
            ContextSource::ActivePlan,
            ContextTrust::Verified,
            ContextClassification::Internal,
            100,
            "revised plan",
            4,
            None,
        )];

        let first = engine.assemble(&request(20), original).unwrap();
        let second = engine.assemble(&request(20), revised).unwrap();
        let stats = engine.cache_stats();

        assert_eq!(first.text(), "original plan");
        assert_eq!(second.text(), "revised plan");
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 2);
    }

    #[test]
    fn changed_fragment_metadata_does_not_reuse_stale_context() {
        let engine = ContextEngine::new();
        let original = vec![
            fragment(
                "plan-a",
                ContextSource::ActivePlan,
                ContextTrust::Verified,
                ContextClassification::Internal,
                100,
                "plan a",
                4,
                None,
            ),
            fragment(
                "plan-b",
                ContextSource::ActivePlan,
                ContextTrust::Verified,
                ContextClassification::Internal,
                90,
                "plan b",
                4,
                None,
            ),
        ];
        let reprioritized = vec![
            fragment(
                "plan-a",
                ContextSource::ActivePlan,
                ContextTrust::Verified,
                ContextClassification::Internal,
                80,
                "plan a",
                4,
                None,
            ),
            fragment(
                "plan-b",
                ContextSource::ActivePlan,
                ContextTrust::Verified,
                ContextClassification::Internal,
                110,
                "plan b",
                4,
                None,
            ),
        ];

        let first = engine.assemble(&request(20), original).unwrap();
        let second = engine.assemble(&request(20), reprioritized).unwrap();

        assert_eq!(first.item_ids(), ["plan-a", "plan-b"]);
        assert_eq!(second.item_ids(), ["plan-b", "plan-a"]);
        assert_eq!(engine.cache_stats().hits(), 0);
    }

    #[test]
    fn sensitive_context_bypasses_the_cache() {
        let engine = ContextEngine::new();
        let sensitive = vec![fragment(
            "execution-evidence",
            ContextSource::L1Evidence,
            ContextTrust::Verified,
            ContextClassification::Sensitive,
            100,
            "bounded execution evidence",
            4,
            None,
        )];
        let scoped_request = request_with_boundary(20, ContextClassification::Sensitive);

        let first = engine.assemble(&scoped_request, sensitive.clone()).unwrap();
        let second = engine.assemble(&scoped_request, sensitive).unwrap();
        let stats = engine.cache_stats();

        assert_eq!(first.text(), "bounded execution evidence");
        assert_eq!(second.text(), "bounded execution evidence");
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.entries(), 0);
    }

    #[test]
    fn dynamic_context_sources_bypass_the_cache_even_when_marked_internal() {
        for source in [
            ContextSource::L1Evidence,
            ContextSource::Retrieved,
            ContextSource::Conversation,
        ] {
            let engine = ContextEngine::new();
            let fragments = vec![fragment(
                "dynamic-context",
                source,
                ContextTrust::Verified,
                ContextClassification::Internal,
                100,
                "dynamic context",
                4,
                None,
            )];

            engine.assemble(&request(20), fragments.clone()).unwrap();
            engine.assemble(&request(20), fragments).unwrap();
            let stats = engine.cache_stats();

            assert_eq!(stats.hits(), 0, "source {} was cached", source.as_str());
            assert_eq!(stats.entries(), 0, "source {} was stored", source.as_str());
        }
    }

    #[test]
    fn cached_context_is_not_reused_after_a_fragment_expires() {
        let engine = ContextEngine::new();
        let fragments = vec![fragment(
            "active-plan",
            ContextSource::ActivePlan,
            ContextTrust::Verified,
            ContextClassification::Internal,
            100,
            "active plan",
            4,
            Some(110),
        )];

        let fresh = engine
            .assemble(&request_at(20, 100), fragments.clone())
            .unwrap();
        let expired = engine.assemble(&request_at(20, 110), fragments).unwrap();
        let stats = engine.cache_stats();

        assert_eq!(fresh.text(), "active plan");
        assert_eq!(expired.text(), "");
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 2);
    }

    #[test]
    fn cache_evicts_the_oldest_assembly_at_its_fixed_capacity() {
        let engine = ContextEngine::new();
        for index in 0..65 {
            engine
                .assemble(
                    &request(20),
                    vec![fragment(
                        "plan",
                        ContextSource::ActivePlan,
                        ContextTrust::Verified,
                        ContextClassification::Internal,
                        100,
                        &format!("plan {index}"),
                        4,
                        None,
                    )],
                )
                .unwrap();
        }

        engine
            .assemble(
                &request(20),
                vec![fragment(
                    "plan",
                    ContextSource::ActivePlan,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    100,
                    "plan 0",
                    4,
                    None,
                )],
            )
            .unwrap();
        let stats = engine.cache_stats();

        assert_eq!(stats.entries(), 64);
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 66);
    }

    #[test]
    fn oversized_assembly_bypasses_the_cache_even_with_a_small_token_claim() {
        let engine = ContextEngine::new();
        let oversized = "x".repeat(65_537);
        let fragments = vec![fragment(
            "oversized-plan",
            ContextSource::ActivePlan,
            ContextTrust::Verified,
            ContextClassification::Internal,
            100,
            &oversized,
            1,
            None,
        )];

        engine.assemble(&request(20), fragments.clone()).unwrap();
        engine.assemble(&request(20), fragments).unwrap();
        let stats = engine.cache_stats();

        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.entries(), 0);
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
