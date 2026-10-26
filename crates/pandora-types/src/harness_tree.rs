#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_TREE_FILE_SIZE_BYTES: usize = 512 * 1024; // 512 KB per file
pub const MAX_TREE_TOTAL_SIZE_BYTES: usize = 16 * 1024 * 1024; // 16 MB total tree
pub const MAX_TREE_FILES_COUNT: usize = 1024;

/// The 8 canonical harness components (c1..c8) structuring the agent's environment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessComponent {
    /// c1: Foundational operational charter, role constraints, and primary persona.
    SystemPrompt,
    /// c2: Inviolable boundaries, policy limits, security assertions, and prohibitions.
    Guardrails,
    /// c3: Dynamic context, episodic references, working state, and memory seeds.
    Memory,
    /// c4: Reusable procedural guidelines and SOP markdown playbooks (SKILL.md).
    Skills,
    /// c5: In-flight tool guidance, parameter formats, schemas, and heuristic tips.
    ToolGuidance,
    /// c6: Deterministic local executables, scripts, and helper automation.
    Scripts,
    /// c7: Specialist subagent definitions, capabilities, and delegation topologies.
    Subagents,
    /// c8: Middleware plugins, event hooks, interceptors, and environment adapters.
    Plugins,
}

impl HarnessComponent {
    pub const ALL: [Self; 8] = [
        Self::SystemPrompt,
        Self::Guardrails,
        Self::Memory,
        Self::Skills,
        Self::ToolGuidance,
        Self::Scripts,
        Self::Subagents,
        Self::Plugins,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::SystemPrompt => "c1",
            Self::Guardrails => "c2",
            Self::Memory => "c3",
            Self::Skills => "c4",
            Self::ToolGuidance => "c5",
            Self::Scripts => "c6",
            Self::Subagents => "c7",
            Self::Plugins => "c8",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system_prompt",
            Self::Guardrails => "guardrails",
            Self::Memory => "memory",
            Self::Skills => "skills",
            Self::ToolGuidance => "tool_guidance",
            Self::Scripts => "scripts",
            Self::Subagents => "subagents",
            Self::Plugins => "plugins",
        }
    }

    pub const fn canonical_prefix(self) -> &'static str {
        match self {
            Self::SystemPrompt => "prompts/",
            Self::Guardrails => "guardrails/",
            Self::Memory => "memory/",
            Self::Skills => "skills/",
            Self::ToolGuidance => "tools/",
            Self::Scripts => "scripts/",
            Self::Subagents => "subagents/",
            Self::Plugins => "plugins/",
        }
    }

    /// Classify a canonical normalized relative path into its corresponding component.
    pub fn classify_path(path: &str) -> Option<Self> {
        let p = path.trim_start_matches('/');
        if p.starts_with("prompts/") || p == "prompt.md" || p == "system.md" {
            Some(Self::SystemPrompt)
        } else if p.starts_with("guardrails/") || p == "guardrails.md" || p == "policy.md" {
            Some(Self::Guardrails)
        } else if p.starts_with("memory/") || p == "memory.md" || p == "context.md" {
            Some(Self::Memory)
        } else if p.starts_with("skills/") || p.ends_with("SKILL.md") {
            Some(Self::Skills)
        } else if p.starts_with("tools/") || p == "tools.md" || p == "tool_guidance.md" {
            Some(Self::ToolGuidance)
        } else if p.starts_with("scripts/") {
            Some(Self::Scripts)
        } else if p.starts_with("subagents/") || p.starts_with("agents/") {
            Some(Self::Subagents)
        } else if p.starts_with("plugins/") {
            Some(Self::Plugins)
        } else {
            None
        }
    }
}

impl fmt::Display for HarnessComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.id(), self.name())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessTreeError {
    EmptyPath,
    AbsolutePathForbidden(String),
    PathTraversalForbidden(String),
    BackslashForbidden(String),
    NullByteForbidden,
    FileTooLarge { path: String, bytes: usize },
    TotalSizeExceeded { total_bytes: usize },
    TooManyFiles(usize),
    InvalidComponent { path: String },
}

impl fmt::Display for HarnessTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("file path in harness tree cannot be empty"),
            Self::AbsolutePathForbidden(path) => {
                write!(formatter, "absolute path forbidden in harness tree: {path}")
            }
            Self::PathTraversalForbidden(path) => {
                write!(
                    formatter,
                    "path traversal (..) forbidden in harness tree: {path}"
                )
            }
            Self::BackslashForbidden(path) => {
                write!(formatter, "backslashes forbidden in harness tree: {path}")
            }
            Self::NullByteForbidden => formatter.write_str("null bytes forbidden in file paths"),
            Self::FileTooLarge { path, bytes } => {
                write!(
                    formatter,
                    "file '{path}' exceeds max size of {MAX_TREE_FILE_SIZE_BYTES} bytes (was {bytes})"
                )
            }
            Self::TotalSizeExceeded { total_bytes } => {
                write!(
                    formatter,
                    "total harness tree exceeds max size of {MAX_TREE_TOTAL_SIZE_BYTES} bytes (was {total_bytes})"
                )
            }
            Self::TooManyFiles(count) => {
                write!(
                    formatter,
                    "harness tree exceeds max file count of {MAX_TREE_FILES_COUNT} (was {count})"
                )
            }
            Self::InvalidComponent { path } => {
                write!(
                    formatter,
                    "path '{path}' cannot be classified into any canonical c1..c8 harness component"
                )
            }
        }
    }
}

impl std::error::Error for HarnessTreeError {}

/// Full editable harness tree consisting of c1..c8 component files.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HarnessTree {
    /// Canonical relative path -> file content.
    pub files: BTreeMap<String, String>,
    /// Metadata tags (e.g. author, benchmark, iteration).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl HarnessTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a file in the tree after validating path security and size.
    pub fn insert(
        &mut self,
        path: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<(), HarnessTreeError> {
        let p = path.into();
        let c = content.into();
        Self::validate_entry(&p, &c)?;
        self.files.insert(p, c);
        self.validate_totals()?;
        Ok(())
    }

    /// Remove a file from the tree.
    pub fn remove(&mut self, path: &str) -> Option<String> {
        self.files.remove(path)
    }

    /// Get content of a specific file.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// Check if a path exists.
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Total number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True if no files are in the tree.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Calculate total bytes across all files.
    pub fn total_bytes(&self) -> usize {
        self.files.values().map(|v| v.len()).sum()
    }

    /// Filter files belonging to a specific c1..c8 component.
    pub fn component_files(&self, component: HarnessComponent) -> BTreeMap<&str, &str> {
        self.files
            .iter()
            .filter(|(path, _)| HarnessComponent::classify_path(path) == Some(component))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Counts of files categorized by c1..c8 component.
    pub fn component_breakdown(&self) -> BTreeMap<HarnessComponent, usize> {
        let mut breakdown = BTreeMap::new();
        for component in HarnessComponent::ALL {
            breakdown.insert(component, 0);
        }
        for path in self.files.keys() {
            if let Some(comp) = HarnessComponent::classify_path(path) {
                *breakdown.entry(comp).or_insert(0) += 1;
            }
        }
        breakdown
    }

    /// Compute the omitted-is-inherited diff between this base tree and an updated tree.
    pub fn diff(&self, other: &Self) -> HarnessTreeDiff {
        let mut added = BTreeMap::new();
        let mut modified = BTreeMap::new();
        let mut removed = BTreeSet::new();

        for (path, new_content) in &other.files {
            match self.files.get(path) {
                None => {
                    added.insert(path.clone(), new_content.clone());
                }
                Some(old_content) if old_content != new_content => {
                    modified.insert(path.clone(), (old_content.clone(), new_content.clone()));
                }
                _ => {}
            }
        }

        for path in self.files.keys() {
            if !other.files.contains_key(path) {
                removed.insert(path.clone());
            }
        }

        HarnessTreeDiff {
            added,
            modified,
            removed,
        }
    }

    /// Apply an omitted-is-inherited diff to this tree to produce a refined tree.
    pub fn apply_diff(&self, diff: &HarnessTreeDiff) -> Result<Self, HarnessTreeError> {
        let mut new_tree = self.clone();

        for path in &diff.removed {
            new_tree.files.remove(path);
        }

        for (path, content) in &diff.added {
            Self::validate_entry(path, content)?;
            new_tree.files.insert(path.clone(), content.clone());
        }

        for (path, (_old, new)) in &diff.modified {
            Self::validate_entry(path, new)?;
            new_tree.files.insert(path.clone(), new.clone());
        }

        new_tree.validate()?;
        Ok(new_tree)
    }

    /// Deterministic SHA-256 digest of the tree.
    pub fn digest(&self) -> String {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(b"pandora-harness-tree-v1\0");
        for (path, content) in &self.files {
            buffer.extend_from_slice(path.as_bytes());
            buffer.push(0);
            buffer.extend_from_slice(content.as_bytes());
            buffer.push(0);
        }
        format!("htree-{}", crate::hash_artifact(&buffer))
    }

    /// Validate the complete tree.
    pub fn validate(&self) -> Result<(), HarnessTreeError> {
        if self.files.len() > MAX_TREE_FILES_COUNT {
            return Err(HarnessTreeError::TooManyFiles(self.files.len()));
        }
        let mut total = 0;
        for (path, content) in &self.files {
            Self::validate_entry(path, content)?;
            total += content.len();
        }
        if total > MAX_TREE_TOTAL_SIZE_BYTES {
            return Err(HarnessTreeError::TotalSizeExceeded { total_bytes: total });
        }
        Ok(())
    }

    fn validate_entry(path: &str, content: &str) -> Result<(), HarnessTreeError> {
        if path.is_empty() {
            return Err(HarnessTreeError::EmptyPath);
        }
        if path.contains('\0') {
            return Err(HarnessTreeError::NullByteForbidden);
        }
        if path.contains('\\') {
            return Err(HarnessTreeError::BackslashForbidden(path.to_owned()));
        }
        if path.starts_with('/') {
            return Err(HarnessTreeError::AbsolutePathForbidden(path.to_owned()));
        }
        for segment in path.split('/') {
            if segment == ".." {
                return Err(HarnessTreeError::PathTraversalForbidden(path.to_owned()));
            }
        }
        if HarnessComponent::classify_path(path).is_none() {
            return Err(HarnessTreeError::InvalidComponent {
                path: path.to_owned(),
            });
        }
        if content.len() > MAX_TREE_FILE_SIZE_BYTES {
            return Err(HarnessTreeError::FileTooLarge {
                path: path.to_owned(),
                bytes: content.len(),
            });
        }
        Ok(())
    }

    fn validate_totals(&self) -> Result<(), HarnessTreeError> {
        if self.files.len() > MAX_TREE_FILES_COUNT {
            return Err(HarnessTreeError::TooManyFiles(self.files.len()));
        }
        let total = self.total_bytes();
        if total > MAX_TREE_TOTAL_SIZE_BYTES {
            return Err(HarnessTreeError::TotalSizeExceeded { total_bytes: total });
        }
        Ok(())
    }
}

/// Omitted-is-inherited diff for component tree mutations.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HarnessTreeDiff {
    /// Newly added files (path -> content).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub added: BTreeMap<String, String>,
    /// Modified files (path -> (old_content, new_content)).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modified: BTreeMap<String, (String, String)>,
    /// Removed file paths.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed: BTreeSet<String>,
}

impl HarnessTreeDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.removed.is_empty()
    }

    pub fn total_changes(&self) -> usize {
        self.added.len() + self.modified.len() + self.removed.len()
    }

    /// Deterministic SHA-256 digest of the diff.
    pub fn digest(&self) -> String {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(b"pandora-harness-diff-v1\0");
        for (path, content) in &self.added {
            buffer.extend_from_slice(b"add\0");
            buffer.extend_from_slice(path.as_bytes());
            buffer.push(0);
            buffer.extend_from_slice(content.as_bytes());
            buffer.push(0);
        }
        for (path, (old, new)) in &self.modified {
            buffer.extend_from_slice(b"mod\0");
            buffer.extend_from_slice(path.as_bytes());
            buffer.push(0);
            buffer.extend_from_slice(old.as_bytes());
            buffer.push(0);
            buffer.extend_from_slice(new.as_bytes());
            buffer.push(0);
        }
        for path in &self.removed {
            buffer.extend_from_slice(b"del\0");
            buffer.extend_from_slice(path.as_bytes());
            buffer.push(0);
        }
        format!("hdiff-{}", crate::hash_artifact(&buffer))
    }
}

/// An entry in the persistent improvement harness library accumulated across tasks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HarnessLibraryEntry {
    pub id: String,
    pub task_id: String,
    pub tree: HarnessTree,
    pub adopted_at_utc: String,
    pub reflection_notes: String,
    pub pass_rate: u32, // 0..100
    pub token_cost: u64,
    pub latency_ms: u64,
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_classification_covers_all_eight_components() {
        assert_eq!(
            HarnessComponent::classify_path("prompts/system.md"),
            Some(HarnessComponent::SystemPrompt)
        );
        assert_eq!(
            HarnessComponent::classify_path("guardrails/safety.md"),
            Some(HarnessComponent::Guardrails)
        );
        assert_eq!(
            HarnessComponent::classify_path("memory/context.md"),
            Some(HarnessComponent::Memory)
        );
        assert_eq!(
            HarnessComponent::classify_path("skills/rust_refactor/SKILL.md"),
            Some(HarnessComponent::Skills)
        );
        assert_eq!(
            HarnessComponent::classify_path("tools/cargo.md"),
            Some(HarnessComponent::ToolGuidance)
        );
        assert_eq!(
            HarnessComponent::classify_path("scripts/verify_build.py"),
            Some(HarnessComponent::Scripts)
        );
        assert_eq!(
            HarnessComponent::classify_path("subagents/deep_coder.md"),
            Some(HarnessComponent::Subagents)
        );
        assert_eq!(
            HarnessComponent::classify_path("plugins/security_scanner/hooks.json"),
            Some(HarnessComponent::Plugins)
        );
    }

    #[test]
    fn tree_diff_and_apply_roundtrips_cleanly() {
        let mut base = HarnessTree::new();
        base.insert("prompts/system.md", "You are Pandora.")
            .unwrap();
        base.insert("guardrails/limits.md", "No destructive deletes.")
            .unwrap();
        base.insert("tools/cargo.md", "Use cargo clippy.").unwrap();

        let mut candidate = base.clone();
        // Modify
        candidate
            .insert("prompts/system.md", "You are Pandora, an autonomous agent.")
            .unwrap();
        // Add
        candidate
            .insert("skills/testing/SKILL.md", "# Testing Skill")
            .unwrap();
        // Remove
        candidate.remove("tools/cargo.md");

        let diff = base.diff(&candidate);
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);

        let reconstructed = base.apply_diff(&diff).unwrap();
        assert_eq!(reconstructed, candidate);
        assert_eq!(reconstructed.digest(), candidate.digest());
    }

    #[test]
    fn tree_validation_rejects_path_traversal_and_escapes() {
        let mut tree = HarnessTree::new();
        assert!(tree.insert("../etc/passwd", "malicious").is_err());
        assert!(tree.insert("prompts/../../secret.txt", "escape").is_err());
        assert!(tree.insert("prompts\\windows.md", "backslash").is_err());
        assert!(
            tree.insert("/prompts/absolute.md", "leading slash")
                .is_err()
        );
    }
}
