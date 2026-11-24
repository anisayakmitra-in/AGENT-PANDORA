pub use crate::hnsw::HnswIndexStats;
use crate::hnsw::{
    HnswConfig, HnswError, HnswIndex, HnswVector, PVHN_HEADER_SIZE, PVHN_TRAILER_SIZE,
};
use crate::semantic_indexer::{TropicalHybridRanker, TropicalProjector, TropicalVector};
pub use crate::semantic_memory::{ContextSearchOptions, HybridSearchResult, SearchResultItem};
use crate::semantic_memory::{
    DenseFeatureHasher, HybridRrfRanker, QuantizedVector, SparseBm25Index,
    extract_term_frequencies, generate_snippet,
};
use atomic_write_file::AtomicWriteFile;
use pandora_types::{
    ContextClassification, ContextFragment, ContextOrigin, ContextSource, ContextTrust, ContextUri,
    FeatureMap, MemoryIntent, MemoryRecord, MemoryTier, SessionArchiveRecord, SkillManifest,
    StoredSop, TenantId, VfsDirectoryEntry, VfsNodeKind, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VFS_INDEX_MAGIC: &[u8; 4] = b"PVFS";
pub const PVFS_TOMBSTONE_MAGIC: &[u8; 4] = b"TOMB";
pub const VFS_INDEX_VERSION_V1: u32 = 1;
pub const VFS_INDEX_VERSION_V2: u32 = 2;
pub const VFS_INDEX_VERSION_V3: u32 = 3;
pub const VFS_INDEX_VERSION: u32 = VFS_INDEX_VERSION_V3;
pub const VFS_INDEX_HEADER_SIZE: usize = 68;
pub const VFS_INDEX_TRAILER_SIZE: usize = 32;
pub const VFS_DENSE_DIMENSION: u32 = 128;
pub const VFS_QUANTIZED_VECTOR_SIZE: usize = 132; // 4 bytes scale + 128 bytes i8 data
pub const VFS_HNSW_INDEX_PATH_DEFAULT: &str = ".pandora/indices/vfs_hnsw.idx";
pub const DEFAULT_AUTO_COMPACT_MIN_TOMBSTONES: usize = 8;
pub const DEFAULT_AUTO_COMPACT_MAX_RATIO: f64 = 0.25;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VfsError {
    InvalidUri(String),
    NotFound(String),
    IsADirectory(String),
    AccessDenied(String),
    BackendError(String),
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUri(msg) => write!(f, "invalid context URI: {msg}"),
            Self::NotFound(path) => write!(f, "context node not found: {path}"),
            Self::IsADirectory(path) => write!(f, "path is a directory: {path}"),
            Self::AccessDenied(msg) => write!(f, "context VFS access denied: {msg}"),
            Self::BackendError(msg) => write!(f, "context VFS backend error: {msg}"),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<HnswError> for VfsError {
    fn from(err: HnswError) -> Self {
        VfsError::BackendError(err.to_string())
    }
}

/// Persistent VFS index statistics and integrity metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct VfsIndexStats {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub doc_count: u32,
    pub vocab_size: u32,
    pub dense_dimension: u32,
    pub total_tokens: u64,
    pub created_at_unix: u64,
    pub corpus_hash: String,
    pub checksum_valid: bool,
    pub is_fresh: bool,
    pub version: u32,
    pub vector_bytes: u64,
    pub vector_bytes_saved: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    #[serde(default)]
    pub tombstone_count: u32,
    #[serde(default)]
    pub tombstone_ratio: f64,
    #[serde(default)]
    pub compaction_count: u32,
    #[serde(default)]
    pub last_compacted_unix: u64,
}

/// Summary report returned by [`ContextVfs::compact_index`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VfsCompactionReport {
    pub docs_before: usize,
    pub docs_after: usize,
    pub tombstones_purged: usize,
    pub hnsw_compacted: bool,
    pub hnsw_edges_repaired: usize,
    pub hnsw_nodes_reconnected: usize,
    pub compaction_count: u32,
    pub compacted_at_unix: u64,
}

/// A mounted virtual document ready for indexing.
#[derive(Clone, Debug)]
pub struct VfsDoc {
    pub uri: ContextUri,
    pub title: String,
    pub content: String,
    pub tier: Option<MemoryTier>,
    pub intent: Option<MemoryIntent>,
}

/// Computes deterministic SHA-256 fingerprint over URI, title, content, tier, and intent.
pub fn compute_doc_fingerprint(doc: &VfsDoc) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let uri_bytes = doc.uri.as_str().as_bytes();
    hasher.update((uri_bytes.len() as u32).to_le_bytes());
    hasher.update(uri_bytes);

    let title_bytes = doc.title.as_bytes();
    hasher.update((title_bytes.len() as u32).to_le_bytes());
    hasher.update(title_bytes);

    let content_bytes = doc.content.as_bytes();
    hasher.update((content_bytes.len() as u32).to_le_bytes());
    hasher.update(content_bytes);

    hasher.update([tier_to_u8(doc.tier)]);
    hasher.update([intent_to_u8(doc.intent)]);
    hasher.finalize().into()
}

/// Differential index tuple containing BM25 postings, tropical vectors, and legacy quantized vectors.
pub type TropicalDifferentialIndex = (
    SparseBm25Index,
    Vec<(usize, TropicalVector)>,
    Vec<(usize, QuantizedVector)>,
);

/// Incremental diff cache entry representing an indexed buffer in ContextVfs.
#[derive(Clone, Debug, PartialEq)]
pub struct CachedBufferEntry {
    pub uri: ContextUri,
    pub fingerprint: [u8; 32],
    pub vector: QuantizedVector,
    pub tropical_vector: TropicalVector,
    pub doc_length: usize,
    pub term_frequencies: Vec<(String, u32)>,
}

/// In-memory representation of a deserialized VFS persistent index.
#[derive(Clone, Debug)]
pub struct DeserializedVfsIndex {
    pub magic: [u8; 4],
    pub version: u32,
    pub created_at: u64,
    pub doc_count: u32,
    pub dense_dim: u32,
    pub total_tokens: u64,
    pub vocab_size: u32,
    pub corpus_hash: [u8; 32],
    pub docs: Vec<VfsDoc>,
    pub doc_vectors: Vec<(usize, QuantizedVector)>,
    pub tropical_vectors: Vec<(usize, TropicalVector)>,
    pub bm25: SparseBm25Index,
    pub tombstoned_uris: Vec<String>,
    pub compaction_count: u32,
    pub last_compacted_unix: u64,
}

pub struct ContextVfs {
    meta_rules: RwLock<Vec<ContextFragment>>,
    l1_records: RwLock<Vec<MemoryRecord>>,
    l2_records: RwLock<Vec<MemoryRecord>>,
    sops: RwLock<Vec<StoredSop>>,
    archives: RwLock<Vec<SessionArchiveRecord>>,
    feature_maps: RwLock<Vec<(String, FeatureMap)>>,
    skills: RwLock<Vec<SkillManifest>>,
    index_path: RwLock<Option<PathBuf>>,
    cached_index: RwLock<Option<Arc<DeserializedVfsIndex>>>,
    diff_cache: RwLock<HashMap<String, CachedBufferEntry>>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    hnsw_index_path: RwLock<Option<PathBuf>>,
    cached_hnsw_index: RwLock<Option<Arc<HnswIndex>>>,
    hnsw_docs: RwLock<Vec<Option<VfsDoc>>>,
    hnsw_uri_to_doc_id: RwLock<HashMap<String, usize>>,
    tombstoned_uris: RwLock<BTreeSet<String>>,
    auto_compact_min_tombstones: AtomicUsize,
    auto_compact_max_ratio_bits: AtomicU64,
    compaction_count: AtomicU32,
    last_compacted_unix: AtomicU64,
    last_modified_unix: AtomicU64,
}

impl Default for ContextVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextVfs {
    pub fn new() -> Self {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let default_rule = ContextFragment::new_with_origin(
            "meta:rules",
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            255,
            "# System Constitutional Rules\n1. Maintain architectural integrity.\n2. Do not bypass security bounds.\n3. Verify all code changes with tests.",
            64,
            None,
            ContextOrigin::new("pandora-runtime", "constitution")
                .unwrap_or_else(|_| ContextOrigin::incomplete()),
        )
        .expect("default meta rules should be valid");

        Self {
            meta_rules: RwLock::new(vec![default_rule]),
            l1_records: RwLock::new(Vec::new()),
            l2_records: RwLock::new(Vec::new()),
            sops: RwLock::new(Vec::new()),
            archives: RwLock::new(Vec::new()),
            feature_maps: RwLock::new(Vec::new()),
            skills: RwLock::new(Vec::new()),
            index_path: RwLock::new(Some(PathBuf::from(".pandora/indices/vfs.idx"))),
            cached_index: RwLock::new(None),
            diff_cache: RwLock::new(HashMap::new()),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            hnsw_index_path: RwLock::new(Some(PathBuf::from(VFS_HNSW_INDEX_PATH_DEFAULT))),
            cached_hnsw_index: RwLock::new(None),
            hnsw_docs: RwLock::new(Vec::new()),
            hnsw_uri_to_doc_id: RwLock::new(HashMap::new()),
            tombstoned_uris: RwLock::new(BTreeSet::new()),
            auto_compact_min_tombstones: AtomicUsize::new(DEFAULT_AUTO_COMPACT_MIN_TOMBSTONES),
            auto_compact_max_ratio_bits: AtomicU64::new(DEFAULT_AUTO_COMPACT_MAX_RATIO.to_bits()),
            compaction_count: AtomicU32::new(0),
            last_compacted_unix: AtomicU64::new(0),
            last_modified_unix: AtomicU64::new(now_unix),
        }
    }

    pub fn from_session_store(
        store: &crate::sessions::SessionStore,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Self, VfsError> {
        let vfs = Self::new();

        let sops = store
            .list_sops_by_intent(tenant_id, workspace_id, None)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        for sop in sops {
            vfs.add_sop(sop);
        }

        let archives = store
            .list_recent_archives(tenant_id, workspace_id, 256)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        for arc in archives {
            vfs.add_archive(arc);
        }

        let records = store
            .list_memory_records_by_workspace(tenant_id, workspace_id, None, 512)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        for rec in records {
            vfs.add_memory_record(rec);
        }

        Ok(vfs)
    }

    fn mark_buffer_modified(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_modified_unix.store(now, Ordering::Relaxed);
    }

    /// Configures the automated tombstone compaction thresholds (builder pattern).
    ///
    /// Compaction triggers automatically during [`Self::save_index_to_disk`] or
    /// [`Self::run_index_maintenance`] when `tombstone_count >= min_tombstones` and the
    /// tombstone ratio `tombstones / (live + tombstones)` is at least `max_ratio`
    /// (or when `tombstone_count >= (min_tombstones * 4).max(64)`).
    pub fn with_auto_compact_threshold(self, min_tombstones: usize, max_ratio: f64) -> Self {
        self.set_auto_compact_threshold(min_tombstones, max_ratio);
        self
    }

    /// Updates the automated tombstone compaction thresholds in-place.
    pub fn set_auto_compact_threshold(&self, min_tombstones: usize, max_ratio: f64) {
        let clamped_min = min_tombstones.max(1);
        let clamped_ratio = if max_ratio.is_finite() {
            max_ratio.clamp(0.0, 1.0)
        } else {
            DEFAULT_AUTO_COMPACT_MAX_RATIO
        };
        self.auto_compact_min_tombstones
            .store(clamped_min, Ordering::Relaxed);
        self.auto_compact_max_ratio_bits
            .store(clamped_ratio.to_bits(), Ordering::Relaxed);
    }

    /// Returns `(min_tombstones, max_ratio)` configured for automated compaction.
    pub fn auto_compact_threshold(&self) -> (usize, f64) {
        (
            self.auto_compact_min_tombstones.load(Ordering::Relaxed),
            f64::from_bits(self.auto_compact_max_ratio_bits.load(Ordering::Relaxed)),
        )
    }

    /// Returns the number of tombstoned URIs currently tracked by this `ContextVfs`.
    pub fn tombstone_count(&self) -> usize {
        self.tombstoned_uris.read().map(|g| g.len()).unwrap_or(0)
    }

    /// Returns `true` if `uri` (exact or L1/L2 shorthand) is currently tombstoned.
    pub fn is_tombstoned(&self, uri: &str) -> bool {
        self.tombstoned_uris
            .read()
            .map(|g| g.contains(uri) || self.matches_shorthand_tombstone(&g, uri))
            .unwrap_or(false)
    }

    /// Returns a deterministic sorted list of currently tombstoned URIs.
    pub fn tombstoned_uris(&self) -> Vec<String> {
        self.tombstoned_uris
            .read()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the cumulative number of compactions executed on this `ContextVfs`.
    pub fn compaction_count(&self) -> u32 {
        self.compaction_count.load(Ordering::Relaxed)
    }

    /// Returns the Unix timestamp of the most recent compaction (`0` if never compacted).
    pub fn last_compacted_unix(&self) -> u64 {
        self.last_compacted_unix.load(Ordering::Relaxed)
    }

    /// Returns `true` when tracked tombstones exceed the configured auto-compaction threshold.
    pub fn should_auto_compact(&self) -> bool {
        let tombstones = self.tombstone_count();
        if tombstones == 0 {
            return false;
        }
        let (min_tombstones, max_ratio) = self.auto_compact_threshold();
        let live_docs = self.collect_all_documents().map(|d| d.len()).unwrap_or(0);
        let total = live_docs.saturating_add(tombstones);
        let ratio = if total == 0 {
            0.0
        } else {
            tombstones as f64 / total as f64
        };
        (tombstones >= min_tombstones && ratio >= max_ratio)
            || tombstones >= min_tombstones.saturating_mul(4).max(64)
    }

    fn matches_shorthand_tombstone(&self, tombstones: &BTreeSet<String>, uri: &str) -> bool {
        if let Some(rest) = uri.strip_prefix("pandora://facts/")
            && !rest.contains('/')
        {
            return tombstones
                .iter()
                .any(|t| t.starts_with("pandora://facts/") && t.ends_with(&format!("/{rest}")));
        }
        if let Some(rest) = uri.strip_prefix("pandora://index/")
            && !rest.contains('/')
        {
            return tombstones
                .iter()
                .any(|t| t.starts_with("pandora://index/") && t.ends_with(&format!("/{rest}")));
        }
        false
    }

    fn clear_tombstone_for_uri(&self, canonical_uri: &str) {
        if let Ok(mut tombstones) = self.tombstoned_uris.write() {
            tombstones.remove(canonical_uri);
        }
    }

    fn populate_hnsw_doc_mapping(&self, docs: &[VfsDoc]) {
        if let Ok(mut hnsw_docs) = self.hnsw_docs.write()
            && let Ok(mut uri_map) = self.hnsw_uri_to_doc_id.write()
        {
            hnsw_docs.clear();
            uri_map.clear();
            hnsw_docs.reserve(docs.len());
            uri_map.reserve(docs.len());
            for (idx, doc) in docs.iter().enumerate() {
                uri_map.insert(doc.uri.as_str().to_string(), idx);
                hnsw_docs.push(Some(doc.clone()));
            }
        }
    }

    fn sync_hnsw_on_upsert(&self, doc: &VfsDoc) {
        let has_cached = self.cached_hnsw_index.read().is_ok_and(|g| g.is_some());
        if !has_cached {
            return;
        }

        let Ok(all_docs) = self.collect_all_documents() else {
            return;
        };
        let new_corpus_hash = compute_corpus_hash(&all_docs);

        let uses_tropical = self
            .cached_hnsw_index
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|idx| idx.nodes().iter().any(|n| n.vector.is_tropical()) || idx.is_empty())
            })
            .unwrap_or(true);

        let fp = compute_doc_fingerprint(doc);
        let (doc_len, term_freqs) = extract_term_frequencies(&doc.content);
        let hasher = DenseFeatureHasher::new();
        let f32_vec = hasher.vectorize(&doc.content);
        let qv = QuantizedVector::quantize(&f32_vec);
        let tv = TropicalProjector::project(&doc.content);

        if let Ok(mut cache) = self.diff_cache.write() {
            cache.insert(
                doc.uri.as_str().to_string(),
                CachedBufferEntry {
                    uri: doc.uri.clone(),
                    fingerprint: fp,
                    vector: qv,
                    tropical_vector: tv,
                    doc_length: doc_len,
                    term_frequencies: term_freqs,
                },
            );
        }

        let hnsw_vec = if uses_tropical {
            HnswVector::Tropical(tv)
        } else {
            HnswVector::Quantized(qv)
        };

        if let Ok(mut hnsw_guard) = self.cached_hnsw_index.write()
            && let Some(arc_hnsw) = hnsw_guard.as_mut()
            && let Ok(mut docs_vec) = self.hnsw_docs.write()
            && let Ok(mut uri_map) = self.hnsw_uri_to_doc_id.write()
        {
            if docs_vec.is_empty() && !arc_hnsw.is_empty() {
                for (idx, d) in all_docs.iter().enumerate() {
                    if d.uri.as_str() != doc.uri.as_str() {
                        uri_map.insert(d.uri.as_str().to_string(), idx);
                    }
                }
                *docs_vec = all_docs
                    .iter()
                    .map(|d| {
                        if d.uri.as_str() == doc.uri.as_str() {
                            None
                        } else {
                            Some(d.clone())
                        }
                    })
                    .collect();
            }

            let uri_key = doc.uri.as_str().to_string();
            let doc_id = if let Some(&existing_id) = uri_map.get(&uri_key) {
                if existing_id < docs_vec.len() {
                    docs_vec[existing_id] = Some(doc.clone());
                } else {
                    while docs_vec.len() < existing_id {
                        docs_vec.push(None);
                    }
                    docs_vec.push(Some(doc.clone()));
                }
                existing_id
            } else {
                let next_id = docs_vec.len().max(
                    arc_hnsw
                        .nodes()
                        .iter()
                        .map(|n| n.doc_id + 1)
                        .max()
                        .unwrap_or(0),
                );
                while docs_vec.len() < next_id {
                    docs_vec.push(None);
                }
                docs_vec.push(Some(doc.clone()));
                uri_map.insert(uri_key, next_id);
                next_id
            };

            let hnsw_mut = Arc::make_mut(arc_hnsw);
            hnsw_mut.upsert(doc_id, hnsw_vec);
            hnsw_mut.set_corpus_hash(new_corpus_hash);
        }
    }

    fn sync_hnsw_on_delete(&self, canonical_uri: &str) {
        let has_cached = self.cached_hnsw_index.read().is_ok_and(|g| g.is_some());
        if !has_cached {
            return;
        }

        let Ok(all_docs) = self.collect_all_documents() else {
            return;
        };
        let new_corpus_hash = compute_corpus_hash(&all_docs);

        if let Ok(mut hnsw_guard) = self.cached_hnsw_index.write()
            && let Some(arc_hnsw) = hnsw_guard.as_mut()
            && let Ok(mut docs_vec) = self.hnsw_docs.write()
            && let Ok(mut uri_map) = self.hnsw_uri_to_doc_id.write()
        {
            let hnsw_mut = Arc::make_mut(arc_hnsw);
            if let Some(doc_id) = uri_map.remove(canonical_uri) {
                hnsw_mut.mark_deleted(doc_id);
                if doc_id < docs_vec.len() {
                    docs_vec[doc_id] = None;
                }
            }
            hnsw_mut.set_corpus_hash(new_corpus_hash);
        }
    }

    pub fn add_meta_rule(&self, rule: ContextFragment) {
        let maybe_doc = meta_rule_to_doc(&rule);
        if let Ok(mut rules) = self.meta_rules.write() {
            rules.retain(|r| r.id() != rule.id());
            rules.push(rule);
        }
        if let Some(doc) = maybe_doc {
            self.clear_tombstone_for_uri(doc.uri.as_str());
            self.mark_buffer_modified();
            self.sync_hnsw_on_upsert(&doc);
        } else {
            self.mark_buffer_modified();
        }
    }

    pub fn add_sop(&self, sop: StoredSop) {
        let maybe_doc = sop_to_doc(&sop);
        if let Ok(mut sops) = self.sops.write() {
            sops.retain(|s| s.sop.id != sop.sop.id);
            sops.push(sop);
        }
        if let Some(doc) = maybe_doc {
            self.clear_tombstone_for_uri(doc.uri.as_str());
            self.mark_buffer_modified();
            self.sync_hnsw_on_upsert(&doc);
        } else {
            self.mark_buffer_modified();
        }
    }

    pub fn add_archive(&self, archive: SessionArchiveRecord) {
        let maybe_doc = archive_to_doc(&archive);
        if let Ok(mut archives) = self.archives.write() {
            archives.retain(|a| a.archive_id != archive.archive_id);
            archives.push(archive);
        }
        if let Some(doc) = maybe_doc {
            self.clear_tombstone_for_uri(doc.uri.as_str());
            self.mark_buffer_modified();
            self.sync_hnsw_on_upsert(&doc);
        } else {
            self.mark_buffer_modified();
        }
    }

    pub fn add_memory_record(&self, record: MemoryRecord) {
        match record.tier() {
            MemoryTier::L1 => {
                let maybe_doc = l1_record_to_doc(&record);
                if let Ok(mut list) = self.l1_records.write() {
                    list.retain(|r| r.id() != record.id());
                    list.push(record);
                }
                if let Some(doc) = maybe_doc {
                    self.clear_tombstone_for_uri(doc.uri.as_str());
                    self.mark_buffer_modified();
                    self.sync_hnsw_on_upsert(&doc);
                } else {
                    self.mark_buffer_modified();
                }
            }
            MemoryTier::L2 => {
                let maybe_doc = l2_record_to_doc(&record);
                if let Ok(mut list) = self.l2_records.write() {
                    list.retain(|r| r.id() != record.id());
                    list.push(record);
                }
                if let Some(doc) = maybe_doc {
                    self.clear_tombstone_for_uri(doc.uri.as_str());
                    self.mark_buffer_modified();
                    self.sync_hnsw_on_upsert(&doc);
                } else {
                    self.mark_buffer_modified();
                }
            }
            _ => {}
        }
    }

    pub fn add_feature_map(&self, name: impl Into<String>, feature_map: FeatureMap) {
        let name = name.into();
        let maybe_doc = feature_map_to_doc(&name, &feature_map);
        if let Ok(mut maps) = self.feature_maps.write() {
            maps.retain(|(n, _)| n != &name);
            maps.push((name, feature_map));
        }
        if let Some(doc) = maybe_doc {
            self.clear_tombstone_for_uri(doc.uri.as_str());
            self.mark_buffer_modified();
            self.sync_hnsw_on_upsert(&doc);
        } else {
            self.mark_buffer_modified();
        }
    }

    pub fn add_skill(&self, skill: SkillManifest) {
        let maybe_doc = skill_to_doc(&skill);
        if let Ok(mut skills) = self.skills.write() {
            skills.retain(|s| s.id() != skill.id());
            skills.push(skill);
        }
        if let Some(doc) = maybe_doc {
            self.clear_tombstone_for_uri(doc.uri.as_str());
            self.mark_buffer_modified();
            self.sync_hnsw_on_upsert(&doc);
        } else {
            self.mark_buffer_modified();
        }
    }

    fn resolve_canonical_uri(&self, uri: &str) -> Option<String> {
        if let Ok(rules) = self.meta_rules.read() {
            for rule in rules.iter() {
                if let Some(doc) = meta_rule_to_doc(rule)
                    && doc.uri.as_str() == uri
                {
                    return Some(doc.uri.as_str().to_string());
                }
            }
        }
        if let Ok(l1) = self.l1_records.read() {
            for rec in l1.iter() {
                if let Some(doc) = l1_record_to_doc(rec) {
                    let shorthand = format!("pandora://index/{}", rec.id().as_str());
                    if doc.uri.as_str() == uri || shorthand == uri {
                        return Some(doc.uri.as_str().to_string());
                    }
                }
            }
        }
        if let Ok(l2) = self.l2_records.read() {
            for rec in l2.iter() {
                if let Some(doc) = l2_record_to_doc(rec) {
                    let shorthand = format!("pandora://facts/{}", rec.id().as_str());
                    if doc.uri.as_str() == uri || shorthand == uri {
                        return Some(doc.uri.as_str().to_string());
                    }
                }
            }
        }
        if let Ok(sops) = self.sops.read() {
            for sop in sops.iter() {
                if let Some(doc) = sop_to_doc(sop)
                    && doc.uri.as_str() == uri
                {
                    return Some(doc.uri.as_str().to_string());
                }
            }
        }
        if let Ok(archives) = self.archives.read() {
            for arc in archives.iter() {
                if let Some(doc) = archive_to_doc(arc)
                    && doc.uri.as_str() == uri
                {
                    return Some(doc.uri.as_str().to_string());
                }
            }
        }
        if let Ok(maps) = self.feature_maps.read() {
            for (name, map) in maps.iter() {
                if let Some(doc) = feature_map_to_doc(name, map)
                    && doc.uri.as_str() == uri
                {
                    return Some(doc.uri.as_str().to_string());
                }
            }
        }
        if let Ok(skills) = self.skills.read() {
            for skill in skills.iter() {
                if let Some(doc) = skill_to_doc(skill)
                    && doc.uri.as_str() == uri
                {
                    return Some(doc.uri.as_str().to_string());
                }
            }
        }
        None
    }

    fn purge_backing_records_for_uris(&self, target_uris: &HashSet<String>) {
        if target_uris.is_empty() {
            return;
        }
        if let Ok(mut rules) = self.meta_rules.write() {
            rules.retain(|r| {
                meta_rule_to_doc(r).is_none_or(|d| !target_uris.contains(d.uri.as_str()))
            });
        }
        if let Ok(mut l1) = self.l1_records.write() {
            l1.retain(|r| {
                l1_record_to_doc(r).is_none_or(|d| !target_uris.contains(d.uri.as_str()))
            });
        }
        if let Ok(mut l2) = self.l2_records.write() {
            l2.retain(|r| {
                l2_record_to_doc(r).is_none_or(|d| !target_uris.contains(d.uri.as_str()))
            });
        }
        if let Ok(mut sops) = self.sops.write() {
            sops.retain(|s| sop_to_doc(s).is_none_or(|d| !target_uris.contains(d.uri.as_str())));
        }
        if let Ok(mut archives) = self.archives.write() {
            archives.retain(|a| {
                archive_to_doc(a).is_none_or(|d| !target_uris.contains(d.uri.as_str()))
            });
        }
        if let Ok(mut maps) = self.feature_maps.write() {
            maps.retain(|(name, map)| {
                feature_map_to_doc(name, map).is_none_or(|d| !target_uris.contains(d.uri.as_str()))
            });
        }
        if let Ok(mut skills) = self.skills.write() {
            skills
                .retain(|s| skill_to_doc(s).is_none_or(|d| !target_uris.contains(d.uri.as_str())));
        }
    }

    /// Soft-deletes a mounted document by recording a tombstone for `uri` and
    /// incrementally marking its node deleted in any cached HNSW graph.
    ///
    /// Returns `true` if a live document matched `uri` and was tombstoned, or `false` if absent
    /// or already tombstoned.
    pub fn tombstone_document(&self, uri: &str) -> bool {
        let Some(canonical) = self.resolve_canonical_uri(uri) else {
            return false;
        };
        let newly_tombstoned = self
            .tombstoned_uris
            .write()
            .map(|mut g| g.insert(canonical.clone()))
            .unwrap_or(false);
        if !newly_tombstoned {
            return false;
        }
        if let Ok(mut cache) = self.diff_cache.write() {
            cache.remove(&canonical);
        }
        self.mark_buffer_modified();
        self.sync_hnsw_on_delete(&canonical);
        true
    }

    /// Removes a mounted document by purging its backing record, recording an index tombstone
    /// (until compaction), and incrementally marking its HNSW node deleted.
    ///
    /// Returns `true` if a live document was removed, or `false` if not found / already tombstoned.
    pub fn remove_document(&self, uri: &str) -> bool {
        let Some(canonical) = self.resolve_canonical_uri(uri) else {
            return false;
        };
        let was_already_tombstoned = self
            .tombstoned_uris
            .read()
            .map(|g| g.contains(&canonical))
            .unwrap_or(false);

        let mut single = HashSet::new();
        single.insert(canonical.clone());
        self.purge_backing_records_for_uris(&single);

        if was_already_tombstoned {
            return false;
        }

        if let Ok(mut tombstones) = self.tombstoned_uris.write() {
            tombstones.insert(canonical.clone());
        }
        if let Ok(mut cache) = self.diff_cache.write() {
            cache.remove(&canonical);
        }
        self.mark_buffer_modified();
        self.sync_hnsw_on_delete(&canonical);
        true
    }

    /// Removes an L1 or L2 memory record by `id`, recording a tombstone and syncing HNSW.
    pub fn remove_memory_record(&self, id: &str) -> bool {
        let mut target_uri = None;
        if let Ok(l1) = self.l1_records.read()
            && let Some(rec) = l1.iter().find(|r| r.id().as_str() == id)
            && let Some(doc) = l1_record_to_doc(rec)
        {
            target_uri = Some(doc.uri.as_str().to_string());
        }
        if target_uri.is_none()
            && let Ok(l2) = self.l2_records.read()
            && let Some(rec) = l2.iter().find(|r| r.id().as_str() == id)
            && let Some(doc) = l2_record_to_doc(rec)
        {
            target_uri = Some(doc.uri.as_str().to_string());
        }
        target_uri.is_some_and(|u| self.remove_document(&u))
    }

    /// Removes a mounted SOP by `id`, recording a tombstone and syncing HNSW.
    pub fn remove_sop(&self, id: &str) -> bool {
        self.remove_document(&format!("pandora://sops/{id}"))
    }

    /// Removes a mounted session archive by `archive_id`, recording a tombstone and syncing HNSW.
    pub fn remove_archive(&self, archive_id: &str) -> bool {
        self.remove_document(&format!("pandora://archives/{archive_id}"))
    }

    /// Removes a mounted feature map by `name`, recording a tombstone and syncing HNSW.
    pub fn remove_feature_map(&self, name: &str) -> bool {
        self.remove_document(&format!("pandora://features/{name}"))
    }

    /// Removes a mounted skill by `id`, recording a tombstone and syncing HNSW.
    pub fn remove_skill(&self, id: &str) -> bool {
        self.remove_document(&format!("pandora://skills/{id}"))
    }

    /// Compacts all tombstoned documents, strips tombstones from `PVFS` state, compacts the
    /// cached `HnswIndex` with neighbor/entry-point repair and re-packed `doc_id`s, and
    /// refreshes any already-persisted on-disk index files.
    pub fn compact_index(&self) -> Result<VfsCompactionReport, VfsError> {
        let tombstones_snapshot: HashSet<String> = self
            .tombstoned_uris
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?
            .iter()
            .cloned()
            .collect();
        let tombstones_purged = tombstones_snapshot.len();
        let live_before = self.collect_all_documents()?.len();
        let docs_before = live_before.saturating_add(tombstones_purged);

        self.purge_backing_records_for_uris(&tombstones_snapshot);
        if let Ok(mut tombstones) = self.tombstoned_uris.write() {
            tombstones.clear();
        }

        let live_docs = self.collect_all_documents()?;
        let docs_after = live_docs.len();
        let live_uris: HashSet<String> = live_docs
            .iter()
            .map(|d| d.uri.as_str().to_string())
            .collect();

        if let Ok(mut cache) = self.diff_cache.write() {
            cache.retain(|uri, _| live_uris.contains(uri));
        }

        let (bm25, tropical_vectors, doc_vectors) =
            self.build_index_differential_tropical(&live_docs)?;
        let corpus_hash = compute_corpus_hash(&live_docs);
        let compacted_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let new_compaction_count = self.compaction_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.last_compacted_unix
            .store(compacted_at_unix, Ordering::Relaxed);
        self.mark_buffer_modified();

        let mut hnsw_compacted = false;
        let mut hnsw_edges_repaired = 0usize;
        let mut hnsw_nodes_reconnected = 0usize;

        if let Ok(mut hnsw_guard) = self.cached_hnsw_index.write()
            && let Some(arc_hnsw) = hnsw_guard.as_mut()
        {
            let hnsw_mut = Arc::make_mut(arc_hnsw);
            let old_uri_map = self
                .hnsw_uri_to_doc_id
                .read()
                .map(|g| g.clone())
                .unwrap_or_default();

            let mut old_to_new_doc_id = HashMap::with_capacity(live_docs.len());
            let mut missing_docs = Vec::new();
            for (new_idx, doc) in live_docs.iter().enumerate() {
                if let Some(&old_doc_id) = old_uri_map.get(doc.uri.as_str()) {
                    old_to_new_doc_id.insert(old_doc_id, new_idx);
                } else {
                    missing_docs.push((new_idx, doc));
                }
            }

            let hnsw_report = hnsw_mut.compact();
            hnsw_edges_repaired = hnsw_report.edges_repaired;
            hnsw_nodes_reconnected = hnsw_report.nodes_reconnected;
            hnsw_mut.remap_doc_ids(&old_to_new_doc_id);

            let uses_tropical =
                hnsw_mut.nodes().iter().any(|n| n.vector.is_tropical()) || hnsw_mut.is_empty();
            for (new_idx, _doc) in missing_docs {
                let vec = if uses_tropical {
                    HnswVector::Tropical(tropical_vectors[new_idx].1)
                } else {
                    HnswVector::Quantized(doc_vectors[new_idx].1)
                };
                hnsw_mut.upsert(new_idx, vec);
            }
            hnsw_mut.set_corpus_hash(corpus_hash);
            hnsw_compacted = true;
            self.populate_hnsw_doc_mapping(&live_docs);
        }

        let deserialized = DeserializedVfsIndex {
            magic: *VFS_INDEX_MAGIC,
            version: VFS_INDEX_VERSION,
            created_at: compacted_at_unix,
            doc_count: docs_after as u32,
            dense_dim: VFS_DENSE_DIMENSION,
            total_tokens: bm25.total_tokens() as u64,
            vocab_size: bm25.postings().len() as u32,
            corpus_hash,
            docs: live_docs,
            doc_vectors,
            tropical_vectors,
            bm25,
            tombstoned_uris: Vec::new(),
            compaction_count: new_compaction_count,
            last_compacted_unix: compacted_at_unix,
        };
        if let Ok(mut cache) = self.cached_index.write() {
            *cache = Some(Arc::new(deserialized));
        }

        if let Some(p) = self.index_path()
            && p.is_file()
        {
            self.write_index_to_disk_inner(&p)?;
        }
        if hnsw_compacted
            && let Some(hp) = self.hnsw_index_path()
            && hp.is_file()
            && let Some(hnsw_arc) = self.cached_hnsw_index()
        {
            hnsw_arc.save_to_file(&hp).map_err(|e| {
                VfsError::BackendError(format!("failed to save compacted HNSW index: {e}"))
            })?;
        }

        Ok(VfsCompactionReport {
            docs_before,
            docs_after,
            tombstones_purged,
            hnsw_compacted,
            hnsw_edges_repaired,
            hnsw_nodes_reconnected,
            compaction_count: new_compaction_count,
            compacted_at_unix,
        })
    }

    /// Executes threshold-based automated index maintenance, compacting only when tombstones
    /// exceed [`Self::auto_compact_threshold`].
    pub fn run_index_maintenance(&self) -> Result<Option<VfsCompactionReport>, VfsError> {
        if self.should_auto_compact() {
            Ok(Some(self.compact_index()?))
        } else {
            Ok(None)
        }
    }

    pub fn list(&self, uri: &ContextUri) -> Result<Vec<VfsDirectoryEntry>, VfsError> {
        let segments = uri.segments();
        if segments.is_empty() {
            return Ok(vec![
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://meta")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "meta",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://index")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "index",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://facts")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "facts",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://sops")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "sops",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://archives")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "archives",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://features")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "features",
                    VfsNodeKind::Directory,
                    0,
                ),
                VfsDirectoryEntry::new(
                    ContextUri::new("pandora://skills")
                        .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                    "skills",
                    VfsNodeKind::Directory,
                    0,
                ),
            ]);
        }

        let tombstones = self
            .tombstoned_uris
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;

        match segments[0] {
            "meta" => {
                if segments.len() == 1 {
                    let rules = self
                        .meta_rules
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let mut entries = Vec::new();
                    for rule in rules.iter() {
                        let name = rule.id().strip_prefix("meta:").unwrap_or_else(|| rule.id());
                        let entry_uri_str = if rule.id() == "meta:rules" || rule.id() == "rules" {
                            "pandora://meta/rules".to_string()
                        } else {
                            format!("pandora://meta/{name}")
                        };
                        if tombstones.contains(&entry_uri_str) {
                            continue;
                        }
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://meta/{name}"))
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            name.to_owned(),
                            VfsNodeKind::Fragment,
                            rule.content().len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "sops" => {
                if segments.len() == 1 {
                    let sops = self
                        .sops
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let mut entries = Vec::new();
                    for sop in sops.iter() {
                        let entry_uri = format!("pandora://sops/{}", sop.sop.id);
                        if tombstones.contains(&entry_uri) {
                            continue;
                        }
                        let json = serde_json::to_string(&sop.sop).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(entry_uri)
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            sop.sop.id.clone(),
                            VfsNodeKind::Sop,
                            json.len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "archives" => {
                if segments.len() == 1 {
                    let archives = self
                        .archives
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let mut entries = Vec::new();
                    for arc in archives.iter() {
                        let entry_uri = format!("pandora://archives/{}", arc.archive_id);
                        if tombstones.contains(&entry_uri) {
                            continue;
                        }
                        let json = serde_json::to_string(arc).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(entry_uri)
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            arc.archive_id.clone(),
                            VfsNodeKind::Archive,
                            json.len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "features" => {
                if segments.len() == 1 {
                    let maps = self
                        .feature_maps
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let mut entries = Vec::new();
                    for (name, map) in maps.iter() {
                        let entry_uri = format!("pandora://features/{name}");
                        if tombstones.contains(&entry_uri) {
                            continue;
                        }
                        let json = serde_json::to_string(map).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(entry_uri)
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            name.clone(),
                            VfsNodeKind::FeatureMap,
                            json.len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "skills" => {
                if segments.len() == 1 {
                    let skills = self
                        .skills
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let mut entries = Vec::new();
                    for skill in skills.iter() {
                        let entry_uri = format!("pandora://skills/{}", skill.id().as_str());
                        if tombstones.contains(&entry_uri) {
                            continue;
                        }
                        let content = format!(
                            "id: {}\nname: {}\nversion: {}\ndescription: {}\n",
                            skill.id().as_str(),
                            skill.name(),
                            skill.version(),
                            skill.description()
                        );
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(entry_uri)
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            skill.id().as_str().to_owned(),
                            VfsNodeKind::Fragment,
                            content.len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "index" => {
                let records = self
                    .l1_records
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                if segments.len() == 1 {
                    let mut entries = Vec::new();
                    let mut seen_intents = std::collections::HashSet::new();
                    for r in records.iter() {
                        let intent_str = r.intent().as_str();
                        let canonical = format!("pandora://index/{intent_str}/{}", r.id().as_str());
                        if tombstones.contains(&canonical) {
                            continue;
                        }
                        if seen_intents.insert(intent_str) {
                            entries.push(VfsDirectoryEntry::new(
                                ContextUri::new(format!("pandora://index/{intent_str}"))
                                    .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                                intent_str.to_owned(),
                                VfsNodeKind::Directory,
                                0,
                            ));
                        }
                    }
                    Ok(entries)
                } else if segments.len() == 2 {
                    let target_intent = segments[1];
                    let mut entries = Vec::new();
                    for r in records.iter() {
                        if r.intent().as_str() == target_intent {
                            let canonical =
                                format!("pandora://index/{target_intent}/{}", r.id().as_str());
                            if tombstones.contains(&canonical) {
                                continue;
                            }
                            entries.push(VfsDirectoryEntry::new(
                                ContextUri::new(canonical)
                                    .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                                r.id().as_str().to_owned(),
                                VfsNodeKind::Fragment,
                                r.summary().len(),
                            ));
                        }
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "facts" => {
                let records = self
                    .l2_records
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                if segments.len() == 1 {
                    let mut entries = Vec::new();
                    for r in records.iter() {
                        let canonical = format!(
                            "pandora://facts/{}/{}",
                            r.intent().as_str(),
                            r.id().as_str()
                        );
                        if tombstones.contains(&canonical) {
                            continue;
                        }
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://facts/{}", r.id().as_str()))
                                .map_err(|e| VfsError::InvalidUri(e.to_string()))?,
                            r.id().as_str().to_owned(),
                            VfsNodeKind::Fragment,
                            r.summary().len(),
                        ));
                    }
                    Ok(entries)
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            _ => Err(VfsError::NotFound(uri.as_str().to_owned())),
        }
    }

    pub fn resolve(&self, uri: &ContextUri) -> Result<ContextFragment, VfsError> {
        let segments = uri.segments();
        if segments.is_empty() {
            return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
        }
        if self.is_tombstoned(uri.as_str()) {
            return Err(VfsError::NotFound(uri.as_str().to_owned()));
        }

        match segments[0] {
            "meta" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                if segments.len() == 2 {
                    let target = segments[1];
                    let rules = self
                        .meta_rules
                        .read()
                        .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                    let rule = rules
                        .iter()
                        .rev()
                        .find(|r| {
                            let id = r.id();
                            id == target
                                || id.strip_prefix("meta:").is_some_and(|s| s == target)
                                || (target == "rules" && (id == "meta:rules" || id == "rules"))
                        })
                        .or_else(|| {
                            if target == "rules" {
                                rules.last()
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                    if let Some(doc) = meta_rule_to_doc(rule)
                        && self.is_tombstoned(doc.uri.as_str())
                    {
                        return Err(VfsError::NotFound(uri.as_str().to_owned()));
                    }
                    Ok(rule.clone())
                } else {
                    Err(VfsError::NotFound(uri.as_str().to_owned()))
                }
            }
            "sops" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let sop_id = segments[1];
                let sops = self
                    .sops
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let sop = sops
                    .iter()
                    .find(|s| s.sop.id == sop_id)
                    .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                let content = serde_json::to_string_pretty(&sop.sop).unwrap_or_default();
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:sop:{}", sop.sop.id),
                    ContextSource::Retrieved,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    150,
                    content,
                    token_cost,
                    None,
                    ContextOrigin::new("pandora-runtime", format!("vfs:sop:{}", sop.sop.id))
                        .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            "archives" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let archive_id = segments[1];
                let archives = self
                    .archives
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let arc = archives
                    .iter()
                    .find(|a| a.archive_id == archive_id)
                    .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                let content = serde_json::to_string_pretty(arc).unwrap_or_default();
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:archive:{}", arc.archive_id),
                    ContextSource::Retrieved,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    120,
                    content,
                    token_cost,
                    None,
                    ContextOrigin::new(
                        "pandora-runtime",
                        format!("vfs:archive:{}", arc.archive_id),
                    )
                    .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            "features" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let module = segments[1];
                let maps = self
                    .feature_maps
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let (_, map) = maps
                    .iter()
                    .find(|(n, _)| n == module)
                    .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                let content = serde_json::to_string_pretty(map).unwrap_or_default();
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:feature:{module}"),
                    ContextSource::Retrieved,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    130,
                    content,
                    token_cost,
                    None,
                    ContextOrigin::new("pandora-runtime", format!("vfs:feature:{module}"))
                        .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            "skills" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let skill_id = segments[1];
                let skills = self
                    .skills
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let skill = skills
                    .iter()
                    .find(|s| s.id().as_str() == skill_id)
                    .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                let content = format!(
                    "# Skill: {}\nVersion: {}\nName: {}\n\n{}\n\nResources:\n{}",
                    skill.id().as_str(),
                    skill.version(),
                    skill.name(),
                    skill.description(),
                    skill.resources().join("\n")
                );
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:skill:{}", skill.id().as_str()),
                    ContextSource::Retrieved,
                    ContextTrust::Verified,
                    ContextClassification::Internal,
                    140,
                    content,
                    token_cost,
                    None,
                    ContextOrigin::new(
                        "pandora-runtime",
                        format!("vfs:skill:{}", skill.id().as_str()),
                    )
                    .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            "index" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let records = self
                    .l1_records
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let record = if segments.len() == 2 {
                    let key = segments[1];
                    let is_intent = records.iter().any(|r| r.intent().as_str() == key);
                    let is_exact_id = records.iter().any(|r| r.id().as_str() == key);
                    if is_intent && !is_exact_id {
                        return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                    }
                    records.iter().find(|r| r.id().as_str() == key)
                } else {
                    let id = segments[2];
                    records.iter().find(|r| r.id().as_str() == id)
                }
                .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                if let Some(doc) = l1_record_to_doc(record)
                    && self.is_tombstoned(doc.uri.as_str())
                {
                    return Err(VfsError::NotFound(uri.as_str().to_owned()));
                }
                let content = record.summary().to_owned();
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:index:{}", record.id().as_str()),
                    ContextSource::L1Evidence,
                    ContextTrust::Verified,
                    record.classification(),
                    100,
                    content,
                    token_cost,
                    record.expires_at(),
                    ContextOrigin::new("pandora-runtime", record.provenance())
                        .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            "facts" => {
                if segments.len() == 1 {
                    return Err(VfsError::IsADirectory(uri.as_str().to_owned()));
                }
                let topic_or_id = segments[1];
                let records = self
                    .l2_records
                    .read()
                    .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
                let record = records
                    .iter()
                    .find(|r| r.id().as_str() == topic_or_id || r.intent().as_str() == topic_or_id)
                    .ok_or_else(|| VfsError::NotFound(uri.as_str().to_owned()))?;
                if let Some(doc) = l2_record_to_doc(record)
                    && self.is_tombstoned(doc.uri.as_str())
                {
                    return Err(VfsError::NotFound(uri.as_str().to_owned()));
                }
                let content = record.summary().to_owned();
                let token_cost = (content.len() / 4).max(1) as u32;
                ContextFragment::new_with_origin(
                    format!("vfs:facts:{}", record.id().as_str()),
                    ContextSource::Retrieved,
                    ContextTrust::Verified,
                    record.classification(),
                    110,
                    content,
                    token_cost,
                    record.expires_at(),
                    ContextOrigin::new("pandora-runtime", record.provenance())
                        .unwrap_or_else(|_| ContextOrigin::incomplete()),
                )
                .map_err(|e| VfsError::BackendError(e.to_string()))
            }
            _ => Err(VfsError::NotFound(uri.as_str().to_owned())),
        }
    }

    /// Executes a hybrid BM25 and dense similarity search over all mounted virtual resources.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResultItem>, VfsError> {
        let options = ContextSearchOptions::new(limit);
        self.search_with_options(query, &options)
    }

    /// Configures the disk destination for the persistent semantic index.
    pub fn set_index_path(&self, path: impl Into<PathBuf>) {
        if let Ok(mut guard) = self.index_path.write() {
            *guard = Some(path.into());
        }
    }

    /// Returns the currently configured persistent index path.
    pub fn index_path(&self) -> Option<PathBuf> {
        self.index_path.read().ok().and_then(|g| g.clone())
    }

    /// Configures the disk destination for the persistent HNSW graph index.
    pub fn set_hnsw_index_path(&self, path: impl Into<PathBuf>) {
        if let Ok(mut guard) = self.hnsw_index_path.write() {
            *guard = Some(path.into());
        }
    }

    /// Returns the currently configured persistent HNSW index path.
    pub fn hnsw_index_path(&self) -> Option<PathBuf> {
        self.hnsw_index_path.read().ok().and_then(|g| g.clone())
    }

    /// Clears the in-memory cached persistent index and diff cache.
    pub fn clear_cached_index(&self) {
        if let Ok(mut guard) = self.cached_index.write() {
            *guard = None;
        }
        if let Ok(mut cache) = self.diff_cache.write() {
            cache.clear();
        }
        self.clear_cached_hnsw_index();
    }

    /// Clears the in-memory cached HNSW graph index.
    pub fn clear_cached_hnsw_index(&self) {
        if let Ok(mut guard) = self.cached_hnsw_index.write() {
            *guard = None;
        }
        if let Ok(mut docs) = self.hnsw_docs.write() {
            docs.clear();
        }
        if let Ok(mut map) = self.hnsw_uri_to_doc_id.write() {
            map.clear();
        }
    }

    /// Returns the currently cached HNSW index, if loaded.
    pub fn cached_hnsw_index(&self) -> Option<Arc<HnswIndex>> {
        self.cached_hnsw_index.read().ok().and_then(|g| g.clone())
    }

    /// Clears the diff cache only.
    pub fn clear_diff_cache(&self) {
        if let Ok(mut cache) = self.diff_cache.write() {
            cache.clear();
        }
    }

    /// Returns the total diff cache hits and misses since initialization.
    pub fn diff_cache_stats(&self) -> (u64, u64) {
        (
            self.cache_hits.load(std::sync::atomic::Ordering::Relaxed),
            self.cache_misses.load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Resets diff cache statistics.
    pub fn reset_diff_cache_stats(&self) {
        self.cache_hits
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.cache_misses
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns a copy of the current in-memory diff cache.
    pub fn diff_cache(&self) -> HashMap<String, CachedBufferEntry> {
        self.diff_cache
            .read()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// Populates the diff cache from an existing deserialized index.
    pub fn populate_diff_cache_from_index(&self, index: &DeserializedVfsIndex) {
        let mut doc_term_freqs: HashMap<usize, Vec<(String, u32)>> = HashMap::new();
        for (term, postings) in index.bm25.postings() {
            for &(doc_id, tf) in postings {
                doc_term_freqs
                    .entry(doc_id)
                    .or_default()
                    .push((term.clone(), tf));
            }
        }

        if let Ok(mut cache) = self.diff_cache.write() {
            for (idx, doc) in index.docs.iter().enumerate() {
                let fp = compute_doc_fingerprint(doc);
                let vec = index
                    .doc_vectors
                    .get(idx)
                    .map(|(_, v)| *v)
                    .unwrap_or(QuantizedVector {
                        scale: 0.0,
                        data: [0i8; 128],
                    });
                let doc_len = *index.bm25.doc_lengths().get(&idx).unwrap_or(&0);
                let mut tfs = doc_term_freqs.remove(&idx).unwrap_or_default();
                tfs.sort_by(|a, b| a.0.cmp(&b.0));
                let tropical_vec = index
                    .tropical_vectors
                    .get(idx)
                    .map(|(_, tv)| *tv)
                    .unwrap_or_else(|| TropicalProjector::project(&doc.content));
                cache.insert(
                    doc.uri.as_str().to_string(),
                    CachedBufferEntry {
                        uri: doc.uri.clone(),
                        fingerprint: fp,
                        vector: vec,
                        tropical_vector: tropical_vec,
                        doc_length: doc_len,
                        term_frequencies: tfs,
                    },
                );
            }
        }
    }

    /// Executes incremental differential indexing pipeline:
    /// - Evicts deleted documents from `diff_cache`.
    /// - For unchanged documents (fingerprint matches), reuses cached vector, doc_length, and term frequencies.
    /// - For new or modified documents, vectorizes, quantizes, extracts term frequencies, and updates cache.
    /// - Re-accumulates BM25 index from term frequencies without re-tokenizing unchanged documents.
    pub fn build_index_differential(
        &self,
        docs: &[VfsDoc],
    ) -> Result<(SparseBm25Index, Vec<(usize, QuantizedVector)>), VfsError> {
        let current_uris: std::collections::HashSet<String> =
            docs.iter().map(|d| d.uri.as_str().to_string()).collect();

        let mut cache = self
            .diff_cache
            .write()
            .map_err(|_| VfsError::BackendError("poisoned diff_cache lock".into()))?;

        // Evict deleted documents from diff_cache
        cache.retain(|uri, _| current_uris.contains(uri));

        let hasher = DenseFeatureHasher::new();
        let mut doc_vectors = Vec::with_capacity(docs.len());
        let mut bm25 = SparseBm25Index::new();

        for (idx, doc) in docs.iter().enumerate() {
            let fp = compute_doc_fingerprint(doc);
            let uri_str = doc.uri.as_str();

            let is_hit = cache
                .get(uri_str)
                .is_some_and(|entry| entry.fingerprint == fp);

            if is_hit {
                self.cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let entry = cache.get(uri_str).unwrap();
                bm25.add_document_terms(idx, entry.doc_length, &entry.term_frequencies);
                doc_vectors.push((idx, entry.vector));
            } else {
                self.cache_misses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (doc_len, term_freqs) = extract_term_frequencies(&doc.content);
                let f32_vec = hasher.vectorize(&doc.content);
                let qv = QuantizedVector::quantize(&f32_vec);
                let tropical_vec = TropicalProjector::project(&doc.content);

                bm25.add_document_terms(idx, doc_len, &term_freqs);
                doc_vectors.push((idx, qv));

                cache.insert(
                    uri_str.to_string(),
                    CachedBufferEntry {
                        uri: doc.uri.clone(),
                        fingerprint: fp,
                        vector: qv,
                        tropical_vector: tropical_vec,
                        doc_length: doc_len,
                        term_frequencies: term_freqs,
                    },
                );
            }
        }

        Ok((bm25, doc_vectors))
    }

    /// Executes incremental differential indexing including zero-multiplication tropical projections.
    pub fn build_index_differential_tropical(
        &self,
        docs: &[VfsDoc],
    ) -> Result<TropicalDifferentialIndex, VfsError> {
        let (bm25, doc_vectors) = self.build_index_differential(docs)?;
        let cache = self
            .diff_cache
            .read()
            .map_err(|_| VfsError::BackendError("poisoned diff_cache lock".into()))?;
        let mut tropical_vectors = Vec::with_capacity(docs.len());
        for (idx, doc) in docs.iter().enumerate() {
            let tv = cache
                .get(doc.uri.as_str())
                .map(|entry| entry.tropical_vector)
                .unwrap_or_else(|| TropicalProjector::project(&doc.content));
            tropical_vectors.push((idx, tv));
        }
        Ok((bm25, tropical_vectors, doc_vectors))
    }

    /// Collects all live mounted virtual documents across constitutional rules,
    /// L1/L2 memory records, SOPs, session archives, feature maps, and active skills,
    /// excluding any URIs currently tombstoned.
    pub fn collect_all_documents(&self) -> Result<Vec<VfsDoc>, VfsError> {
        let tombstones = self
            .tombstoned_uris
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        let mut docs = Vec::new();

        // 1. Meta rules (Tier L0)
        let rules = self
            .meta_rules
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for rule in rules.iter() {
            if let Some(doc) = meta_rule_to_doc(rule)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 2. L1 records (Tier L1)
        let l1 = self
            .l1_records
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for rec in l1.iter() {
            if let Some(doc) = l1_record_to_doc(rec)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 3. L2 records (Tier L2)
        let l2 = self
            .l2_records
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for rec in l2.iter() {
            if let Some(doc) = l2_record_to_doc(rec)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 4. SOPs (Tier L3)
        let sops = self
            .sops
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for stored in sops.iter() {
            if let Some(doc) = sop_to_doc(stored)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 5. Archives (Tier L4)
        let archives = self
            .archives
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for arc in archives.iter() {
            if let Some(doc) = archive_to_doc(arc)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 6. Feature maps
        let maps = self
            .feature_maps
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for (name, map) in maps.iter() {
            if let Some(doc) = feature_map_to_doc(name, map)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        // 7. Active skills
        let skills = self
            .skills
            .read()
            .map_err(|_| VfsError::BackendError("lock poisoned".into()))?;
        for skill in skills.iter() {
            if let Some(doc) = skill_to_doc(skill)
                && !tombstones.contains(doc.uri.as_str())
            {
                docs.push(doc);
            }
        }

        Ok(docs)
    }

    /// Serializes and writes the current BM25 inverted index and 128-d dense embeddings
    /// atomically to disk at the specified path using differential indexing.
    ///
    /// Automatically triggers [`Self::compact_index`] beforehand if tracked tombstones
    /// exceed [`Self::auto_compact_threshold`].
    pub fn save_index_to_disk(&self, index_path: &Path) -> Result<(), VfsError> {
        if self.should_auto_compact() {
            self.compact_index()?;
        }
        self.write_index_to_disk_inner(index_path)
    }

    fn write_index_to_disk_inner(&self, index_path: &Path) -> Result<(), VfsError> {
        let docs = self.collect_all_documents()?;
        let (bm25, tropical_vectors, doc_vectors) =
            self.build_index_differential_tropical(&docs)?;

        let corpus_hash = compute_corpus_hash(&docs);
        let created_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let tombstones = self.tombstoned_uris();
        let compaction_count = self.compaction_count();
        let last_compacted_unix = self.last_compacted_unix();

        let bytes = serialize_vfs_index_v3_with_tombstones(
            &docs,
            &bm25,
            &doc_vectors,
            &tropical_vectors,
            corpus_hash,
            created_at_unix,
            &tombstones,
            compaction_count,
            last_compacted_unix,
        )?;

        if let Some(parent) = index_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                VfsError::BackendError(format!(
                    "failed to create index directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let mut file = AtomicWriteFile::open(index_path).map_err(|e| {
            VfsError::BackendError(format!(
                "failed to open atomic file at {}: {e}",
                index_path.display()
            ))
        })?;
        file.write_all(&bytes)
            .map_err(|e| VfsError::BackendError(format!("failed to write index bytes: {e}")))?;
        file.commit()
            .map_err(|e| VfsError::BackendError(format!("failed to commit index file: {e}")))?;

        let deserialized = DeserializedVfsIndex {
            magic: *VFS_INDEX_MAGIC,
            version: VFS_INDEX_VERSION,
            created_at: created_at_unix,
            doc_count: docs.len() as u32,
            dense_dim: VFS_DENSE_DIMENSION,
            total_tokens: bm25.total_tokens() as u64,
            vocab_size: bm25.postings().len() as u32,
            corpus_hash,
            docs,
            doc_vectors,
            tropical_vectors,
            bm25,
            tombstoned_uris: tombstones,
            compaction_count,
            last_compacted_unix,
        };
        if let Ok(mut cache) = self.cached_index.write() {
            *cache = Some(Arc::new(deserialized));
        }

        Ok(())
    }

    /// Checks if a persistent index exists at `index_path` and matches the current corpus hash.
    pub fn is_index_fresh(&self, index_path: &Path) -> Result<bool, VfsError> {
        if !index_path.is_file() {
            return Ok(false);
        }
        let stats = read_index_stats(index_path, Some(self))?;
        Ok(stats.checksum_valid && stats.is_fresh)
    }

    /// Loads an existing fresh persistent index from `workspace_dir/.pandora/indices/vfs.idx`,
    /// or builds and persists one if missing or stale. Returns index statistics.
    pub fn load_or_build_index(workspace_dir: &Path) -> Result<VfsIndexStats, VfsError> {
        let vfs = Self::new();
        let feat_map_path = workspace_dir.join(".pandora/feature_map.json");
        if let Ok(bytes) = std::fs::read(&feat_map_path)
            && let Ok(map) = serde_json::from_slice::<FeatureMap>(&bytes)
            && map.validate().is_ok()
        {
            vfs.add_feature_map("workspace", map);
        }

        let index_dir = workspace_dir.join(".pandora/indices");
        std::fs::create_dir_all(&index_dir).map_err(|e| {
            VfsError::BackendError(format!(
                "failed to create indices directory {}: {e}",
                index_dir.display()
            ))
        })?;
        let index_path = index_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        if !vfs.is_index_fresh(&index_path).unwrap_or(false) {
            vfs.save_index_to_disk(&index_path)?;
        }

        read_index_stats(&index_path, Some(&vfs))
    }

    /// Executes a hybrid BM25 and dense similarity search with configurable options.
    /// Checks if a persistent index exists at `.pandora/indices/vfs.idx` and is fresh
    /// (corpus hash match); if so, uses the pre-loaded / cached index for instantaneous
    /// cold-start retrieval (< 5ms). If missing or stale, builds and atomically writes to disk.
    pub fn search_with_options(
        &self,
        query: &str,
        options: &ContextSearchOptions,
    ) -> Result<Vec<SearchResultItem>, VfsError> {
        if query.trim().is_empty() || options.limit == 0 {
            return Ok(Vec::new());
        }

        let all_docs = self.collect_all_documents()?;
        if all_docs.is_empty() {
            return Ok(Vec::new());
        }

        let current_corpus_hash = compute_corpus_hash(&all_docs);
        let index_path = self
            .index_path
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(|| PathBuf::from(".pandora/indices/vfs.idx"));

        let mut active_index: Option<Arc<DeserializedVfsIndex>> = None;

        // 1. Check in-memory cached index
        if let Ok(guard) = self.cached_index.read()
            && let Some(cached) = guard.as_ref()
            && cached.corpus_hash == current_corpus_hash
        {
            active_index = Some(Arc::clone(cached));
        }

        // 2. Check persistent index on disk if not in-memory
        if active_index.is_none()
            && index_path.is_file()
            && let Ok(bytes) = std::fs::read(&index_path)
            && let Ok(deserialized) = deserialize_vfs_index(&bytes)
            && deserialized.corpus_hash == current_corpus_hash
        {
            self.populate_diff_cache_from_index(&deserialized);
            let arc_idx = Arc::new(deserialized);
            if let Ok(mut guard) = self.cached_index.write() {
                *guard = Some(Arc::clone(&arc_idx));
            }
            active_index = Some(arc_idx);
        }

        // 3. If missing or stale, build differentially and atomically write to disk
        let (index_docs, bm25, doc_vectors) = match active_index {
            Some(loaded) => (
                loaded.docs.clone(),
                loaded.bm25.clone(),
                loaded.doc_vectors.clone(),
            ),
            None => {
                self.save_index_to_disk(&index_path)?;
                if let Ok(guard) = self.cached_index.read()
                    && let Some(cached) = guard.as_ref()
                {
                    (
                        cached.docs.clone(),
                        cached.bm25.clone(),
                        cached.doc_vectors.clone(),
                    )
                } else {
                    let (bm25, doc_vectors) = self.build_index_differential(&all_docs)?;
                    (all_docs, bm25, doc_vectors)
                }
            }
        };

        let hasher = DenseFeatureHasher::new();
        let sparse_scores = bm25.score(query);
        let query_vec = hasher.vectorize(query);

        // Route dense scoring through HNSW when available, providing O(log N) logarithmic retrieval
        let dense_scores = {
            let maybe_hnsw: Option<Arc<HnswIndex>> = {
                let in_memory = self.cached_hnsw_index.read().ok().and_then(|guard| {
                    guard.as_ref().and_then(|cached| {
                        if cached.corpus_hash() == &current_corpus_hash {
                            Some(Arc::clone(cached))
                        } else {
                            None
                        }
                    })
                });
                if in_memory.is_some() {
                    in_memory
                } else if self.is_hnsw_index_fresh() {
                    self.load_or_build_hnsw_index().ok()
                } else {
                    None
                }
            };

            if let Some(hnsw_idx) = maybe_hnsw {
                let uses_tropical = hnsw_idx.nodes().iter().any(|n| n.vector.is_tropical());
                let q_hnsw = if uses_tropical {
                    HnswVector::Tropical(TropicalProjector::project(query))
                } else {
                    HnswVector::Quantized(QuantizedVector::quantize(&query_vec))
                };
                let ef = (options.limit * 2).max(64);
                let top_k = (options.limit * 5).max(32).min(index_docs.len());
                let hnsw_res = hnsw_idx.search(&q_hnsw, top_k, ef);
                let hnsw_docs_guard = self.hnsw_docs.read().ok();
                let has_hnsw_map = hnsw_docs_guard.as_ref().is_some_and(|g| !g.is_empty());
                let uri_to_idx: Option<HashMap<String, usize>> = if has_hnsw_map {
                    Some(
                        index_docs
                            .iter()
                            .enumerate()
                            .map(|(i, d)| (d.uri.as_str().to_string(), i))
                            .collect(),
                    )
                } else {
                    None
                };
                hnsw_res
                    .into_iter()
                    .filter_map(|r| {
                        let mapped_id = if has_hnsw_map {
                            let hdoc = hnsw_docs_guard
                                .as_ref()?
                                .get(r.doc_id)
                                .and_then(|opt| opt.as_ref())?;
                            if index_docs.get(r.doc_id).is_some_and(|d| d.uri == hdoc.uri) {
                                r.doc_id
                            } else {
                                *uri_to_idx.as_ref()?.get(hdoc.uri.as_str())?
                            }
                        } else {
                            r.doc_id
                        };
                        Some((mapped_id, r.similarity))
                    })
                    .collect()
            } else {
                hasher.score_quantized_docs(&query_vec, &doc_vectors)
            }
        };

        let ranker = HybridRrfRanker::new();
        let fused = ranker.fuse(&sparse_scores, &dense_scores);

        let mut results = Vec::new();
        for item in fused {
            if item.hybrid_score < options.min_score {
                continue;
            }
            if item.doc_id >= index_docs.len() {
                continue;
            }
            let doc = &index_docs[item.doc_id];

            if let Some(target_tier) = options.tier
                && doc.tier != Some(target_tier)
            {
                continue;
            }
            if let Some(target_intent) = options.intent
                && doc.intent != Some(target_intent)
            {
                continue;
            }

            let snippet = generate_snippet(&doc.content, query);
            results.push(SearchResultItem {
                uri: doc.uri.clone(),
                title: doc.title.clone(),
                snippet,
                sparse_score: item.sparse_score,
                dense_score: item.dense_score,
                hybrid_score: item.hybrid_score,
                tier: doc.tier,
                intent: doc.intent,
            });
            if results.len() >= options.limit {
                break;
            }
        }

        Ok(results)
    }

    /// Executes hybrid BM25 + Tropical Max-Plus semantic projection search across all mounted
    /// L0–L4 memory records and virtual `pandora://` files, with fallback to legacy dense embeddings.
    pub fn search_tropical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, VfsError> {
        let options = ContextSearchOptions::new(limit);
        self.search_tropical_with_options(query, &options)
    }

    /// Executes hybrid BM25 + Tropical Max-Plus semantic projection search with tier and intent filters.
    pub fn search_tropical_with_options(
        &self,
        query: &str,
        options: &ContextSearchOptions,
    ) -> Result<Vec<SearchResultItem>, VfsError> {
        if query.trim().is_empty() || options.limit == 0 {
            return Ok(Vec::new());
        }

        let docs = self.collect_all_documents()?;
        if docs.is_empty() {
            return Ok(Vec::new());
        }

        let (bm25, tropical_vectors, dense_vectors) =
            self.build_index_differential_tropical(&docs)?;
        let sparse_scores = bm25.score(query);
        let query_trop = TropicalProjector::project(query);
        let tropical_scores = TropicalProjector::score_docs(&query_trop, &tropical_vectors);

        let dense_fallback = if tropical_scores.is_empty() {
            let hasher = DenseFeatureHasher::new();
            let query_dense = hasher.vectorize(query);
            hasher.score_quantized_docs(&query_dense, &dense_vectors)
        } else {
            Vec::new()
        };

        let ranker = TropicalHybridRanker::new();
        let fused = ranker.fuse_with_fallback(&sparse_scores, &tropical_scores, &dense_fallback);

        let mut results = Vec::new();
        for item in fused {
            if item.hybrid_score < options.min_score {
                continue;
            }
            if item.doc_id >= docs.len() {
                continue;
            }
            let doc = &docs[item.doc_id];

            if let Some(target_tier) = options.tier
                && doc.tier != Some(target_tier)
            {
                continue;
            }
            if let Some(target_intent) = options.intent
                && doc.intent != Some(target_intent)
            {
                continue;
            }

            let snippet = generate_snippet(&doc.content, query);
            results.push(SearchResultItem {
                uri: doc.uri.clone(),
                title: doc.title.clone(),
                snippet,
                sparse_score: item.sparse_score,
                dense_score: item.dense_score,
                hybrid_score: item.hybrid_score,
                tier: doc.tier,
                intent: doc.intent,
            });
            if results.len() >= options.limit {
                break;
            }
        }

        Ok(results)
    }

    /// Constructs an HNSW graph index using zero-multiplication `TropicalVector` projections
    /// across all mounted ContextVfs documents.
    pub fn build_hnsw_index_tropical(
        &self,
        config: Option<HnswConfig>,
    ) -> Result<HnswIndexStats, VfsError> {
        let docs = self.collect_all_documents()?;
        let (_bm25, tropical_vectors, _dense_vectors) =
            self.build_index_differential_tropical(&docs)?;

        let cfg = config.unwrap_or_default();
        let mut index = HnswIndex::new(cfg);
        let corpus_hash = compute_corpus_hash(&docs);
        index.set_corpus_hash(corpus_hash);

        for (doc_id, tv) in tropical_vectors {
            index.insert(doc_id, HnswVector::Tropical(tv));
        }

        let stats = index.stats();
        let arc_idx = Arc::new(index);
        if let Ok(mut guard) = self.cached_hnsw_index.write() {
            *guard = Some(arc_idx);
        }
        self.populate_hnsw_doc_mapping(&docs);

        Ok(stats)
    }

    /// Constructs an HNSW graph index from all mounted ContextVfs documents.
    /// Vectorizes documents into QuantizedVectors, constructs the multi-layer skip graph,
    /// caches the index in memory, and returns graph statistics.
    pub fn build_hnsw_index(&self, config: Option<HnswConfig>) -> Result<HnswIndexStats, VfsError> {
        let docs = self.collect_all_documents()?;
        let (_bm25, doc_vectors) = self.build_index_differential(&docs)?;

        let cfg = config.unwrap_or_default();
        let mut index = HnswIndex::new(cfg);
        let corpus_hash = compute_corpus_hash(&docs);
        index.set_corpus_hash(corpus_hash);

        for (doc_id, qv) in doc_vectors {
            index.insert(doc_id, HnswVector::Quantized(qv));
        }

        let stats = index.stats();
        let arc_idx = Arc::new(index);
        if let Ok(mut guard) = self.cached_hnsw_index.write() {
            *guard = Some(arc_idx);
        }
        self.populate_hnsw_doc_mapping(&docs);

        Ok(stats)
    }

    /// Serializes the cached HNSW index to disk in binary PVHN v1 format at `path`
    /// (or `.pandora/indices/vfs_hnsw.idx` if unspecified) using safe atomic disk write.
    #[allow(clippy::collapsible_if)]
    pub fn save_hnsw_index(&self, path: Option<&Path>) -> Result<PathBuf, VfsError> {
        if self.should_auto_compact() {
            self.compact_index()?;
        }
        let target_path = match path {
            Some(p) => p.to_path_buf(),
            None => self
                .hnsw_index_path
                .read()
                .ok()
                .and_then(|guard| guard.clone())
                .unwrap_or_else(|| PathBuf::from(VFS_HNSW_INDEX_PATH_DEFAULT)),
        };

        let docs = self.collect_all_documents()?;
        let current_corpus_hash = compute_corpus_hash(&docs);

        let needs_build = match self.cached_hnsw_index.read() {
            Ok(guard) => match guard.as_ref() {
                Some(cached) => cached.corpus_hash() != &current_corpus_hash,
                None => true,
            },
            Err(_) => true,
        };

        if needs_build {
            self.build_hnsw_index_tropical(None)?;
        }

        let index_arc = self
            .cached_hnsw_index
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| VfsError::BackendError("failed to obtain cached HNSW index".into()))?;

        index_arc.save_to_file(&target_path).map_err(|e| {
            VfsError::BackendError(format!("failed to save HNSW index to file: {e}"))
        })?;

        if path.is_some() {
            if let Ok(mut guard) = self.hnsw_index_path.write() {
                *guard = Some(target_path.clone());
            }
        }

        Ok(target_path)
    }

    /// Checks freshness of the HNSW index; if a fresh index file exists on disk
    /// (or fresh index in cache), loads/reuses it; otherwise builds and saves a new one.
    #[allow(clippy::collapsible_if)]
    pub fn load_or_build_hnsw_index(&self) -> Result<Arc<HnswIndex>, VfsError> {
        let docs = self.collect_all_documents()?;
        let current_corpus_hash = compute_corpus_hash(&docs);

        // 1. Check in-memory cache
        if let Ok(guard) = self.cached_hnsw_index.read() {
            if let Some(cached) = guard.as_ref() {
                if cached.corpus_hash() == &current_corpus_hash {
                    return Ok(Arc::clone(cached));
                }
            }
        }

        // 2. Check persistent file on disk
        let index_path = self
            .hnsw_index_path
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(|| PathBuf::from(VFS_HNSW_INDEX_PATH_DEFAULT));

        if index_path.is_file() {
            if let Ok(bytes) = std::fs::read(&index_path) {
                if let Ok(index) = HnswIndex::from_pvhn_bytes(&bytes) {
                    if index.corpus_hash() == &current_corpus_hash {
                        let arc_idx = Arc::new(index);
                        if let Ok(mut guard) = self.cached_hnsw_index.write() {
                            *guard = Some(Arc::clone(&arc_idx));
                        }
                        self.populate_hnsw_doc_mapping(&docs);
                        return Ok(arc_idx);
                    }
                }
            }
        }

        // 3. Stale or missing: build tropical HNSW index and save
        self.build_hnsw_index_tropical(None)?;
        let _ = self.save_hnsw_index(Some(&index_path))?;

        if let Ok(guard) = self.cached_hnsw_index.read() {
            if let Some(cached) = guard.as_ref() {
                return Ok(Arc::clone(cached));
            }
        }

        Err(VfsError::BackendError(
            "failed to load or build HNSW index".into(),
        ))
    }

    /// Executes logarithmic $O(\log N)$ approximate nearest neighbor search over mounted virtual
    /// resources via the HNSW graph index.
    pub fn search_hnsw(
        &self,
        query: &str,
        limit: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResultItem>, VfsError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let docs = self.collect_all_documents()?;
        if docs.is_empty() {
            return Ok(Vec::new());
        }

        let index = self.load_or_build_hnsw_index()?;

        let uses_tropical = index.nodes().iter().any(|n| n.vector.is_tropical());
        let query_vector = if uses_tropical {
            HnswVector::Tropical(TropicalProjector::project(query))
        } else {
            let hasher = DenseFeatureHasher::new();
            let query_f32 = hasher.vectorize(query);
            let query_qv = QuantizedVector::quantize(&query_f32);
            HnswVector::Quantized(query_qv)
        };

        let candidate_limit = if uses_tropical {
            limit.saturating_mul(2).max(limit.saturating_add(4))
        } else {
            limit
        };
        let hnsw_results = index.search(&query_vector, candidate_limit, ef_search);
        let hnsw_docs_guard = self.hnsw_docs.read().ok();
        let has_hnsw_map = hnsw_docs_guard.as_ref().is_some_and(|g| !g.is_empty());

        let mut results = Vec::with_capacity(hnsw_results.len());
        for res in hnsw_results {
            let doc = if has_hnsw_map {
                let Some(d) = hnsw_docs_guard
                    .as_ref()
                    .and_then(|g| g.get(res.doc_id))
                    .and_then(|opt| opt.as_ref())
                else {
                    continue;
                };
                d
            } else {
                let Some(d) = docs.get(res.doc_id) else {
                    continue;
                };
                d
            };
            let snippet = generate_snippet(&doc.content, query);
            let tier_bias = if uses_tropical {
                match doc.tier {
                    Some(MemoryTier::L0) => 0.000004,
                    Some(MemoryTier::L1) => 0.000003,
                    Some(MemoryTier::L2) => 0.000002,
                    Some(MemoryTier::L3) => 0.000001,
                    Some(MemoryTier::L4) | None => 0.0,
                }
            } else {
                0.0
            };
            let weighted_score = (res.similarity * 0.99999 + tier_bias).clamp(0.0, 1.0);
            results.push(SearchResultItem {
                uri: doc.uri.clone(),
                title: doc.title.clone(),
                snippet,
                sparse_score: 0.0,
                dense_score: res.similarity,
                hybrid_score: weighted_score,
                tier: doc.tier,
                intent: doc.intent,
            });
        }
        if uses_tropical {
            results.sort_by(|a, b| {
                b.hybrid_score
                    .partial_cmp(&a.hybrid_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        b.dense_score
                            .partial_cmp(&a.dense_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            results.truncate(limit);
        }

        Ok(results)
    }

    /// Returns structural graph statistics of the current HNSW index.
    pub fn hnsw_index_stats(&self) -> Result<HnswIndexStats, VfsError> {
        let index = self.load_or_build_hnsw_index()?;
        Ok(index.stats())
    }

    /// Checks if the persistent HNSW index file exists on disk, is valid,
    /// and has matching cryptographic corpus hash with current VFS documents.
    pub fn is_hnsw_index_fresh(&self) -> bool {
        let index_path = match self.hnsw_index_path.read() {
            Ok(guard) => match guard.as_ref() {
                Some(p) => p.clone(),
                None => PathBuf::from(VFS_HNSW_INDEX_PATH_DEFAULT),
            },
            Err(_) => return false,
        };

        if !index_path.is_file() {
            return false;
        }

        // Verify integrity and corpus hash match
        let Ok(docs) = self.collect_all_documents() else {
            return false;
        };
        let current_corpus_hash = compute_corpus_hash(&docs);

        let Ok(bytes) = std::fs::read(&index_path) else {
            return false;
        };
        if bytes.len() < PVHN_HEADER_SIZE + PVHN_TRAILER_SIZE {
            return false;
        }

        let payload_len = bytes.len() - PVHN_TRAILER_SIZE;
        let computed = Sha256::digest(&bytes[..payload_len]);
        if computed.as_slice() != &bytes[payload_len..] {
            return false;
        }

        let file_corpus_hash = &bytes[46..78];
        file_corpus_hash == current_corpus_hash
    }
}

fn tier_to_u8(tier: Option<MemoryTier>) -> u8 {
    match tier {
        None => 0,
        Some(MemoryTier::L0) => 1,
        Some(MemoryTier::L1) => 2,
        Some(MemoryTier::L2) => 3,
        Some(MemoryTier::L3) => 4,
        Some(MemoryTier::L4) => 5,
    }
}

fn u8_to_tier(val: u8) -> Option<MemoryTier> {
    match val {
        1 => Some(MemoryTier::L0),
        2 => Some(MemoryTier::L1),
        3 => Some(MemoryTier::L2),
        4 => Some(MemoryTier::L3),
        5 => Some(MemoryTier::L4),
        _ => None,
    }
}

fn intent_to_u8(intent: Option<MemoryIntent>) -> u8 {
    match intent {
        None => 0,
        Some(MemoryIntent::Repo) => 1,
        Some(MemoryIntent::Procedure) => 2,
        Some(MemoryIntent::Coding) => 3,
        Some(MemoryIntent::User) => 4,
    }
}

fn u8_to_intent(val: u8) -> Option<MemoryIntent> {
    match val {
        1 => Some(MemoryIntent::Repo),
        2 => Some(MemoryIntent::Procedure),
        3 => Some(MemoryIntent::Coding),
        4 => Some(MemoryIntent::User),
        _ => None,
    }
}

/// Computes a deterministic SHA-256 fingerprint over all mounted documents.
pub fn compute_corpus_hash(docs: &[VfsDoc]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for doc in docs {
        let uri_bytes = doc.uri.as_str().as_bytes();
        hasher.update((uri_bytes.len() as u32).to_le_bytes());
        hasher.update(uri_bytes);

        let content_bytes = doc.content.as_bytes();
        hasher.update((content_bytes.len() as u32).to_le_bytes());
        hasher.update(content_bytes);

        hasher.update([tier_to_u8(doc.tier)]);
        hasher.update([intent_to_u8(doc.intent)]);
    }
    hasher.finalize().into()
}

/// Serializes documents, BM25 postings, and Int8 quantized dense vectors into the pure-Rust
/// binary layout (Version 2) with SHA-256 integrity trailer.
pub fn serialize_vfs_index(
    docs: &[VfsDoc],
    bm25: &SparseBm25Index,
    doc_vectors: &[(usize, QuantizedVector)],
    corpus_hash: [u8; 32],
    created_at_unix: u64,
) -> Result<Vec<u8>, VfsError> {
    serialize_vfs_index_with_version(
        VFS_INDEX_VERSION_V2,
        docs,
        bm25,
        doc_vectors,
        &[],
        corpus_hash,
        created_at_unix,
        &[],
        0,
        0,
    )
}

/// Serializes documents, BM25 postings, Int8 quantized vectors, and 128-d `TropicalVector`
/// projections into the `PVFS` Version 3 binary layout with SHA-256 integrity trailer.
pub fn serialize_vfs_index_v3(
    docs: &[VfsDoc],
    bm25: &SparseBm25Index,
    doc_vectors: &[(usize, QuantizedVector)],
    tropical_vectors: &[(usize, TropicalVector)],
    corpus_hash: [u8; 32],
    created_at_unix: u64,
) -> Result<Vec<u8>, VfsError> {
    serialize_vfs_index_with_version(
        VFS_INDEX_VERSION_V3,
        docs,
        bm25,
        doc_vectors,
        tropical_vectors,
        corpus_hash,
        created_at_unix,
        &[],
        0,
        0,
    )
}

/// Serializes a `PVFS` Version 3 index along with optional tombstone & compaction metadata,
/// preserving byte-level compatibility with legacy v3 payloads when no tombstones/compactions exist.
#[allow(clippy::too_many_arguments)]
pub fn serialize_vfs_index_v3_with_tombstones(
    docs: &[VfsDoc],
    bm25: &SparseBm25Index,
    doc_vectors: &[(usize, QuantizedVector)],
    tropical_vectors: &[(usize, TropicalVector)],
    corpus_hash: [u8; 32],
    created_at_unix: u64,
    tombstoned_uris: &[String],
    compaction_count: u32,
    last_compacted_unix: u64,
) -> Result<Vec<u8>, VfsError> {
    serialize_vfs_index_with_version(
        VFS_INDEX_VERSION_V3,
        docs,
        bm25,
        doc_vectors,
        tropical_vectors,
        corpus_hash,
        created_at_unix,
        tombstoned_uris,
        compaction_count,
        last_compacted_unix,
    )
}

#[allow(clippy::too_many_arguments)]
fn serialize_vfs_index_with_version(
    version: u32,
    docs: &[VfsDoc],
    bm25: &SparseBm25Index,
    doc_vectors: &[(usize, QuantizedVector)],
    tropical_vectors: &[(usize, TropicalVector)],
    corpus_hash: [u8; 32],
    created_at_unix: u64,
    tombstoned_uris: &[String],
    compaction_count: u32,
    last_compacted_unix: u64,
) -> Result<Vec<u8>, VfsError> {
    let mut buffer = Vec::new();

    // 1. Header (68 bytes)
    buffer.extend_from_slice(VFS_INDEX_MAGIC);
    buffer.extend_from_slice(&version.to_le_bytes());
    buffer.extend_from_slice(&created_at_unix.to_le_bytes());
    let doc_count = docs.len() as u32;
    buffer.extend_from_slice(&doc_count.to_le_bytes());
    buffer.extend_from_slice(&VFS_DENSE_DIMENSION.to_le_bytes());
    buffer.extend_from_slice(&(bm25.total_tokens() as u64).to_le_bytes());

    let mut terms: Vec<_> = bm25.postings().keys().collect();
    terms.sort();
    let vocab_size = terms.len() as u32;
    buffer.extend_from_slice(&vocab_size.to_le_bytes());
    buffer.extend_from_slice(&corpus_hash);

    debug_assert_eq!(buffer.len(), VFS_INDEX_HEADER_SIZE);

    // 2. Document Metadata & Quantized Vector Table (132 bytes per doc)
    for (idx, doc) in docs.iter().enumerate() {
        let doc_id = idx as u32;
        buffer.extend_from_slice(&doc_id.to_le_bytes());

        let uri_bytes = doc.uri.as_str().as_bytes();
        if uri_bytes.len() > u16::MAX as usize {
            return Err(VfsError::BackendError(
                "URI exceeds maximum u16 length".into(),
            ));
        }
        buffer.extend_from_slice(&(uri_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(uri_bytes);

        let title_bytes = doc.title.as_bytes();
        if title_bytes.len() > u16::MAX as usize {
            return Err(VfsError::BackendError(
                "title exceeds maximum u16 length".into(),
            ));
        }
        buffer.extend_from_slice(&(title_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(title_bytes);

        let content_bytes = doc.content.as_bytes();
        if content_bytes.len() > u32::MAX as usize {
            return Err(VfsError::BackendError(
                "content exceeds maximum u32 length".into(),
            ));
        }
        buffer.extend_from_slice(&(content_bytes.len() as u32).to_le_bytes());
        buffer.extend_from_slice(content_bytes);

        buffer.push(tier_to_u8(doc.tier));
        buffer.push(intent_to_u8(doc.intent));

        let doc_length = bm25.doc_lengths().get(&idx).copied().unwrap_or(0) as u32;
        buffer.extend_from_slice(&doc_length.to_le_bytes());

        // Quantized vector: 4 bytes scale + 128 bytes data = 132 bytes (VFS_QUANTIZED_VECTOR_SIZE)
        let vec = doc_vectors
            .get(idx)
            .map(|(_, v)| *v)
            .unwrap_or(QuantizedVector {
                scale: 0.0,
                data: [0i8; 128],
            });
        buffer.extend_from_slice(&vec.scale.to_le_bytes());
        for b in &vec.data {
            buffer.push(*b as u8);
        }
    }

    // 3. Sparse BM25 Inverted Postings Table
    for term in terms {
        let term_bytes = term.as_bytes();
        if term_bytes.len() > u16::MAX as usize {
            return Err(VfsError::BackendError(
                "term exceeds maximum u16 length".into(),
            ));
        }
        buffer.extend_from_slice(&(term_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(term_bytes);

        let mut postings = bm25.postings().get(term).cloned().unwrap_or_default();
        postings.sort_by_key(|&(doc_id, _)| doc_id);
        let postings_count = postings.len() as u32;
        buffer.extend_from_slice(&postings_count.to_le_bytes());

        for (doc_id, tf) in postings {
            buffer.extend_from_slice(&(doc_id as u32).to_le_bytes());
            buffer.extend_from_slice(&tf.to_le_bytes());
        }
    }

    // 3b. Version 3 Tropical Vector Table (512 bytes per doc)
    if version == VFS_INDEX_VERSION_V3 {
        for (idx, doc) in docs.iter().enumerate() {
            let tv = tropical_vectors
                .get(idx)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| TropicalProjector::project(&doc.content));
            buffer.extend_from_slice(&tv.to_bytes());
        }

        // 3c. Optional Version 3 Tombstone & Compaction Extension Block
        if !tombstoned_uris.is_empty() || compaction_count > 0 || last_compacted_unix > 0 {
            buffer.extend_from_slice(PVFS_TOMBSTONE_MAGIC);
            buffer.extend_from_slice(&(tombstoned_uris.len() as u32).to_le_bytes());
            for uri in tombstoned_uris {
                let uri_bytes = uri.as_bytes();
                if uri_bytes.len() > u16::MAX as usize {
                    return Err(VfsError::BackendError(
                        "tombstone URI exceeds maximum u16 length".into(),
                    ));
                }
                buffer.extend_from_slice(&(uri_bytes.len() as u16).to_le_bytes());
                buffer.extend_from_slice(uri_bytes);
            }
            buffer.extend_from_slice(&compaction_count.to_le_bytes());
            buffer.extend_from_slice(&last_compacted_unix.to_le_bytes());
        }
    }

    // 4. Trailer (32 bytes): SHA-256 checksum over byte 0..end of payload
    let checksum = Sha256::digest(&buffer);
    buffer.extend_from_slice(&checksum);

    Ok(buffer)
}

/// Deserializes a binary VFS index from byte slice, validating the 32-byte SHA-256 trailer checksum.
/// Supports Version 1 (512 bytes per doc), Version 2 (132 bytes per doc), and Version 3 (Quantized + Tropical,
/// with optional backward-compatible `TOMB` tombstone & compaction metadata block).
pub fn deserialize_vfs_index(bytes: &[u8]) -> Result<DeserializedVfsIndex, VfsError> {
    if bytes.len() < VFS_INDEX_HEADER_SIZE + VFS_INDEX_TRAILER_SIZE {
        return Err(VfsError::BackendError(
            "index file too short for header and trailer".into(),
        ));
    }

    let payload_len = bytes.len() - VFS_INDEX_TRAILER_SIZE;
    let payload = &bytes[..payload_len];
    let trailer_checksum = &bytes[payload_len..];

    let computed_hash = Sha256::digest(payload);
    if computed_hash.as_slice() != trailer_checksum {
        return Err(VfsError::BackendError(
            "index integrity verification failed: SHA-256 checksum mismatch".into(),
        ));
    }

    let mut cursor = 0;
    let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
    cursor += 4;
    if &magic != VFS_INDEX_MAGIC {
        return Err(VfsError::BackendError(
            "invalid index magic: expected PVFS".into(),
        ));
    }

    let version = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
    cursor += 4;
    if version != VFS_INDEX_VERSION_V1
        && version != VFS_INDEX_VERSION_V2
        && version != VFS_INDEX_VERSION_V3
    {
        return Err(VfsError::BackendError(format!(
            "unsupported index version: {version}"
        )));
    }

    let created_at = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    let doc_count = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
    cursor += 4;

    let dense_dim = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
    cursor += 4;
    if dense_dim != VFS_DENSE_DIMENSION {
        return Err(VfsError::BackendError(format!(
            "unexpected dense dimension {dense_dim}, expected {VFS_DENSE_DIMENSION}"
        )));
    }

    let total_tokens = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    let vocab_size = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
    cursor += 4;

    let mut corpus_hash = [0u8; 32];
    corpus_hash.copy_from_slice(&bytes[cursor..cursor + 32]);
    cursor += 32;

    debug_assert_eq!(cursor, VFS_INDEX_HEADER_SIZE);

    let mut docs = Vec::with_capacity(doc_count as usize);
    let mut doc_vectors = Vec::with_capacity(doc_count as usize);
    let mut doc_lengths = HashMap::with_capacity(doc_count as usize);

    for _ in 0..doc_count {
        if cursor + 4 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading doc_id".into(),
            ));
        }
        let doc_id = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;

        if cursor + 2 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading uri_len".into(),
            ));
        }
        let uri_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if cursor + uri_len > payload_len {
            return Err(VfsError::BackendError("unexpected EOF reading uri".into()));
        }
        let uri_str = std::str::from_utf8(&bytes[cursor..cursor + uri_len])
            .map_err(|e| VfsError::BackendError(format!("invalid UTF-8 in URI: {e}")))?;
        cursor += uri_len;
        let uri = ContextUri::new(uri_str)
            .map_err(|e| VfsError::BackendError(format!("invalid ContextUri: {e}")))?;

        if cursor + 2 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading title_len".into(),
            ));
        }
        let title_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if cursor + title_len > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading title".into(),
            ));
        }
        let title = std::str::from_utf8(&bytes[cursor..cursor + title_len])
            .map_err(|e| VfsError::BackendError(format!("invalid UTF-8 in title: {e}")))?
            .to_string();
        cursor += title_len;

        if cursor + 4 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading content_len".into(),
            ));
        }
        let content_len =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        if cursor + content_len > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading content".into(),
            ));
        }
        let content = std::str::from_utf8(&bytes[cursor..cursor + content_len])
            .map_err(|e| VfsError::BackendError(format!("invalid UTF-8 in content: {e}")))?
            .to_string();
        cursor += content_len;

        if cursor + 2 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading tier and intent".into(),
            ));
        }
        let tier = u8_to_tier(bytes[cursor]);
        cursor += 1;
        let intent = u8_to_intent(bytes[cursor]);
        cursor += 1;

        if cursor + 4 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading doc_length".into(),
            ));
        }
        let doc_length = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;

        let quantized_vec = if version == VFS_INDEX_VERSION_V1 {
            const VECTOR_BYTES: usize = 128 * 4;
            if cursor + VECTOR_BYTES > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading dense vector (v1)".into(),
                ));
            }
            let mut float_vec = [0.0f32; 128];
            for (i, elem) in float_vec.iter_mut().enumerate() {
                let offset = cursor + i * 4;
                *elem = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            }
            cursor += VECTOR_BYTES;
            QuantizedVector::quantize(&float_vec)
        } else {
            if cursor + VFS_QUANTIZED_VECTOR_SIZE > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading quantized vector (v2)".into(),
                ));
            }
            let scale = f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let mut data = [0i8; 128];
            for (i, byte) in bytes[cursor..cursor + 128].iter().enumerate() {
                data[i] = *byte as i8;
            }
            cursor += 128;
            QuantizedVector { scale, data }
        };

        doc_lengths.insert(doc_id, doc_length);
        docs.push(VfsDoc {
            uri,
            title,
            content,
            tier,
            intent,
        });
        doc_vectors.push((doc_id, quantized_vec));
    }

    let mut postings: HashMap<String, Vec<(usize, u32)>> =
        HashMap::with_capacity(vocab_size as usize);
    for _ in 0..vocab_size {
        if cursor + 2 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading term_len".into(),
            ));
        }
        let term_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if cursor + term_len > payload_len {
            return Err(VfsError::BackendError("unexpected EOF reading term".into()));
        }
        let term = std::str::from_utf8(&bytes[cursor..cursor + term_len])
            .map_err(|e| VfsError::BackendError(format!("invalid UTF-8 in term: {e}")))?
            .to_string();
        cursor += term_len;

        if cursor + 4 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading postings_count".into(),
            ));
        }
        let postings_count =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;

        let mut term_postings = Vec::with_capacity(postings_count);
        for _ in 0..postings_count {
            if cursor + 8 > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading posting".into(),
                ));
            }
            let post_doc_id =
                u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
            let tf = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            term_postings.push((post_doc_id, tf));
        }
        postings.insert(term, term_postings);
    }

    let tropical_vectors = if version == VFS_INDEX_VERSION_V3 {
        let mut tvecs = Vec::with_capacity(doc_count as usize);
        const TROP_BYTES: usize = crate::semantic_indexer::TROPICAL_DIMENSION * 4;
        for doc_id in 0..(doc_count as usize) {
            if cursor + TROP_BYTES > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading tropical vector (v3)".into(),
                ));
            }
            let tv = TropicalVector::from_bytes(&bytes[cursor..cursor + TROP_BYTES])
                .map_err(VfsError::BackendError)?;
            cursor += TROP_BYTES;
            tvecs.push((doc_id, tv));
        }
        tvecs
    } else {
        doc_vectors
            .iter()
            .map(|(doc_id, qv)| (*doc_id, TropicalVector::from_quantized_fallback(qv)))
            .collect()
    };

    let mut tombstoned_uris = Vec::new();
    let mut compaction_count = 0u32;
    let mut last_compacted_unix = 0u64;

    if version == VFS_INDEX_VERSION_V3
        && cursor + 4 <= payload_len
        && &bytes[cursor..cursor + 4] == PVFS_TOMBSTONE_MAGIC
    {
        cursor += 4;
        if cursor + 4 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading tombstone_count".into(),
            ));
        }
        let tomb_count = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        tombstoned_uris.reserve(tomb_count);
        for _ in 0..tomb_count {
            if cursor + 2 > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading tombstone URI length".into(),
                ));
            }
            let uri_len =
                u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2;
            if cursor + uri_len > payload_len {
                return Err(VfsError::BackendError(
                    "unexpected EOF reading tombstone URI".into(),
                ));
            }
            let uri = std::str::from_utf8(&bytes[cursor..cursor + uri_len])
                .map_err(|e| {
                    VfsError::BackendError(format!("invalid UTF-8 in tombstone URI: {e}"))
                })?
                .to_string();
            cursor += uri_len;
            tombstoned_uris.push(uri);
        }
        if cursor + 12 > payload_len {
            return Err(VfsError::BackendError(
                "unexpected EOF reading compaction metadata".into(),
            ));
        }
        compaction_count = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        last_compacted_unix = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
        cursor += 8;
    }

    if cursor != payload_len {
        return Err(VfsError::BackendError(format!(
            "index parsing incomplete: cursor at {cursor}, expected {payload_len}"
        )));
    }

    let bm25 = SparseBm25Index::from_parts(
        postings,
        doc_lengths,
        doc_count as usize,
        total_tokens as usize,
    );

    Ok(DeserializedVfsIndex {
        magic,
        version,
        created_at,
        doc_count,
        dense_dim,
        total_tokens,
        vocab_size,
        corpus_hash,
        docs,
        doc_vectors,
        tropical_vectors,
        bm25,
        tombstoned_uris,
        compaction_count,
        last_compacted_unix,
    })
}

/// Reads index metadata from disk and computes integrity validation status.
pub fn read_index_stats(
    index_path: &Path,
    vfs: Option<&ContextVfs>,
) -> Result<VfsIndexStats, VfsError> {
    let metadata = std::fs::metadata(index_path).map_err(|e| {
        VfsError::NotFound(format!(
            "index file not found at {}: {e}",
            index_path.display()
        ))
    })?;
    let size_bytes = metadata.len();

    let bytes = std::fs::read(index_path)
        .map_err(|e| VfsError::BackendError(format!("failed to read index file: {e}")))?;

    if bytes.len() < VFS_INDEX_HEADER_SIZE + VFS_INDEX_TRAILER_SIZE {
        return Err(VfsError::BackendError("index file is truncated".into()));
    }

    let payload_len = bytes.len() - VFS_INDEX_TRAILER_SIZE;
    let computed_hash = Sha256::digest(&bytes[..payload_len]);
    let checksum_valid = computed_hash.as_slice() == &bytes[payload_len..];

    if &bytes[0..4] != VFS_INDEX_MAGIC {
        return Err(VfsError::BackendError("invalid index magic".into()));
    }

    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let created_at_unix = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let doc_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let dense_dimension = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    let total_tokens = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
    let vocab_size = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(&bytes[36..68]);
    let corpus_hash = hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let is_fresh = if let Some(vfs) = vfs {
        if checksum_valid {
            let docs = vfs.collect_all_documents()?;
            let current_hash = compute_corpus_hash(&docs);
            current_hash == hash_bytes
        } else {
            false
        }
    } else {
        checksum_valid
    };

    let (vector_bytes, vector_bytes_saved) =
        if version == VFS_INDEX_VERSION_V2 || version == VFS_INDEX_VERSION_V3 {
            let actual = (doc_count as u64) * (VFS_QUANTIZED_VECTOR_SIZE as u64);
            let baseline = (doc_count as u64) * (dense_dimension as u64 * 4);
            let saved = baseline.saturating_sub(actual);
            (actual, saved)
        } else {
            let actual = (doc_count as u64) * (dense_dimension as u64 * 4);
            (actual, 0)
        };

    let (cache_hits, cache_misses) = if let Some(vfs) = vfs {
        vfs.diff_cache_stats()
    } else {
        (0, 0)
    };

    let (tombstone_count, compaction_count, last_compacted_unix) =
        if checksum_valid && let Ok(parsed) = deserialize_vfs_index(&bytes) {
            (
                parsed.tombstoned_uris.len() as u32,
                parsed.compaction_count,
                parsed.last_compacted_unix,
            )
        } else {
            (0, 0, 0)
        };

    let total_slots = (doc_count as u64).saturating_add(tombstone_count as u64);
    let tombstone_ratio = if total_slots == 0 {
        0.0
    } else {
        tombstone_count as f64 / total_slots as f64
    };

    Ok(VfsIndexStats {
        path: index_path.to_path_buf(),
        size_bytes,
        doc_count,
        vocab_size,
        dense_dimension,
        total_tokens,
        created_at_unix,
        corpus_hash,
        checksum_valid,
        is_fresh,
        version,
        vector_bytes,
        vector_bytes_saved,
        cache_hits,
        cache_misses,
        tombstone_count,
        tombstone_ratio,
        compaction_count,
        last_compacted_unix,
    })
}

fn meta_rule_to_doc(rule: &ContextFragment) -> Option<VfsDoc> {
    let uri_str = if rule.id() == "meta:rules" || rule.id() == "rules" {
        "pandora://meta/rules".to_string()
    } else {
        format!("pandora://meta/{}", rule.id().trim_start_matches("meta:"))
    };
    ContextUri::new(uri_str).ok().map(|uri| VfsDoc {
        uri,
        title: rule.id().to_string(),
        content: rule.content().to_string(),
        tier: Some(MemoryTier::L0),
        intent: None,
    })
}

fn l1_record_to_doc(rec: &MemoryRecord) -> Option<VfsDoc> {
    let intent_str = rec.intent().as_str();
    ContextUri::new(format!(
        "pandora://index/{intent_str}/{}",
        rec.id().as_str()
    ))
    .ok()
    .map(|uri| VfsDoc {
        uri,
        title: rec.id().as_str().to_string(),
        content: format!("{} {}", rec.id().as_str(), rec.summary()),
        tier: Some(MemoryTier::L1),
        intent: Some(rec.intent()),
    })
}

fn l2_record_to_doc(rec: &MemoryRecord) -> Option<VfsDoc> {
    let intent_str = rec.intent().as_str();
    ContextUri::new(format!(
        "pandora://facts/{intent_str}/{}",
        rec.id().as_str()
    ))
    .ok()
    .map(|uri| VfsDoc {
        uri,
        title: rec.id().as_str().to_string(),
        content: format!("{} {}", rec.id().as_str(), rec.summary()),
        tier: Some(MemoryTier::L2),
        intent: Some(rec.intent()),
    })
}

fn sop_to_doc(stored: &StoredSop) -> Option<VfsDoc> {
    ContextUri::new(format!("pandora://sops/{}", stored.sop.id))
        .ok()
        .map(|uri| {
            let steps_text = stored
                .sop
                .steps
                .iter()
                .map(|s| format!("{} {}", s.name, s.command))
                .collect::<Vec<_>>()
                .join(" ");
            let verif = stored
                .sop
                .verification_command
                .as_deref()
                .unwrap_or_default();
            VfsDoc {
                uri,
                title: stored.sop.name.clone(),
                content: format!(
                    "{} {} {} {}",
                    stored.sop.id, stored.sop.name, steps_text, verif
                ),
                tier: Some(MemoryTier::L3),
                intent: Some(stored.sop.trigger_intent),
            }
        })
}

fn archive_to_doc(arc: &SessionArchiveRecord) -> Option<VfsDoc> {
    ContextUri::new(format!("pandora://archives/{}", arc.archive_id))
        .ok()
        .map(|uri| VfsDoc {
            uri,
            title: arc.archive_id.clone(),
            content: format!("{} {}", arc.archive_id, arc.summary),
            tier: Some(MemoryTier::L4),
            intent: None,
        })
}

fn feature_map_to_doc(name: &str, map: &FeatureMap) -> Option<VfsDoc> {
    ContextUri::new(format!("pandora://features/{name}"))
        .ok()
        .map(|uri| {
            let map_json = serde_json::to_string(map).unwrap_or_default();
            VfsDoc {
                uri,
                title: format!("Feature Map: {name}"),
                content: format!("{name} {map_json}"),
                tier: None,
                intent: None,
            }
        })
}

fn skill_to_doc(skill: &SkillManifest) -> Option<VfsDoc> {
    ContextUri::new(format!("pandora://skills/{}", skill.id().as_str()))
        .ok()
        .map(|uri| VfsDoc {
            uri,
            title: skill.name().to_string(),
            content: format!(
                "{} {} {} {}",
                skill.id().as_str(),
                skill.name(),
                skill.description(),
                skill.resources().join(" ")
            ),
            tier: None,
            intent: None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{MemoryIntent, MemoryScope, SessionId, SopDefinition, SopStep, Timestamp};

    #[test]
    fn vfs_resolves_default_meta_rules() {
        let vfs = ContextVfs::new();
        let uri = ContextUri::new("pandora://meta/rules").unwrap();
        let fragment = vfs.resolve(&uri).unwrap();
        assert!(fragment.content().contains("Constitutional Rules"));
        assert_eq!(fragment.source(), ContextSource::Constitutional);
        assert_eq!(fragment.trust(), ContextTrust::Constitutional);
    }

    #[test]
    fn vfs_lists_root_and_subdirectories() {
        let vfs = ContextVfs::new();
        let root = ContextUri::new("pandora://").unwrap();
        let entries = vfs.list(&root).unwrap();
        assert_eq!(entries.len(), 7);
        assert!(entries.iter().any(|e| e.name == "meta"));
        assert!(entries.iter().any(|e| e.name == "sops"));
        assert!(entries.iter().any(|e| e.name == "archives"));

        let meta_dir = ContextUri::new("pandora://meta").unwrap();
        let meta_entries = vfs.list(&meta_dir).unwrap();
        assert_eq!(meta_entries.len(), 1);
        assert_eq!(meta_entries[0].name, "rules");
        assert_eq!(meta_entries[0].kind, VfsNodeKind::Fragment);
    }

    #[test]
    fn vfs_adds_and_resolves_sops() {
        let vfs = ContextVfs::new();
        let step = SopStep::new("s1", "build", "cargo build", vec![]).unwrap();
        let sop_def = SopDefinition::new(
            "sop-deploy",
            "Deploy SOP",
            MemoryIntent::Procedure,
            vec![step],
            Some("cargo test".into()),
        )
        .unwrap();
        let stored = StoredSop::new(
            sop_def,
            TenantId::new("t1").unwrap(),
            WorkspaceId::new("w1").unwrap(),
            1,
            Timestamp::from_unix_seconds(100),
            Timestamp::from_unix_seconds(100),
        );
        vfs.add_sop(stored);

        let list_uri = ContextUri::new("pandora://sops").unwrap();
        let entries = vfs.list(&list_uri).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "sop-deploy");
        assert_eq!(entries[0].kind, VfsNodeKind::Sop);

        let resolve_uri = ContextUri::new("pandora://sops/sop-deploy").unwrap();
        let frag = vfs.resolve(&resolve_uri).unwrap();
        assert!(frag.content().contains("Deploy SOP"));

        let dir_resolve = vfs.resolve(&list_uri);
        assert!(matches!(dir_resolve, Err(VfsError::IsADirectory(_))));
    }

    #[test]
    fn vfs_adds_and_resolves_memory_records() {
        let vfs = ContextVfs::new();
        let scope = MemoryScope::new(
            TenantId::new("t1").unwrap(),
            WorkspaceId::new("w1").unwrap(),
            pandora_types::ids::SessionId::new("s1").unwrap(),
            "provider-a",
        )
        .unwrap();
        let rec = MemoryRecord::new_l1(
            "mem-1",
            pandora_types::MemoryKind::ExecutionEvidence,
            scope,
            "architecture insight",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(100),
            "provenance-a",
        )
        .unwrap()
        .with_intent(MemoryIntent::Repo);
        vfs.add_memory_record(rec);

        let index_uri = ContextUri::new("pandora://index").unwrap();
        let index_entries = vfs.list(&index_uri).unwrap();
        assert_eq!(index_entries.len(), 1);
        assert_eq!(index_entries[0].name, "repo");

        let repo_uri = ContextUri::new("pandora://index/repo").unwrap();
        let repo_entries = vfs.list(&repo_uri).unwrap();
        assert_eq!(repo_entries.len(), 1);
        assert_eq!(repo_entries[0].name, "mem-1");

        let resolve_uri = ContextUri::new("pandora://index/repo/mem-1").unwrap();
        let frag = vfs.resolve(&resolve_uri).unwrap();
        assert_eq!(frag.content(), "architecture insight");

        // Attempting to resolve directory pandora://index/repo must return IsADirectory
        assert!(matches!(
            vfs.resolve(&repo_uri),
            Err(VfsError::IsADirectory(_))
        ));
    }

    #[test]
    fn vfs_overrides_meta_rule_and_resolves_updated_content() {
        let vfs = ContextVfs::new();
        let custom_rule = ContextFragment::new_with_origin(
            "meta:rules",
            ContextSource::Constitutional,
            ContextTrust::Constitutional,
            ContextClassification::Internal,
            255,
            "# Custom System Rules\n1. Enhanced security.\n2. Strict quality gates.",
            32,
            None,
            ContextOrigin::new("custom", "constitution").unwrap(),
        )
        .unwrap();
        vfs.add_meta_rule(custom_rule);

        let uri = ContextUri::new("pandora://meta/rules").unwrap();
        let frag = vfs.resolve(&uri).unwrap();
        assert!(frag.content().contains("Custom System Rules"));
    }

    #[test]
    fn vfs_search_hybrid_retrieval_and_lesson_pattern() {
        let vfs = ContextVfs::new();

        // Add L1 lesson record
        let scope = MemoryScope::new(
            TenantId::new("t1").unwrap(),
            WorkspaceId::new("w1").unwrap(),
            SessionId::new("s1").unwrap(),
            "p1",
        )
        .unwrap();

        let lesson = MemoryRecord::new_l1(
            "eval-lesson-exec-42",
            pandora_types::MemoryKind::Lesson,
            scope.clone(),
            "failed evaluations: compiler diagnostics type mismatch E0308; retry requires fresh verification",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(100),
            "evaluation:exec-42",
        )
        .unwrap()
        .with_intent(MemoryIntent::Procedure);
        vfs.add_memory_record(lesson);

        // Add SOP
        let sop = StoredSop {
            sop: SopDefinition {
                id: "sop-fix-e0308".to_string(),
                name: "Type Mismatch Remediation SOP".to_string(),
                trigger_intent: MemoryIntent::Procedure,
                steps: vec![SopStep {
                    id: "step-1".to_string(),
                    name: "Inspect Diagnostics".to_string(),
                    command: "cargo check --message-format=json".to_string(),
                    depends_on: vec![],
                }],
                verification_command: Some("cargo test -p pandora-runtime".to_string()),
            },
            tenant_id: TenantId::new("t1").unwrap(),
            workspace_id: WorkspaceId::new("w1").unwrap(),
            success_count: 5,
            created_at: Timestamp::from_unix_seconds(10),
            updated_at: Timestamp::from_unix_seconds(20),
        };
        vfs.add_sop(sop);

        // Search for compiler error
        let results = vfs
            .search("compiler diagnostics type mismatch E0308", 5)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "eval-lesson-exec-42");
        assert_eq!(
            results[0].uri.as_str(),
            "pandora://index/procedure/eval-lesson-exec-42"
        );
        assert!(results[0].hybrid_score > 0.0);
        assert!(
            results[0].snippet.contains("diagnostics")
                || results[0].snippet.contains("evaluations")
        );

        // Search for constitutional rules
        let rule_results = vfs
            .search("architectural integrity constitutional", 5)
            .unwrap();
        assert!(!rule_results.is_empty());
        assert!(rule_results[0].uri.as_str().starts_with("pandora://meta"));

        // Filter search by tier L3
        let options = ContextSearchOptions::new(5).with_tier(MemoryTier::L3);
        let sop_results = vfs
            .search_with_options("type mismatch remediation", &options)
            .unwrap();
        assert_eq!(sop_results.len(), 1);
        assert_eq!(sop_results[0].title, "Type Mismatch Remediation SOP");
        assert_eq!(sop_results[0].tier, Some(MemoryTier::L3));
    }

    fn make_test_record(
        id: &str,
        tier: MemoryTier,
        intent: MemoryIntent,
        summary: &str,
    ) -> MemoryRecord {
        let scope = MemoryScope::new(
            TenantId::new("t1").unwrap(),
            WorkspaceId::new("w1").unwrap(),
            SessionId::new("s1").unwrap(),
            "p1",
        )
        .unwrap();
        let l1 = MemoryRecord::new_l1(
            id,
            pandora_types::MemoryKind::Lesson,
            scope,
            summary,
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(100),
            "prov-1",
        )
        .unwrap()
        .with_intent(intent);
        if tier == MemoryTier::L2 {
            let approval = pandora_types::MemoryApproval::new("app-1", "operator-1").unwrap();
            MemoryRecord::promote_l2(l1, approval, Timestamp::from_unix_seconds(150)).unwrap()
        } else {
            l1
        }
    }

    fn make_test_sop(id: &str, name: &str, intent: MemoryIntent, step_cmd: &str) -> StoredSop {
        StoredSop {
            sop: SopDefinition {
                id: id.to_string(),
                name: name.to_string(),
                trigger_intent: intent,
                steps: vec![SopStep {
                    id: "step-1".to_string(),
                    name: "step-name".to_string(),
                    command: step_cmd.to_string(),
                    depends_on: vec![],
                }],
                verification_command: Some("cargo check".to_string()),
            },
            tenant_id: TenantId::new("t1").unwrap(),
            workspace_id: WorkspaceId::new("w1").unwrap(),
            success_count: 1,
            created_at: Timestamp::from_unix_seconds(10),
            updated_at: Timestamp::from_unix_seconds(20),
        }
    }

    #[test]
    fn test_vfs_index_binary_roundtrip() {
        let vfs = ContextVfs::new();

        let l1 = make_test_record(
            "rec-roundtrip-1",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust async cancellation tokens and graceful shutdown",
        );
        vfs.add_memory_record(l1);

        let sop = make_test_sop(
            "sop-roundtrip-1",
            "Cargo Workspace Build and Verify",
            MemoryIntent::Procedure,
            "cargo test --workspace",
        );
        vfs.add_sop(sop);

        let docs = vfs.collect_all_documents().unwrap();
        assert!(docs.len() >= 3); // default rule + l1 + sop

        let hasher = DenseFeatureHasher::new();
        let mut bm25 = SparseBm25Index::new();
        let mut doc_vectors = Vec::with_capacity(docs.len());

        for (idx, doc) in docs.iter().enumerate() {
            bm25.add_document(idx, &doc.content);
            let vec = hasher.vectorize(&doc.content);
            doc_vectors.push((idx, QuantizedVector::quantize(&vec)));
        }

        let corpus_hash = compute_corpus_hash(&docs);
        let created_at = 1730000000u64;

        let bytes = serialize_vfs_index(&docs, &bm25, &doc_vectors, corpus_hash, created_at)
            .expect("serialization must succeed");

        // Verify header layout: 4 (magic) + 4 (version) + 8 (created) + 4 (docs) + 4 (dim) + 8 (tokens) + 4 (vocab) + 32 (hash) = 68
        assert_eq!(&bytes[0..4], b"PVFS");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            created_at
        );
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            docs.len() as u32
        );
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 128);

        // Trailer: last 32 bytes are sha256 checksum
        let payload_len = bytes.len() - 32;
        let expected_checksum = Sha256::digest(&bytes[..payload_len]);
        assert_eq!(&bytes[payload_len..], expected_checksum.as_slice());

        // Deserialize
        let deserialized = deserialize_vfs_index(&bytes).expect("deserialization must succeed");
        assert_eq!(deserialized.version, 2);
        assert_eq!(deserialized.created_at, created_at);
        assert_eq!(deserialized.doc_count, docs.len() as u32);
        assert_eq!(deserialized.dense_dim, 128);
        assert_eq!(deserialized.corpus_hash, corpus_hash);
        assert_eq!(deserialized.docs.len(), docs.len());
        assert_eq!(deserialized.doc_vectors.len(), doc_vectors.len());

        for i in 0..docs.len() {
            assert_eq!(deserialized.docs[i].uri, docs[i].uri);
            assert_eq!(deserialized.docs[i].title, docs[i].title);
            assert_eq!(deserialized.docs[i].content, docs[i].content);
            assert_eq!(deserialized.docs[i].tier, docs[i].tier);
            assert_eq!(deserialized.docs[i].intent, docs[i].intent);

            // Vector bitwise equality
            assert_eq!(deserialized.doc_vectors[i].0, doc_vectors[i].0);
            assert_eq!(deserialized.doc_vectors[i].1, doc_vectors[i].1);
        }

        // Search equivalence on deserialized BM25
        let orig_scores = bm25.score("cancellation tokens");
        let deser_scores = deserialized.bm25.score("cancellation tokens");
        assert_eq!(orig_scores, deser_scores);
    }

    #[test]
    fn test_vfs_index_corrupted_checksum_detected() {
        let vfs = ContextVfs::new();
        let docs = vfs.collect_all_documents().unwrap();
        let hasher = DenseFeatureHasher::new();
        let mut bm25 = SparseBm25Index::new();
        let mut doc_vectors = Vec::new();
        for (idx, doc) in docs.iter().enumerate() {
            bm25.add_document(idx, &doc.content);
            let vec = hasher.vectorize(&doc.content);
            doc_vectors.push((idx, QuantizedVector::quantize(&vec)));
        }

        let corpus_hash = compute_corpus_hash(&docs);
        let bytes = serialize_vfs_index(&docs, &bm25, &doc_vectors, corpus_hash, 1000).unwrap();

        // Tamper with a payload byte
        let mut corrupted_payload = bytes.clone();
        corrupted_payload[10] ^= 0x55;
        let err1 = deserialize_vfs_index(&corrupted_payload).unwrap_err();
        assert!(
            matches!(err1, VfsError::BackendError(msg) if msg.contains("SHA-256 checksum mismatch"))
        );

        // Tamper with trailer checksum
        let mut corrupted_trailer = bytes.clone();
        let last = corrupted_trailer.len() - 1;
        corrupted_trailer[last] ^= 0x01;
        let err2 = deserialize_vfs_index(&corrupted_trailer).unwrap_err();
        assert!(
            matches!(err2, VfsError::BackendError(msg) if msg.contains("SHA-256 checksum mismatch"))
        );

        // Truncated bytes
        let truncated = &bytes[..40];
        let err3 = deserialize_vfs_index(truncated).unwrap_err();
        assert!(matches!(err3, VfsError::BackendError(msg) if msg.contains("too short")));
    }

    #[test]
    fn test_vfs_index_invalidation_on_corpus_modification() {
        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_inval_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");

        let vfs = ContextVfs::new();
        vfs.set_index_path(&index_path);

        // Initially not fresh (file doesn't exist)
        assert!(!vfs.is_index_fresh(&index_path).unwrap());

        // Save index
        vfs.save_index_to_disk(&index_path).unwrap();
        assert!(vfs.is_index_fresh(&index_path).unwrap());

        // Read stats
        let stats = read_index_stats(&index_path, Some(&vfs)).unwrap();
        assert!(stats.checksum_valid);
        assert!(stats.is_fresh);
        assert_eq!(stats.dense_dimension, 128);
        assert_eq!(stats.version, VFS_INDEX_VERSION);
        assert!(stats.vector_bytes > 0);
        assert!(stats.vector_bytes_saved > 0);

        // Modify corpus: add new SOP
        let new_sop = make_test_sop(
            "sop-inval-test",
            "New SOP Triggering Invalidation",
            MemoryIntent::Procedure,
            "cargo test --workspace",
        );
        vfs.add_sop(new_sop);

        // Index is now stale!
        assert!(!vfs.is_index_fresh(&index_path).unwrap());

        // Search automatically detects stale index, rebuilds and persists it
        let search_res = vfs.search("Triggering Invalidation", 5).unwrap();
        assert!(!search_res.is_empty());
        assert_eq!(search_res[0].title, "New SOP Triggering Invalidation");

        // Now index is fresh again!
        assert!(vfs.is_index_fresh(&index_path).unwrap());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_vfs_index_cold_start_performance() {
        let vfs = ContextVfs::new();
        // Add 15 records to simulate a realistic corpus
        for i in 0..15 {
            let rec = make_test_record(
                &format!("perf-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!(
                    "Persistent index performance benchmarking session entry {i} covering latency"
                ),
            );
            vfs.add_memory_record(rec);
        }

        let docs = vfs.collect_all_documents().unwrap();
        let hasher = DenseFeatureHasher::new();
        let mut bm25 = SparseBm25Index::new();
        let mut doc_vectors = Vec::new();
        for (idx, doc) in docs.iter().enumerate() {
            bm25.add_document(idx, &doc.content);
            let vec = hasher.vectorize(&doc.content);
            doc_vectors.push((idx, QuantizedVector::quantize(&vec)));
        }

        let corpus_hash = compute_corpus_hash(&docs);
        let bytes = serialize_vfs_index(&docs, &bm25, &doc_vectors, corpus_hash, 12345).unwrap();

        // Cold-start deserialization must complete well within 5ms
        let start = std::time::Instant::now();
        let deserialized = deserialize_vfs_index(&bytes).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(deserialized.doc_count, docs.len() as u32);
        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "cold-start deserialization took {:?}, expected < 5ms",
            elapsed
        );
    }

    #[test]
    fn test_vfs_search_results_equivalence() {
        let vfs = ContextVfs::new();
        let rec1 = make_test_record(
            "equiv-1",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust trait bounds and associated type parameters",
        );
        let rec2 = make_test_record(
            "equiv-2",
            MemoryTier::L2,
            MemoryIntent::Repo,
            "Workspace build configuration with w64devkit MinGW toolchain",
        );
        vfs.add_memory_record(rec1);
        vfs.add_memory_record(rec2);

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_equiv_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        // Save index
        vfs.save_index_to_disk(&index_path).unwrap();

        // Query with cached/persistent index
        let query = "trait bounds associated generics";
        let results_cached = vfs.search(query, 5).unwrap();

        // Create an unindexed VFS with identical content (pure on-the-fly)
        let vfs_onthefly = ContextVfs::new();
        let rec1_clone = make_test_record(
            "equiv-1",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust trait bounds and associated type parameters",
        );
        let rec2_clone = make_test_record(
            "equiv-2",
            MemoryTier::L2,
            MemoryIntent::Repo,
            "Workspace build configuration with w64devkit MinGW toolchain",
        );
        vfs_onthefly.add_memory_record(rec1_clone);
        vfs_onthefly.add_memory_record(rec2_clone);
        vfs_onthefly.set_index_path(temp_dir.join("nonexistent_vfs.idx"));
        let results_onthefly = vfs_onthefly.search(query, 5).unwrap();

        assert_eq!(results_cached.len(), results_onthefly.len());
        for i in 0..results_cached.len() {
            assert_eq!(results_cached[i].uri, results_onthefly[i].uri);
            assert_eq!(results_cached[i].title, results_onthefly[i].title);
            assert!(
                (results_cached[i].hybrid_score - results_onthefly[i].hybrid_score).abs() < 1e-5
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// Helper to serialize Version 1 index files for backward-compatibility testing.
    fn serialize_vfs_index_v1(
        docs: &[VfsDoc],
        bm25: &SparseBm25Index,
        doc_vectors: &[(usize, [f32; 128])],
        corpus_hash: [u8; 32],
        created_at_unix: u64,
    ) -> Result<Vec<u8>, VfsError> {
        let mut buffer = Vec::new();

        // 1. Header (68 bytes) with version = 1
        buffer.extend_from_slice(VFS_INDEX_MAGIC);
        buffer.extend_from_slice(&VFS_INDEX_VERSION_V1.to_le_bytes());
        buffer.extend_from_slice(&created_at_unix.to_le_bytes());
        let doc_count = docs.len() as u32;
        buffer.extend_from_slice(&doc_count.to_le_bytes());
        buffer.extend_from_slice(&VFS_DENSE_DIMENSION.to_le_bytes());
        buffer.extend_from_slice(&(bm25.total_tokens() as u64).to_le_bytes());

        let mut terms: Vec<_> = bm25.postings().keys().collect();
        terms.sort();
        let vocab_size = terms.len() as u32;
        buffer.extend_from_slice(&vocab_size.to_le_bytes());
        buffer.extend_from_slice(&corpus_hash);

        // 2. Document Metadata & 512-byte Float Vector Table
        for (idx, doc) in docs.iter().enumerate() {
            let doc_id = idx as u32;
            buffer.extend_from_slice(&doc_id.to_le_bytes());

            let uri_bytes = doc.uri.as_str().as_bytes();
            buffer.extend_from_slice(&(uri_bytes.len() as u16).to_le_bytes());
            buffer.extend_from_slice(uri_bytes);

            let title_bytes = doc.title.as_bytes();
            buffer.extend_from_slice(&(title_bytes.len() as u16).to_le_bytes());
            buffer.extend_from_slice(title_bytes);

            let content_bytes = doc.content.as_bytes();
            buffer.extend_from_slice(&(content_bytes.len() as u32).to_le_bytes());
            buffer.extend_from_slice(content_bytes);

            buffer.push(tier_to_u8(doc.tier));
            buffer.push(intent_to_u8(doc.intent));

            let doc_length = bm25.doc_lengths().get(&idx).copied().unwrap_or(0) as u32;
            buffer.extend_from_slice(&doc_length.to_le_bytes());

            // 512 bytes per document float vector
            let vec = doc_vectors
                .get(idx)
                .map(|(_, v)| *v)
                .unwrap_or([0.0f32; 128]);
            for f in vec {
                buffer.extend_from_slice(&f.to_le_bytes());
            }
        }

        // 3. Postings Table
        for term in terms {
            let term_bytes = term.as_bytes();
            buffer.extend_from_slice(&(term_bytes.len() as u16).to_le_bytes());
            buffer.extend_from_slice(term_bytes);

            let mut postings = bm25.postings().get(term).cloned().unwrap_or_default();
            postings.sort_by_key(|&(doc_id, _)| doc_id);
            let postings_count = postings.len() as u32;
            buffer.extend_from_slice(&postings_count.to_le_bytes());

            for (doc_id, tf) in postings {
                buffer.extend_from_slice(&(doc_id as u32).to_le_bytes());
                buffer.extend_from_slice(&tf.to_le_bytes());
            }
        }

        // 4. Trailer
        let checksum = Sha256::digest(&buffer);
        buffer.extend_from_slice(&checksum);

        Ok(buffer)
    }

    #[test]
    fn test_vfs_index_version_1_backward_compatibility() {
        let vfs = ContextVfs::new();
        let docs = vfs.collect_all_documents().unwrap();
        let hasher = DenseFeatureHasher::new();
        let mut bm25 = SparseBm25Index::new();
        let mut f32_vectors = Vec::new();
        for (idx, doc) in docs.iter().enumerate() {
            bm25.add_document(idx, &doc.content);
            f32_vectors.push((idx, hasher.vectorize(&doc.content)));
        }

        let corpus_hash = compute_corpus_hash(&docs);
        let v1_bytes =
            serialize_vfs_index_v1(&docs, &bm25, &f32_vectors, corpus_hash, 9999).unwrap();

        // Confirm version header is 1
        assert_eq!(u32::from_le_bytes(v1_bytes[4..8].try_into().unwrap()), 1);

        // Deserializer must seamlessly decode Version 1 files
        let deser = deserialize_vfs_index(&v1_bytes).expect("must deserialize v1 index");
        assert_eq!(deser.version, 1);
        assert_eq!(deser.doc_count, docs.len() as u32);
        assert_eq!(deser.docs.len(), docs.len());
        assert_eq!(deser.doc_vectors.len(), docs.len());

        // Verify vectors were quantized with high precision
        for (idx, (_, qv)) in deser.doc_vectors.iter().enumerate() {
            let orig_f32 = &f32_vectors[idx].1;
            let deq = qv.dequantize();
            for dim in 0..128 {
                assert!((deq[dim] - orig_f32[dim]).abs() < 0.01);
            }
        }

        // Search on deserialized v1 index
        let scores = deser.bm25.score("constitutional rules");
        assert!(!scores.is_empty());
    }

    #[test]
    fn test_vfs_storage_reduction_75_percent() {
        let vfs = ContextVfs::new();
        for i in 0..10 {
            vfs.add_memory_record(make_test_record(
                &format!("storage-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Storage footprint reduction validation record {i} for context VFS int8 quantization"),
            ));
        }

        let docs = vfs.collect_all_documents().unwrap();
        let doc_count = docs.len();

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_storage_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        vfs.save_index_to_disk(&index_path).unwrap();

        let stats = read_index_stats(&index_path, Some(&vfs)).unwrap();
        assert_eq!(stats.version, VFS_INDEX_VERSION);
        assert_eq!(stats.doc_count as usize, doc_count);

        // Exact coordinate reduction: 128 bytes vs 512 bytes = 75.00%
        let baseline_coord_bytes = doc_count * 128 * 4;
        let quant_coord_bytes = doc_count * 128;
        let coord_reduction =
            (baseline_coord_bytes - quant_coord_bytes) as f64 / (baseline_coord_bytes as f64);
        assert!((coord_reduction - 0.75).abs() < 1e-6);

        // Net storage reduction including scale factor (132 bytes vs 512 bytes):
        // (512 - 132) / 512 = 380 / 512 = 74.21875% ~= 74.22%
        let net_reduction = (baseline_coord_bytes - stats.vector_bytes as usize) as f64
            / (baseline_coord_bytes as f64);
        assert!((net_reduction - 0.7421875).abs() < 1e-4);
        assert_eq!(
            stats.vector_bytes as usize,
            doc_count * VFS_QUANTIZED_VECTOR_SIZE
        );
        assert_eq!(stats.vector_bytes_saved as usize, doc_count * 380);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_vfs_incremental_diff_caching() {
        let vfs = ContextVfs::new();

        // Seed with 20 records
        for i in 0..20 {
            vfs.add_memory_record(make_test_record(
                &format!("diff-cache-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Incremental diff caching corpus record {i} for latency verification"),
            ));
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_diff_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        vfs.reset_diff_cache_stats();

        // 1st indexing pass: cold cache -> all documents are cache misses
        vfs.save_index_to_disk(&index_path).unwrap();
        let (hits1, misses1) = vfs.diff_cache_stats();
        let initial_doc_count = vfs.collect_all_documents().unwrap().len();
        assert_eq!(hits1, 0);
        assert_eq!(misses1 as usize, initial_doc_count);

        // Re-saving without changes: 100% cache hits, 0 cache misses
        vfs.reset_diff_cache_stats();
        vfs.save_index_to_disk(&index_path).unwrap();
        let (hits2, misses2) = vfs.diff_cache_stats();
        assert_eq!(hits2 as usize, initial_doc_count);
        assert_eq!(misses2, 0);

        // Add exactly 1 new record
        vfs.add_memory_record(make_test_record(
            "diff-cache-rec-new-21",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Fresh newly appended record that must trigger exactly one cache miss",
        ));

        // 3rd indexing pass: exactly 1 cache miss, all prior documents hit cache
        vfs.reset_diff_cache_stats();
        vfs.save_index_to_disk(&index_path).unwrap();
        let (hits3, misses3) = vfs.diff_cache_stats();
        assert_eq!(
            misses3, 1,
            "adding 1 document must result in exactly 1 cache miss"
        );
        assert_eq!(
            hits3 as usize, initial_doc_count,
            "all prior documents must hit diff cache"
        );

        // Verify diff cache size matches active document count
        assert_eq!(vfs.diff_cache().len(), initial_doc_count + 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hnsw_build_from_context_vfs_documents() {
        let vfs = ContextVfs::new();

        // Seed with varied documents
        for i in 0..15 {
            vfs.add_memory_record(make_test_record(
                &format!("hnsw-build-rec-{i}"),
                if i % 2 == 0 {
                    MemoryTier::L1
                } else {
                    MemoryTier::L2
                },
                MemoryIntent::Coding,
                &format!(
                    "Document number {i} discussing vector search algorithms and data structures"
                ),
            ));
        }

        let all_docs = vfs.collect_all_documents().unwrap();
        let expected_count = all_docs.len();
        assert!(expected_count >= 16); // 1 default rule + 15 records

        let stats = vfs.build_hnsw_index(None).unwrap();
        assert_eq!(stats.node_count, expected_count);
        assert!(stats.entry_point.is_some());
        assert!(stats.entry_point.unwrap() < stats.node_count);
        assert!(!stats.layer_node_counts.is_empty());
        assert_eq!(stats.layer_node_counts[0], expected_count);
        assert!(stats.average_degree > 0.0);

        let cached = vfs
            .cached_hnsw_index()
            .expect("HNSW index should be cached");
        assert_eq!(cached.len(), expected_count);
        assert_eq!(cached.corpus_hash(), &compute_corpus_hash(&all_docs));
    }

    #[test]
    fn test_hnsw_persistence_to_disk_and_reload_roundtrip() {
        let vfs = ContextVfs::new();

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_hnsw_io_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("vfs_hnsw.idx");
        vfs.set_hnsw_index_path(&idx_path);

        for i in 0..10 {
            vfs.add_memory_record(make_test_record(
                &format!("hnsw-persist-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Repo,
                &format!("Persistence test record {i} for binary PVHN format verification"),
            ));
        }

        // Build and save to disk
        let saved_path = vfs.save_hnsw_index(None).unwrap();
        assert_eq!(saved_path, idx_path);
        assert!(idx_path.is_file());
        let file_len = std::fs::metadata(&idx_path).unwrap().len();
        assert!(file_len > (PVHN_HEADER_SIZE + PVHN_TRAILER_SIZE) as u64);

        // Verify fresh before clearing cache
        assert!(vfs.is_hnsw_index_fresh());

        // Clear in-memory cached index
        vfs.clear_cached_hnsw_index();
        assert!(vfs.cached_hnsw_index().is_none());

        // Still fresh because disk file exists and corpus hasn't changed
        assert!(vfs.is_hnsw_index_fresh());

        // Load or build should reload from disk
        let reloaded = vfs.load_or_build_hnsw_index().unwrap();
        let all_docs = vfs.collect_all_documents().unwrap();
        assert_eq!(reloaded.len(), all_docs.len());
        assert_eq!(reloaded.corpus_hash(), &compute_corpus_hash(&all_docs));

        // Stats should match
        let stats = vfs.hnsw_index_stats().unwrap();
        assert_eq!(stats.node_count, all_docs.len());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hnsw_cold_start_loading_sub_millisecond() {
        let vfs = ContextVfs::new();

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_hnsw_cold_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("vfs_hnsw.idx");
        vfs.set_hnsw_index_path(&idx_path);

        for i in 0..40 {
            vfs.add_memory_record(make_test_record(
                &format!("cold-start-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Performance cold start benchmark document {i} with semantic context"),
            ));
        }

        vfs.save_hnsw_index(None).unwrap();

        // Evict in-memory cache
        vfs.clear_cached_hnsw_index();

        // Benchmark cold-start loading from disk
        let start = std::time::Instant::now();
        let loaded = vfs.load_or_build_hnsw_index().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(loaded.len(), vfs.collect_all_documents().unwrap().len());
        // Verify cold load is fast (under 25ms in unoptimized debug build, < 1ms in release)
        assert!(
            elapsed.as_millis() < 25,
            "Cold start load took {:?}, expected < 25ms in debug",
            elapsed
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hnsw_search_accuracy_and_retrieval() {
        let vfs = ContextVfs::new();

        vfs.add_memory_record(make_test_record(
            "doc-rust-ownership",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust memory safety and borrow checker ownership model prevents data races and dangling pointers",
        ));

        vfs.add_memory_record(make_test_record(
            "doc-quantum-computing",
            MemoryTier::L1,
            MemoryIntent::Repo,
            "Quantum computing qubits superposition entanglement quantum gates and Shor algorithm",
        ));

        vfs.add_memory_record(make_test_record(
            "doc-distributed-raft",
            MemoryTier::L2,
            MemoryIntent::Procedure,
            "Distributed consensus Raft protocol leader election log replication and heartbeat quorum",
        ));

        vfs.build_hnsw_index(None).unwrap();

        // Direct HNSW search
        let results = vfs
            .search_hnsw("quantum computing qubits superposition", 2, 64)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "doc-quantum-computing");
        assert!(results[0].dense_score > 0.6);

        // Search for rust borrow checker
        let rust_results = vfs
            .search_hnsw("Rust memory safety borrow checker", 2, 64)
            .unwrap();
        assert!(!rust_results.is_empty());
        assert_eq!(rust_results[0].title, "doc-rust-ownership");
        assert!(rust_results[0].dense_score > 0.6);

        // Verify search_with_options routes through HNSW seamlessly
        let hybrid_opts = ContextSearchOptions::new(3);
        let hybrid_results = vfs
            .search_with_options("Raft consensus leader election", &hybrid_opts)
            .unwrap();
        assert!(!hybrid_results.is_empty());
        assert_eq!(hybrid_results[0].title, "doc-distributed-raft");
    }

    #[test]
    fn test_hnsw_freshness_invalidation_on_buffer_modification() {
        let vfs = ContextVfs::new();

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_hnsw_fresh_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("vfs_hnsw.idx");
        vfs.set_hnsw_index_path(&idx_path);

        vfs.add_memory_record(make_test_record(
            "fresh-rec-1",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Initial document before modification check",
        ));

        vfs.save_hnsw_index(None).unwrap();
        assert!(vfs.is_hnsw_index_fresh());

        // Small sleep to ensure timestamp moves forward if on second boundary
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Add a new record -> must invalidate freshness
        vfs.add_memory_record(make_test_record(
            "fresh-rec-2",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "New subsequent document invalidating previous HNSW index",
        ));

        assert!(
            !vfs.is_hnsw_index_fresh(),
            "Disk HNSW index must report stale after buffer modification"
        );
        let synced = vfs
            .cached_hnsw_index()
            .expect("cached HNSW index must stay incrementally synchronized");
        let live_docs = vfs.collect_all_documents().unwrap();
        assert_eq!(synced.live_count(), live_docs.len());
        assert_eq!(synced.corpus_hash(), &compute_corpus_hash(&live_docs));

        // Saving synced index restores on-disk freshness
        vfs.save_hnsw_index(None).unwrap();
        assert!(vfs.is_hnsw_index_fresh());

        let stats = vfs.hnsw_index_stats().unwrap();
        assert_eq!(stats.node_count, live_docs.len());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_pvfs_v3_roundtrip_and_tropical_vector_fidelity() {
        let vfs = ContextVfs::new();
        for i in 0..5 {
            vfs.add_memory_record(make_test_record(
                &format!("pvfs-v3-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Tropical semiring max-plus algebra persistence record {i}"),
            ));
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_pvfs_v3_roundtrip_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);
        vfs.save_index_to_disk(&index_path).unwrap();

        let bytes = std::fs::read(&index_path).unwrap();
        let deser = deserialize_vfs_index(&bytes).unwrap();
        assert_eq!(deser.version, VFS_INDEX_VERSION_V3);
        let docs = vfs.collect_all_documents().unwrap();
        assert_eq!(deser.doc_vectors.len(), docs.len());
        assert_eq!(deser.tropical_vectors.len(), docs.len());

        // Verify persisted 512B TropicalVectors match TropicalProjector::project bit-for-bit
        // (proving direct v3 persistence rather than lazy from_quantized_fallback).
        for (doc_idx, doc) in docs.iter().enumerate() {
            let expected_tv = TropicalProjector::project(&doc.content);
            let (_, actual_tv) = deser.tropical_vectors[doc_idx];
            assert_eq!(
                actual_tv.to_bytes(),
                expected_tv.to_bytes(),
                "doc {doc_idx} tropical vector must roundtrip losslessly without fallback"
            );
        }

        // Hydrate a fresh ContextVfs from the v3 index and confirm zero cold-start misses
        let vfs2 = ContextVfs::new();
        for i in 0..5 {
            vfs2.add_memory_record(make_test_record(
                &format!("pvfs-v3-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Tropical semiring max-plus algebra persistence record {i}"),
            ));
        }
        vfs2.populate_diff_cache_from_index(&deser);
        vfs2.reset_diff_cache_stats();
        vfs2.save_index_to_disk(&index_path).unwrap();
        let (hits, misses) = vfs2.diff_cache_stats();
        assert_eq!(hits as usize, docs.len());
        assert_eq!(misses, 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_load_or_build_hnsw_index_defaults_to_tropical() {
        let vfs = ContextVfs::new();
        vfs.add_memory_record(make_test_record(
            "trop-hnsw-rust",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust borrow checker ownership lifetimes and memory safety invariants",
        ));
        vfs.add_memory_record(make_test_record(
            "trop-hnsw-mcts",
            MemoryTier::L2,
            MemoryIntent::Procedure,
            "Tropical semiring PUCT Monte Carlo tree search rollout pruning",
        ));

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_hnsw_trop_default_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let hnsw_path = temp_dir.join("vfs_hnsw.idx");
        vfs.set_hnsw_index_path(&hnsw_path);

        let built = vfs.load_or_build_hnsw_index().unwrap();
        assert!(!built.is_empty());
        assert!(
            built.nodes().iter().all(|n| n.vector.is_tropical()),
            "load_or_build_hnsw_index must default to Tropical vectors on fresh build"
        );
        assert!(hnsw_path.is_file());

        // Cold reload from PVHN disk file preserves Tropical vector representation
        let disk_bytes = std::fs::read(&hnsw_path).unwrap();
        let reloaded = HnswIndex::from_pvhn_bytes(&disk_bytes).unwrap();
        assert_eq!(reloaded.len(), built.len());
        assert!(
            reloaded.nodes().iter().all(|n| n.vector.is_tropical()),
            "reloaded PVHN index must preserve Tropical vectors"
        );

        let hits = vfs
            .search_hnsw("Tropical semiring PUCT rollout", 2, 32)
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].title, "trop-hnsw-mcts");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_vfs_tombstone_remove_and_pvfs_v3_persistence() {
        let vfs = ContextVfs::new().with_auto_compact_threshold(10, 0.9);
        for i in 0..8 {
            vfs.add_memory_record(make_test_record(
                &format!("tomb-rec-{i}"),
                MemoryTier::L1,
                MemoryIntent::Coding,
                &format!("Rust async tokio channel pattern document {i}"),
            ));
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_pvfs_tomb_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        // Legacy v3 payload (0 tombstones) backward-compatibility check
        vfs.save_index_to_disk(&index_path).unwrap();
        let initial_bytes = std::fs::read(&index_path).unwrap();
        let parsed_initial = deserialize_vfs_index(&initial_bytes).unwrap();
        assert!(parsed_initial.tombstoned_uris.is_empty());
        assert_eq!(parsed_initial.compaction_count, 0);

        // Soft-tombstone one document and hard-remove another
        assert!(vfs.tombstone_document("pandora://index/coding/tomb-rec-1"));
        assert!(!vfs.tombstone_document("pandora://index/coding/tomb-rec-1"));
        assert!(vfs.remove_document("pandora://index/coding/tomb-rec-2"));
        assert!(!vfs.remove_document("pandora://index/coding/does-not-exist"));
        assert_eq!(vfs.tombstone_count(), 2);
        assert!(vfs.is_tombstoned("pandora://index/coding/tomb-rec-1"));
        assert!(vfs.is_tombstoned("pandora://index/coding/tomb-rec-2"));

        // Tombstoned URIs are hidden from resolve, list, and search
        let tomb_uri = ContextUri::new("pandora://index/coding/tomb-rec-1").unwrap();
        assert!(matches!(vfs.resolve(&tomb_uri), Err(VfsError::NotFound(_))));
        let listed = vfs
            .list(&ContextUri::new("pandora://index/coding").unwrap())
            .unwrap();
        assert!(
            !listed
                .iter()
                .any(|e| e.name == "tomb-rec-1" || e.name == "tomb-rec-2")
        );

        // Persist PVFS v3 with tombstones below auto-compact threshold
        vfs.save_index_to_disk(&index_path).unwrap();
        let stats = read_index_stats(&index_path, Some(&vfs)).unwrap();
        assert!(stats.checksum_valid);
        assert!(stats.is_fresh);
        assert_eq!(stats.tombstone_count, 2);
        assert!(stats.tombstone_ratio > 0.0);
        assert_eq!(stats.compaction_count, 0);

        let disk_bytes = std::fs::read(&index_path).unwrap();
        let decoded = deserialize_vfs_index(&disk_bytes).unwrap();
        assert_eq!(decoded.tombstoned_uris.len(), 2);
        assert_eq!(decoded.doc_count, 7); // 1 meta rule + 6 remaining L1 records

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_vfs_auto_compaction_threshold_and_manual_compact() {
        let vfs = ContextVfs::new().with_auto_compact_threshold(3, 0.25);
        for i in 0..9 {
            vfs.add_memory_record(make_test_record(
                &format!("auto-rec-{i}"),
                MemoryTier::L2,
                MemoryIntent::Repo,
                &format!("Deterministic repository architecture fact {i}"),
            ));
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_pvfs_autocompact_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let index_path = temp_dir.join("vfs.idx");
        vfs.set_index_path(&index_path);

        // 2 tombstones < min_tombstones(3) -> no auto-compaction yet
        assert!(vfs.tombstone_document("pandora://facts/repo/auto-rec-0"));
        assert!(vfs.tombstone_document("pandora://facts/repo/auto-rec-1"));
        assert!(!vfs.should_auto_compact());
        vfs.save_index_to_disk(&index_path).unwrap();
        assert_eq!(vfs.tombstone_count(), 2);
        assert_eq!(vfs.compaction_count(), 0);

        // 3rd tombstone crosses min_tombstones(3) & ratio(3/10 >= 0.25) -> save triggers auto-compact
        assert!(vfs.tombstone_document("pandora://facts/repo/auto-rec-2"));
        assert!(vfs.should_auto_compact());
        vfs.save_index_to_disk(&index_path).unwrap();

        assert_eq!(vfs.tombstone_count(), 0);
        assert_eq!(vfs.compaction_count(), 1);
        assert!(vfs.last_compacted_unix() > 0);
        let stats = read_index_stats(&index_path, Some(&vfs)).unwrap();
        assert_eq!(stats.tombstone_count, 0);
        assert_eq!(stats.compaction_count, 1);
        assert_eq!(stats.doc_count, 7); // 1 default meta rule + 6 remaining facts

        // Manual compaction works even for a single tombstone
        assert!(vfs.remove_memory_record("auto-rec-3"));
        assert_eq!(vfs.tombstone_count(), 1);
        let report = vfs.compact_index().unwrap();
        assert_eq!(report.tombstones_purged, 1);
        assert_eq!(report.docs_after, 6);
        assert_eq!(report.compaction_count, 2);
        assert_eq!(vfs.tombstone_count(), 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_vfs_incremental_hnsw_sync_and_post_compaction_query_equivalence() {
        let vfs = ContextVfs::new().with_auto_compact_threshold(20, 0.8);
        let clean_vfs = ContextVfs::new();

        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_vfs_hnsw_equiv_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        vfs.set_index_path(temp_dir.join("vfs.idx"));
        vfs.set_hnsw_index_path(temp_dir.join("vfs_hnsw.idx"));
        clean_vfs.set_index_path(temp_dir.join("clean_vfs.idx"));
        clean_vfs.set_hnsw_index_path(temp_dir.join("clean_vfs_hnsw.idx"));

        let corpus = [
            ("doc-00", "Tropical semiring max-plus algebra optimization"),
            ("doc-01", "Kubernetes pod autoscaling horizontal metrics"),
            (
                "doc-02",
                "Ed25519 cryptographic attestation signature verification",
            ),
            (
                "doc-03",
                "PostgreSQL vacuum analyze query planner statistics",
            ),
            ("doc-04", "Rust zero-copy parser lifetime borrow checker"),
            ("doc-05", "Monte Carlo tree search PUCT exploration bonus"),
            (
                "doc-06",
                "Sinkhorn entropic optimal transport Wasserstein drift",
            ),
            ("doc-07", "Martingale e-value anytime valid canary gate"),
            ("doc-08", "HNSW hierarchical navigable small world graph"),
            ("doc-09", "BM25 inverted index term frequency saturation"),
            ("doc-10", "WebAssembly sandbox deterministic fuel metering"),
            ("doc-11", "GraphQL schema federation resolver batching"),
        ];

        for (id, summary) in &corpus {
            vfs.add_memory_record(make_test_record(
                id,
                MemoryTier::L1,
                MemoryIntent::Coding,
                summary,
            ));
        }

        // Build initial tropical HNSW index
        vfs.build_hnsw_index_tropical(Some(HnswConfig::new(8, 32, 64)))
            .unwrap();
        assert!(vfs.cached_hnsw_index().is_some());

        // Incrementally delete 4 docs, update 2 docs, and insert 2 new docs
        assert!(vfs.remove_document("pandora://index/coding/doc-01"));
        assert!(vfs.tombstone_document("pandora://index/coding/doc-03"));
        assert!(vfs.remove_document("pandora://index/coding/doc-07"));
        assert!(vfs.tombstone_document("pandora://index/coding/doc-11"));

        vfs.add_memory_record(make_test_record(
            "doc-04",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Rust async tokio runtime work-stealing scheduler",
        ));
        vfs.add_memory_record(make_test_record(
            "doc-09",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "PVFS v3 tombstone compaction incremental HNSW update",
        ));
        vfs.add_memory_record(make_test_record(
            "doc-12",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "LatsPrefixMemoCache 4-bit quantized decision memoization",
        ));
        vfs.add_memory_record(make_test_record(
            "doc-13",
            MemoryTier::L1,
            MemoryIntent::Coding,
            "DPO preference pair cross-session memory capsule distillation",
        ));

        // Cached HNSW index remained warm and synchronized incrementally
        let pre_compact_hnsw = vfs
            .cached_hnsw_index()
            .expect("cached HNSW must stay incrementally synced without dropping to None");
        assert_eq!(pre_compact_hnsw.tombstone_count(), 6); // 4 deletes + 2 replaced upserts

        // Pre-compaction search excludes deleted docs and surfaces newly upserted docs
        let pre_hits = vfs
            .search_hnsw("LatsPrefixMemoCache quantized memoization", 5, 64)
            .unwrap();
        assert!(!pre_hits.is_empty());
        assert_eq!(pre_hits[0].title, "doc-12");
        assert!(
            !pre_hits
                .iter()
                .any(|h| ["doc-01", "doc-03", "doc-07", "doc-11"].contains(&h.title.as_str()))
        );

        // Compact VFS + HNSW
        let report = vfs.compact_index().unwrap();
        assert!(report.hnsw_compacted);
        assert_eq!(report.tombstones_purged, 4);
        assert_eq!(vfs.tombstone_count(), 0);
        let post_compact_hnsw = vfs.cached_hnsw_index().unwrap();
        assert_eq!(post_compact_hnsw.tombstone_count(), 0);
        assert_eq!(
            post_compact_hnsw.live_count(),
            vfs.collect_all_documents().unwrap().len()
        );

        // Build clean reference VFS with exact same final live corpus
        for doc in vfs.collect_all_documents().unwrap() {
            if doc.uri.as_str().starts_with("pandora://index/") {
                let id = doc.title.clone();
                let summary = doc
                    .content
                    .strip_prefix(&format!("{id} "))
                    .unwrap_or(&doc.content)
                    .to_string();
                clean_vfs.add_memory_record(make_test_record(
                    &id,
                    MemoryTier::L1,
                    MemoryIntent::Coding,
                    &summary,
                ));
            }
        }
        clean_vfs
            .build_hnsw_index_tropical(Some(HnswConfig::new(8, 32, 64)))
            .unwrap();

        let queries = [
            "Tropical semiring max-plus algebra",
            "Ed25519 attestation signature",
            "PVFS v3 tombstone compaction",
            "LatsPrefixMemoCache quantized decision",
            "Sinkhorn Wasserstein drift",
        ];
        for q in &queries {
            let got_hnsw = vfs.search_hnsw(q, 3, 64).unwrap();
            let ref_hnsw = clean_vfs.search_hnsw(q, 3, 64).unwrap();
            assert!(!got_hnsw.is_empty());
            assert_eq!(got_hnsw[0].uri, ref_hnsw[0].uri, "top-1 mismatch for '{q}'");

            let got_hybrid = vfs.search(q, 3).unwrap();
            let ref_hybrid = clean_vfs.search(q, 3).unwrap();
            assert_eq!(
                got_hybrid
                    .iter()
                    .map(|r| r.uri.as_str())
                    .collect::<Vec<_>>(),
                ref_hybrid
                    .iter()
                    .map(|r| r.uri.as_str())
                    .collect::<Vec<_>>(),
                "hybrid search mismatch after compaction for '{q}'"
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
