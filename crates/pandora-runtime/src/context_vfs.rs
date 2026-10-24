use pandora_types::{
    ContextClassification, ContextFragment, ContextOrigin, ContextSource, ContextTrust, ContextUri,
    FeatureMap, MemoryRecord, MemoryTier, SessionArchiveRecord, SkillManifest, StoredSop, TenantId,
    VfsDirectoryEntry, VfsNodeKind, WorkspaceId,
};
use std::fmt;
use std::sync::RwLock;

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

pub struct ContextVfs {
    meta_rules: RwLock<Vec<ContextFragment>>,
    l1_records: RwLock<Vec<MemoryRecord>>,
    l2_records: RwLock<Vec<MemoryRecord>>,
    sops: RwLock<Vec<StoredSop>>,
    archives: RwLock<Vec<SessionArchiveRecord>>,
    feature_maps: RwLock<Vec<(String, FeatureMap)>>,
    skills: RwLock<Vec<SkillManifest>>,
}

impl Default for ContextVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextVfs {
    pub fn new() -> Self {
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

    pub fn add_meta_rule(&self, rule: ContextFragment) {
        if let Ok(mut rules) = self.meta_rules.write() {
            rules.retain(|r| r.id() != rule.id());
            rules.push(rule);
        }
    }

    pub fn add_sop(&self, sop: StoredSop) {
        if let Ok(mut sops) = self.sops.write() {
            sops.retain(|s| s.sop.id != sop.sop.id);
            sops.push(sop);
        }
    }

    pub fn add_archive(&self, archive: SessionArchiveRecord) {
        if let Ok(mut archives) = self.archives.write() {
            archives.retain(|a| a.archive_id != archive.archive_id);
            archives.push(archive);
        }
    }

    pub fn add_memory_record(&self, record: MemoryRecord) {
        match record.tier() {
            MemoryTier::L1 => {
                if let Ok(mut list) = self.l1_records.write() {
                    list.retain(|r| r.id() != record.id());
                    list.push(record);
                }
            }
            MemoryTier::L2 => {
                if let Ok(mut list) = self.l2_records.write() {
                    list.retain(|r| r.id() != record.id());
                    list.push(record);
                }
            }
            _ => {}
        }
    }

    pub fn add_feature_map(&self, name: impl Into<String>, feature_map: FeatureMap) {
        let name = name.into();
        if let Ok(mut maps) = self.feature_maps.write() {
            maps.retain(|(n, _)| n != &name);
            maps.push((name, feature_map));
        }
    }

    pub fn add_skill(&self, skill: SkillManifest) {
        if let Ok(mut skills) = self.skills.write() {
            skills.retain(|s| s.id() != skill.id());
            skills.push(skill);
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
                        let json = serde_json::to_string(&sop.sop).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://sops/{}", sop.sop.id))
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
                        let json = serde_json::to_string(arc).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://archives/{}", arc.archive_id))
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
                        let json = serde_json::to_string(map).unwrap_or_default();
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://features/{name}"))
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
                        let content = format!(
                            "id: {}\nname: {}\nversion: {}\ndescription: {}\n",
                            skill.id().as_str(),
                            skill.name(),
                            skill.version(),
                            skill.description()
                        );
                        entries.push(VfsDirectoryEntry::new(
                            ContextUri::new(format!("pandora://skills/{}", skill.id().as_str()))
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
                            entries.push(VfsDirectoryEntry::new(
                                ContextUri::new(format!(
                                    "pandora://index/{}/{}",
                                    target_intent,
                                    r.id().as_str()
                                ))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{MemoryIntent, MemoryScope, SopDefinition, SopStep, Timestamp};

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
}
