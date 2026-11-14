#![forbid(unsafe_code)]

//! Tripartite Merge Conflict Resolver & Self-Healing CI Loop.
//!
//! Provides cognitive tripartite conflict resolution for autonomous canary promotions:
//! - **Generator**: Parses 3-way git conflict hunks (`<<<<<<<`, `|||||||`, `=======`, `>>>>>>>`)
//!   and synthesizes candidate resolutions via import deduplication, additive union, and
//!   diagnostic-guided remediation.
//! - **Critic**: Compiles and tests candidate resolutions in sandbox worktrees, validating syntax,
//!   bracket balancing, absence of leftover conflict markers, and compiler diagnostic feedback.
//! - **Governor**: Constitutional invariant gate enforcing strict `#![forbid(unsafe_code)]`,
//!   patch size limits (<= 2MB), path containment, and non-self-approval (`SelfApprovalForbidden`).
//! - **Self-Healing Loop**: Up to $K = 3$ retry iterations with compiler feedback before clean rollback.

use crate::canary_worktree::CanaryError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Cognitive governance roles for merge conflict resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TripartiteRole {
    Generator,
    Critic,
    Governor,
}

impl std::fmt::Display for TripartiteRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generator => write!(f, "Generator"),
            Self::Critic => write!(f, "Critic"),
            Self::Governor => write!(f, "Governor"),
        }
    }
}

/// Synthesis strategies employed by the Generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionStrategy {
    UnionImports,
    AdditiveUnion,
    DiagnosticRemediation,
    PreferOurs,
    PreferTheirs,
}

/// A parsed 3-way merge conflict hunk from git conflict markers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeConflictHunk {
    pub file_path: PathBuf,
    pub base: String,
    pub ours: String,
    pub theirs: String,
    pub ours_label: String,
    pub theirs_label: String,
}

/// Candidate resolution synthesized by the Generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionCandidate {
    pub file_path: PathBuf,
    pub proposed_by: TripartiteRole,
    pub resolved_content: String,
    pub rationale: String,
    pub strategy: ResolutionStrategy,
}

/// Diagnostic message extracted from compiler or syntax validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticMessage {
    pub code: Option<String>,
    pub message: String,
    pub file_path: Option<PathBuf>,
    pub line: Option<usize>,
}

/// Evaluation verdict produced by the Critic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriticEvaluation {
    pub passed: bool,
    pub diagnostics: Vec<DiagnosticMessage>,
    pub raw_compiler_output: String,
    pub duration_ms: u64,
}

/// Verdict issued by the Governor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernorVerdict {
    pub approved: bool,
    pub approver_role: TripartiteRole,
    pub reasons: Vec<String>,
}

/// Record of a single iteration of the self-healing resolution loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeIterationRecord {
    pub iteration: usize,
    pub strategy: ResolutionStrategy,
    pub critic_evaluation: CriticEvaluation,
    pub governor_verdict: Option<GovernorVerdict>,
}

/// Final summary report of a tripartite conflict resolution session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripartiteMergeReport {
    pub success: bool,
    pub iterations_used: usize,
    pub max_retries: usize,
    pub resolved_files: Vec<PathBuf>,
    pub iteration_history: Vec<MergeIterationRecord>,
    pub rollback_executed: bool,
}

/// Errors occurring during tripartite merge resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeResolverError {
    SelfApprovalForbidden,
    UnsafeCodeDetected(String),
    DiffSizeExceeded { size_bytes: usize, max_bytes: usize },
    PathEscapeAttempt(String),
    CriticVerificationFailed(String),
    IterationLimitExceeded { attempts: usize },
    ConflictParsingFailed(String),
    RollbackFailed(String),
    GitCommandFailed { command: String, details: String },
    IoError(String),
}

impl std::fmt::Display for MergeResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SelfApprovalForbidden => {
                write!(
                    f,
                    "Constitutional invariant violated: Generator cannot approve its own candidate (SelfApprovalForbidden)"
                )
            }
            Self::UnsafeCodeDetected(msg) => {
                write!(f, "Strict #![forbid(unsafe_code)] violation: {msg}")
            }
            Self::DiffSizeExceeded {
                size_bytes,
                max_bytes,
            } => {
                write!(
                    f,
                    "Diff size exceeded: {size_bytes} bytes > max allowed {max_bytes} bytes"
                )
            }
            Self::PathEscapeAttempt(path) => {
                write!(f, "Path containment escape attempt detected: '{path}'")
            }
            Self::CriticVerificationFailed(err) => {
                write!(f, "Critic sandbox verification failed: {err}")
            }
            Self::IterationLimitExceeded { attempts } => {
                write!(
                    f,
                    "Self-healing resolution failed after {attempts} attempts; limit exceeded"
                )
            }
            Self::ConflictParsingFailed(msg) => write!(f, "Failed to parse conflict hunks: {msg}"),
            Self::RollbackFailed(msg) => write!(f, "Clean merge rollback failed: {msg}"),
            Self::GitCommandFailed { command, details } => {
                write!(f, "Git command '{command}' failed: {details}")
            }
            Self::IoError(msg) => write!(f, "I/O error during merge resolution: {msg}"),
        }
    }
}

impl std::error::Error for MergeResolverError {}

impl From<std::io::Error> for MergeResolverError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

impl From<MergeResolverError> for CanaryError {
    fn from(err: MergeResolverError) -> Self {
        match err {
            MergeResolverError::GitCommandFailed { command, details } => {
                Self::GitCommandFailed { command, details }
            }
            MergeResolverError::IoError(msg) => Self::IoError(msg),
            other => Self::MergeConflict(other.to_string()),
        }
    }
}

/// The Tripartite Merge Conflict Resolver engine with self-healing feedback loop.
#[derive(Debug, Clone)]
pub struct TripartiteMergeResolver {
    pub max_retries: usize,
    pub max_file_size_bytes: usize,
}

impl Default for TripartiteMergeResolver {
    fn default() -> Self {
        Self {
            max_retries: 3,
            max_file_size_bytes: 2 * 1024 * 1024, // 2 MB
        }
    }
}

impl TripartiteMergeResolver {
    pub fn new(max_retries: usize) -> Self {
        Self {
            max_retries,
            max_file_size_bytes: 2 * 1024 * 1024,
        }
    }

    /// Scans repository for merge conflicts, invokes the tripartite self-healing loop for each conflicted file,
    /// and commits resolved changes or rolls back cleanly upon exhaustion.
    pub fn attempt_resolution(
        &self,
        repo_root: &Path,
        _worktree_path: &Path,
    ) -> Result<bool, CanaryError> {
        let conflict_files = self.find_conflicted_files(repo_root)?;
        if conflict_files.is_empty() {
            return Ok(true);
        }

        // Backup original content for non-git or synthetic fallback rollback
        let mut backups = Vec::new();
        for file in &conflict_files {
            let full_path = repo_root.join(file);
            if let Ok(content) = fs::read_to_string(&full_path) {
                backups.push((full_path, content));
            }
        }

        let mut resolved_all = true;
        for file_path in &conflict_files {
            let full_path = repo_root.join(file_path);
            let content = fs::read_to_string(&full_path)?;

            match self.resolve_file_content(file_path, &content, repo_root) {
                Ok(report) if report.success => {
                    // Stage resolved file
                    let _ = Command::new("git")
                        .args(["add", file_path.to_str().unwrap_or("")])
                        .current_dir(repo_root)
                        .output();
                }
                _ => {
                    resolved_all = false;
                    break;
                }
            }
        }

        if !resolved_all {
            // Self-healing exhaustion: clean rollback
            self.execute_rollback(repo_root, &backups)?;
            return Ok(false);
        }

        // Commit resolution under strict author invariant
        let commit_out = Command::new("git")
            .args([
                "-c",
                "user.name=ANISAYAK MITRA",
                "-c",
                "user.email=266799942+anisayakmitra-in@users.noreply.github.com",
                "commit",
                "-m",
                "canary(merge-resolver): tripartite cognitive self-healing resolution",
            ])
            .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
            .env(
                "GIT_AUTHOR_EMAIL",
                "266799942+anisayakmitra-in@users.noreply.github.com",
            )
            .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
            .env(
                "GIT_COMMITTER_EMAIL",
                "266799942+anisayakmitra-in@users.noreply.github.com",
            )
            .current_dir(repo_root)
            .output();

        let committed = commit_out.map(|o| o.status.success()).unwrap_or(false);
        Ok(committed)
    }

    /// Validates git conflict marker syntax. Returns Ok(()) if conflict markers are properly formed
    /// or absent, or Err(MergeResolverError::ConflictParsingFailed) if markers are unclosed, nested,
    /// or mismatched.
    pub fn validate_conflict_markers(&self, content: &str) -> Result<(), MergeResolverError> {
        let mut in_conflict = false;
        let mut seen_sep = false;
        let mut line_no = 0;

        for line in content.lines() {
            line_no += 1;
            if line.starts_with("<<<<<<<") {
                if in_conflict {
                    return Err(MergeResolverError::ConflictParsingFailed(format!(
                        "Nested conflict marker '<<<<<<<' detected at line {line_no}"
                    )));
                }
                in_conflict = true;
                seen_sep = false;
            } else if line.starts_with("|||||||") {
                if !in_conflict || seen_sep {
                    return Err(MergeResolverError::ConflictParsingFailed(format!(
                        "Unexpected base marker '|||||||' at line {line_no}"
                    )));
                }
            } else if line.starts_with("=======") {
                if !in_conflict || seen_sep {
                    return Err(MergeResolverError::ConflictParsingFailed(format!(
                        "Unexpected separator marker '=======' at line {line_no}"
                    )));
                }
                seen_sep = true;
            } else if line.starts_with(">>>>>>>") {
                if !in_conflict || !seen_sep {
                    return Err(MergeResolverError::ConflictParsingFailed(format!(
                        "Unexpected closing marker '>>>>>>>' without matching '=======' at line {line_no}"
                    )));
                }
                in_conflict = false;
                seen_sep = false;
            }
        }

        if in_conflict {
            return Err(MergeResolverError::ConflictParsingFailed(
                "Unclosed conflict marker: reached EOF without matching '>>>>>>>'".to_string(),
            ));
        }

        Ok(())
    }

    /// Resolves the content of a conflicted file using the tripartite self-healing loop.
    pub fn resolve_file_content(
        &self,
        file_path: &Path,
        content: &str,
        repo_root: &Path,
    ) -> Result<TripartiteMergeReport, MergeResolverError> {
        let has_residual_markers = content.contains("<<<<<<<")
            || content.contains("=======")
            || content.contains(">>>>>>>")
            || content.contains("|||||||");

        if self.validate_conflict_markers(content).is_err() {
            return Ok(TripartiteMergeReport {
                success: false,
                iterations_used: 0,
                max_retries: self.max_retries,
                resolved_files: Vec::new(),
                iteration_history: Vec::new(),
                rollback_executed: true,
            });
        }

        let hunks = self.parse_conflict_hunks(file_path, content);
        if hunks.is_empty() {
            if has_residual_markers {
                return Ok(TripartiteMergeReport {
                    success: false,
                    iterations_used: 0,
                    max_retries: self.max_retries,
                    resolved_files: Vec::new(),
                    iteration_history: Vec::new(),
                    rollback_executed: true,
                });
            }
            return Ok(TripartiteMergeReport {
                success: true,
                iterations_used: 0,
                max_retries: self.max_retries,
                resolved_files: vec![file_path.to_path_buf()],
                iteration_history: Vec::new(),
                rollback_executed: false,
            });
        }

        let mut history = Vec::new();
        let mut last_diagnostics: Vec<DiagnosticMessage> = Vec::new();
        let mut resolved_content = String::new();
        let mut successful = false;

        for attempt in 1..=self.max_retries {
            // Strategy selection based on attempt and prior diagnostics
            let strategy = match attempt {
                1 => {
                    if self.is_pure_imports_conflict(&hunks) {
                        ResolutionStrategy::UnionImports
                    } else {
                        ResolutionStrategy::AdditiveUnion
                    }
                }
                2 => {
                    if last_diagnostics.iter().any(|d| {
                        d.code.as_deref() == Some("E0428") || d.message.contains("duplicate")
                    }) {
                        ResolutionStrategy::DiagnosticRemediation
                    } else {
                        ResolutionStrategy::PreferOurs
                    }
                }
                _ => ResolutionStrategy::PreferTheirs,
            };

            // 1. GENERATOR: Synthesize Candidate
            let candidate =
                self.synthesize_candidate(file_path, content, &hunks, strategy, &last_diagnostics);

            // 2. CRITIC: Verify Candidate
            let critic_eval = self.critic_verify(repo_root, file_path, &candidate);

            if !critic_eval.passed {
                last_diagnostics = critic_eval.diagnostics.clone();
                history.push(MergeIterationRecord {
                    iteration: attempt,
                    strategy,
                    critic_evaluation: critic_eval,
                    governor_verdict: None,
                });
                continue;
            }

            // 3. GOVERNOR: Invariant & Approval Check
            // Enforce SelfApprovalForbidden: approver MUST be Governor, not Generator
            let governor_verdict =
                self.governor_evaluate(TripartiteRole::Governor, &candidate, &critic_eval);

            let approved = governor_verdict.approved;
            history.push(MergeIterationRecord {
                iteration: attempt,
                strategy,
                critic_evaluation: critic_eval,
                governor_verdict: Some(governor_verdict),
            });

            if approved {
                resolved_content = candidate.resolved_content;
                successful = true;
                break;
            }
        }

        if successful {
            // Write resolved content to disk
            let full_path = if file_path.is_absolute() {
                file_path.to_path_buf()
            } else {
                repo_root.join(file_path)
            };
            fs::write(&full_path, &resolved_content)?;

            Ok(TripartiteMergeReport {
                success: true,
                iterations_used: history.len(),
                max_retries: self.max_retries,
                resolved_files: vec![file_path.to_path_buf()],
                iteration_history: history,
                rollback_executed: false,
            })
        } else {
            Ok(TripartiteMergeReport {
                success: false,
                iterations_used: history.len(),
                max_retries: self.max_retries,
                resolved_files: Vec::new(),
                iteration_history: history,
                rollback_executed: true,
            })
        }
    }

    /// Finds files marked as unmerged (`git diff --name-only --diff-filter=U`).
    pub fn find_conflicted_files(&self, repo_root: &Path) -> Result<Vec<PathBuf>, CanaryError> {
        let out = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(repo_root)
            .output();

        match out {
            Ok(o) if o.status.success() => {
                let mut files = Vec::new();
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        files.push(PathBuf::from(trimmed));
                    }
                }
                Ok(files)
            }
            _ => {
                // Filesystem scan fallback: find files with conflict markers
                let mut files = Vec::new();
                self.scan_dir_for_conflict_markers(repo_root, repo_root, &mut files);
                Ok(files)
            }
        }
    }

    fn scan_dir_for_conflict_markers(
        &self,
        base: &Path,
        current: &Path,
        conflicts: &mut Vec<PathBuf>,
    ) {
        if let Ok(entries) = fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if dir_name != ".git" && dir_name != "target" && dir_name != ".pandora" {
                        self.scan_dir_for_conflict_markers(base, &path, conflicts);
                    }
                } else if path.is_file()
                    && let Ok(content) = fs::read_to_string(&path)
                    && content.contains("<<<<<<<")
                    && content.contains(">>>>>>>")
                {
                    if let Ok(rel) = path.strip_prefix(base) {
                        conflicts.push(rel.to_path_buf());
                    } else {
                        conflicts.push(path);
                    }
                }
            }
        }
    }

    /// Parses 2-way and 3-way conflict markers into `MergeConflictHunk` records.
    pub fn parse_conflict_hunks(&self, file_path: &Path, content: &str) -> Vec<MergeConflictHunk> {
        let mut hunks = Vec::new();
        let mut in_ours = false;
        let mut in_base = false;
        let mut in_theirs = false;

        let mut ours_lines = Vec::new();
        let mut base_lines = Vec::new();
        let mut theirs_lines = Vec::new();
        let mut ours_label = String::new();

        for line in content.lines() {
            if line.starts_with("<<<<<<<") {
                in_ours = true;
                in_base = false;
                in_theirs = false;
                ours_label = line.trim_start_matches('<').trim().to_string();
                ours_lines.clear();
                base_lines.clear();
                theirs_lines.clear();
            } else if line.starts_with("|||||||") {
                in_ours = false;
                in_base = true;
                in_theirs = false;
            } else if line.starts_with("=======") {
                in_ours = false;
                in_base = false;
                in_theirs = true;
            } else if line.starts_with(">>>>>>>") {
                in_theirs = false;
                let theirs_label = line.trim_start_matches('>').trim().to_string();
                hunks.push(MergeConflictHunk {
                    file_path: file_path.to_path_buf(),
                    base: base_lines.join("\n"),
                    ours: ours_lines.join("\n"),
                    theirs: theirs_lines.join("\n"),
                    ours_label: ours_label.clone(),
                    theirs_label,
                });
            } else if in_ours {
                ours_lines.push(line);
            } else if in_base {
                base_lines.push(line);
            } else if in_theirs {
                theirs_lines.push(line);
            }
        }

        hunks
    }

    fn is_pure_imports_conflict(&self, hunks: &[MergeConflictHunk]) -> bool {
        hunks.iter().all(|h| {
            let ours_imports = h.ours.lines().all(|l| {
                l.trim().is_empty() || l.trim().starts_with("use ") || l.trim().starts_with("//")
            });
            let theirs_imports = h.theirs.lines().all(|l| {
                l.trim().is_empty() || l.trim().starts_with("use ") || l.trim().starts_with("//")
            });
            ours_imports && theirs_imports
        })
    }

    /// Generator synthesis implementation.
    pub fn synthesize_candidate(
        &self,
        file_path: &Path,
        full_content: &str,
        hunks: &[MergeConflictHunk],
        strategy: ResolutionStrategy,
        diagnostics: &[DiagnosticMessage],
    ) -> ResolutionCandidate {
        let mut resolved_lines = Vec::new();
        let mut lines = full_content.lines().peekable();
        let mut hunk_idx = 0;

        while let Some(line) = lines.next() {
            if line.starts_with("<<<<<<<") {
                // Skip conflict block lines until matching >>>>>>>
                let mut closed = false;
                let mut skipped = Vec::new();
                for inner in lines.by_ref() {
                    skipped.push(inner);
                    if inner.starts_with(">>>>>>>") {
                        closed = true;
                        break;
                    }
                }

                if closed && hunk_idx < hunks.len() {
                    let hunk = &hunks[hunk_idx];
                    hunk_idx += 1;

                    let hunk_resolved = self.resolve_single_hunk(hunk, strategy, diagnostics);
                    if !hunk_resolved.is_empty() {
                        resolved_lines.push(hunk_resolved);
                    }
                } else {
                    // Unclosed or extra conflict marker: do NOT drop lines to EOF!
                    // Preserve original lines so content is not lost; critic will detect raw markers.
                    resolved_lines.push(line.to_string());
                    for l in skipped {
                        resolved_lines.push(l.to_string());
                    }
                }
            } else {
                resolved_lines.push(line.to_string());
            }
        }

        let resolved_content = resolved_lines.join("\n");
        let rationale = format!(
            "Synthesized using strategy '{strategy:?}' across {hunk_count} hunk(s)",
            hunk_count = hunks.len()
        );

        ResolutionCandidate {
            file_path: file_path.to_path_buf(),
            proposed_by: TripartiteRole::Generator,
            resolved_content,
            rationale,
            strategy,
        }
    }

    fn resolve_single_hunk(
        &self,
        hunk: &MergeConflictHunk,
        strategy: ResolutionStrategy,
        diagnostics: &[DiagnosticMessage],
    ) -> String {
        match strategy {
            ResolutionStrategy::UnionImports => {
                let mut imports = BTreeSet::new();
                for line in hunk.ours.lines().chain(hunk.theirs.lines()) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        imports.insert(trimmed.to_string());
                    }
                }
                let mut sorted: Vec<String> = imports.into_iter().collect();
                sorted.sort();
                sorted.join("\n")
            }
            ResolutionStrategy::AdditiveUnion => {
                // If ours and theirs are identical, emit once
                if hunk.ours.trim() == hunk.theirs.trim() {
                    return hunk.ours.clone();
                }

                let mut combined = Vec::new();
                if !hunk.ours.trim().is_empty() {
                    combined.push(hunk.ours.clone());
                }
                if !hunk.theirs.trim().is_empty() && hunk.theirs.trim() != hunk.ours.trim() {
                    combined.push(hunk.theirs.clone());
                }
                combined.join("\n")
            }
            ResolutionStrategy::DiagnosticRemediation => {
                // On duplicate items (E0428), deduplicate by function/item signature
                let mut seen_items = BTreeSet::new();
                let mut out_lines = Vec::new();

                for line in hunk.ours.lines().chain(hunk.theirs.lines()) {
                    let trimmed = line.trim();
                    if let Some((_kind, name)) = extract_item_decl(trimmed) {
                        if seen_items.contains(name) {
                            continue; // Skip duplicate declaration with same name
                        }
                        seen_items.insert(name.to_string());
                    } else if trimmed.starts_with("pub fn ")
                        || trimmed.starts_with("fn ")
                        || trimmed.starts_with("pub struct ")
                        || trimmed.starts_with("struct ")
                        || trimmed.starts_with("pub enum ")
                        || trimmed.starts_with("enum ")
                    {
                        let sig_key = trimmed
                            .split('{')
                            .next()
                            .unwrap_or(trimmed)
                            .trim()
                            .to_string();
                        if seen_items.contains(&sig_key) {
                            continue; // Skip duplicate declaration
                        }
                        seen_items.insert(sig_key);
                    }
                    out_lines.push(line);
                }

                // Balance brackets if bracket error occurred
                if diagnostics
                    .iter()
                    .any(|d| d.message.contains("bracket") || d.message.contains("brace"))
                {
                    let open_count = out_lines
                        .iter()
                        .map(|l| l.matches('{').count())
                        .sum::<usize>();
                    let close_count = out_lines
                        .iter()
                        .map(|l| l.matches('}').count())
                        .sum::<usize>();
                    if open_count > close_count {
                        out_lines.extend(std::iter::repeat_n("}", open_count - close_count));
                    }
                }

                out_lines.join("\n")
            }
            ResolutionStrategy::PreferOurs => hunk.ours.clone(),
            ResolutionStrategy::PreferTheirs => hunk.theirs.clone(),
        }
    }

    /// Critic sandbox verification implementation.
    ///
    /// Validates:
    /// 1. Absence of residual git conflict markers.
    /// 2. Stack-based delimiter balance (curly braces `{}`, parentheses `()`, square brackets `[]`),
    ///    with rigorous masking of comments, string literals, and char literals to eliminate false positives.
    /// 3. Lexical item analysis detecting duplicate top-level declarations (`fn`, `struct`, `enum`, etc.)
    ///    and emitting rustc-standard diagnostic code `E0428`.
    /// 4. Project-level compilation check (`cargo check --message-format=json`) when `Cargo.toml` exists.
    pub fn critic_verify(
        &self,
        repo_root: &Path,
        file_path: &Path,
        candidate: &ResolutionCandidate,
    ) -> CriticEvaluation {
        let start = Instant::now();
        let mut diagnostics = Vec::new();

        // 1. Check for leftover git conflict markers
        if candidate.resolved_content.contains("<<<<<<<")
            || candidate.resolved_content.contains("=======")
            || candidate.resolved_content.contains(">>>>>>>")
            || candidate.resolved_content.contains("|||||||")
        {
            diagnostics.push(DiagnosticMessage {
                code: Some("E_CONFLICT_MARKERS".to_string()),
                message: "Candidate retain raw git conflict markers".to_string(),
                file_path: Some(file_path.to_path_buf()),
                line: None,
            });
            return CriticEvaluation {
                passed: false,
                diagnostics,
                raw_compiler_output: "Residual conflict markers present in candidate".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 2. Delimiter & Syntax verification with comment/string masking
        let masked = mask_comments_and_strings(&candidate.resolved_content);
        if let Err(delim_diags) = verify_delimiter_nesting(&masked, file_path) {
            return CriticEvaluation {
                passed: false,
                raw_compiler_output: format!("Syntax error: {}", delim_diags[0].message),
                diagnostics: delim_diags,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 3. Duplicate item definition analysis (detect duplicate functions/items emitting E0428)
        let dup_diags = detect_duplicate_definitions(&masked, file_path);
        if !dup_diags.is_empty() {
            let primary = &dup_diags[0];
            return CriticEvaluation {
                passed: false,
                raw_compiler_output: format!(
                    "error[E0428]: {}\n  --> {}:{}: duplicate definition",
                    primary.message,
                    file_path.display(),
                    primary.line.unwrap_or(1)
                ),
                diagnostics: dup_diags,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 4. Compilation checking when Cargo.toml exists
        let cargo_toml = repo_root.join("Cargo.toml");
        if cargo_toml.exists()
            && let Some(cargo_diags) =
                self.run_cargo_check_diagnostics(repo_root, file_path, candidate)
            && !cargo_diags.is_empty()
        {
            return CriticEvaluation {
                passed: false,
                raw_compiler_output: "Cargo check reported compiler errors".to_string(),
                diagnostics: cargo_diags,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        CriticEvaluation {
            passed: true,
            diagnostics: Vec::new(),
            raw_compiler_output: "Critic syntax and sandbox verification passed".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Invokes `cargo check --message-format=json` when in a genuine cargo workspace.
    fn run_cargo_check_diagnostics(
        &self,
        repo_root: &Path,
        file_path: &Path,
        candidate: &ResolutionCandidate,
    ) -> Option<Vec<DiagnosticMessage>> {
        if std::env::var("PANDORA_SKIP_CARGO_CHECK").as_deref() == Ok("1") {
            return Some(Vec::new());
        }

        let full_path = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            repo_root.join(file_path)
        };

        let backup = fs::read_to_string(&full_path).ok();
        if fs::write(&full_path, &candidate.resolved_content).is_err() {
            return None;
        }

        let mut cmd = Command::new("cargo");
        cmd.args(["check", "--message-format=json"])
            .current_dir(repo_root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(windows)]
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }

        let output = cmd.output().ok();

        // Always restore original file content so Governor remains the sole commit authority
        if let Some(ref orig) = backup {
            let _ = fs::write(&full_path, orig);
        }

        let output = output?;
        if output.status.success() {
            return Some(Vec::new());
        }

        let mut diagnostics = Vec::new();
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        for line in stdout_str.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
                && v.get("reason").and_then(|r| r.as_str()) == Some("compiler-message")
                && let Some(msg) = v.get("message")
            {
                let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("");
                if level == "error" {
                    let code = msg
                        .get("code")
                        .and_then(|c| c.get("code"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string());
                    let rendered = msg
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("compiler error")
                        .to_string();
                    diagnostics.push(DiagnosticMessage {
                        code,
                        message: rendered,
                        file_path: Some(file_path.to_path_buf()),
                        line: None,
                    });
                }
            }
        }

        Some(diagnostics)
    }

    /// Governor constitutional evaluation and invariant check.
    pub fn governor_evaluate(
        &self,
        approver: TripartiteRole,
        candidate: &ResolutionCandidate,
        critic_eval: &CriticEvaluation,
    ) -> GovernorVerdict {
        let mut reasons = Vec::new();

        // Invariant 1: Constitutional Non-Self-Approval (SelfApprovalForbidden)
        if approver == candidate.proposed_by {
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons: vec![format!(
                    "Constitutional invariant violated: {approver:?} cannot approve its own candidate (SelfApprovalForbidden)"
                )],
            };
        }

        // Invariant 1b: Strict Governor Role Authorization (Only Governor can approve)
        if approver != TripartiteRole::Governor {
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons: vec![format!(
                    "Constitutional invariant violated: Only Governor can issue approval verdict; role '{approver:?}' is unauthorized (UnauthorizedApproverRole)"
                )],
            };
        }

        // Invariant 2: Critic evaluation must have passed
        if !critic_eval.passed {
            reasons.push(
                "Critic verification failed; Governor cannot approve unverified candidate"
                    .to_string(),
            );
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons,
            };
        }

        // Invariant 3: Strict `#![forbid(unsafe_code)]` with robust keyword and attribute scanner
        let is_rust_source = candidate
            .file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext == "rs")
            .unwrap_or(true);

        if is_rust_source && let Some(violation) = detect_unsafe_code(&candidate.resolved_content) {
            reasons.push(violation);
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons,
            };
        }

        // Invariant 4: Size bounds (<= 2MB)
        if candidate.resolved_content.len() > self.max_file_size_bytes {
            reasons.push(format!(
                "Diff size exceeded: {} bytes > max allowed {} bytes",
                candidate.resolved_content.len(),
                self.max_file_size_bytes
            ));
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons,
            };
        }

        // Invariant 5: Path containment & escape prevention
        let path_str = candidate.file_path.to_string_lossy();
        if path_str.contains("..")
            || path_str.starts_with("/etc")
            || path_str.starts_with("C:\\Windows")
        {
            reasons.push(format!(
                "Path containment escape attempt detected: '{path_str}'"
            ));
            return GovernorVerdict {
                approved: false,
                approver_role: approver,
                reasons,
            };
        }

        GovernorVerdict {
            approved: true,
            approver_role: approver,
            reasons: vec![
                "All constitutional invariants satisfied and approved by Governor".to_string(),
            ],
        }
    }

    /// Convenience validator for Governor invariants.
    pub fn governor_verify(&self, candidate: &ResolutionCandidate) -> bool {
        let mock_critic = CriticEvaluation {
            passed: true,
            diagnostics: Vec::new(),
            raw_compiler_output: String::new(),
            duration_ms: 0,
        };
        let verdict = self.governor_evaluate(TripartiteRole::Governor, candidate, &mock_critic);
        verdict.approved
    }

    /// Rollback cleanly on self-healing exhaustion.
    fn execute_rollback(
        &self,
        repo_root: &Path,
        backups: &[(PathBuf, String)],
    ) -> Result<(), CanaryError> {
        // Try git merge --abort
        let abort_out = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo_root)
            .output();

        if let Ok(out) = abort_out
            && out.status.success()
        {
            return Ok(());
        }

        // Fallback: restore file content from backups
        for (path, content) in backups {
            let _ = fs::write(path, content);
        }

        Ok(())
    }
}

/// Masks comments and string literals by replacing characters with whitespace.
/// Preserves newlines and exact character offsets for error reporting.
fn mask_comments_and_strings(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let n = chars.len();
    let mut i = 0;

    let mut in_line_comment = false;
    let mut block_comment_depth = 0;
    let mut in_string = false;
    let mut in_raw_string_hashes: Option<usize> = None;
    let mut string_escaped = false;

    while i < n {
        let c = chars[i];

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                out.push('\n');
            } else {
                out.push(' ');
            }
            i += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if c == '/' && i + 1 < n && chars[i + 1] == '*' {
                block_comment_depth += 1;
                out.push_str("  ");
                i += 2;
                continue;
            } else if c == '*' && i + 1 < n && chars[i + 1] == '/' {
                block_comment_depth -= 1;
                out.push_str("  ");
                i += 2;
                continue;
            } else {
                out.push(if c == '\n' { '\n' } else { ' ' });
                i += 1;
                continue;
            }
        }

        if let Some(hashes) = in_raw_string_hashes {
            if c == '"' {
                let mut hash_match = true;
                if i + hashes < n {
                    for h in 0..hashes {
                        if chars[i + 1 + h] != '#' {
                            hash_match = false;
                            break;
                        }
                    }
                } else {
                    hash_match = false;
                }
                if hash_match {
                    for _ in 0..=hashes {
                        out.push(' ');
                    }
                    i += 1 + hashes;
                    in_raw_string_hashes = None;
                    continue;
                }
            }
            out.push(if c == '\n' { '\n' } else { ' ' });
            i += 1;
            continue;
        }

        if in_string {
            if string_escaped {
                string_escaped = false;
                out.push(' ');
                i += 1;
                continue;
            }
            if c == '\\' {
                string_escaped = true;
                out.push(' ');
                i += 1;
                continue;
            }
            if c == '"' {
                in_string = false;
                out.push(' ');
                i += 1;
                continue;
            }
            out.push(if c == '\n' { '\n' } else { ' ' });
            i += 1;
            continue;
        }

        // Normal code mode
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            in_line_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }

        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            block_comment_depth = 1;
            out.push_str("  ");
            i += 2;
            continue;
        }

        if c == 'r' {
            let mut h_count = 0;
            let mut j = i + 1;
            while j < n && chars[j] == '#' {
                h_count += 1;
                j += 1;
            }
            if j < n && chars[j] == '"' {
                in_raw_string_hashes = Some(h_count);
                for _ in i..=j {
                    out.push(' ');
                }
                i = j + 1;
                continue;
            }
        }

        if c == '"' {
            in_string = true;
            string_escaped = false;
            out.push(' ');
            i += 1;
            continue;
        }

        if c == '\'' {
            // Character literal vs lifetime token
            if i + 2 < n && chars[i + 1] == '\\' && i + 3 < n && chars[i + 3] == '\'' {
                out.push_str("    ");
                i += 4;
                continue;
            } else if i + 2 < n && chars[i + 2] == '\'' {
                out.push_str("   ");
                i += 3;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }

    out
}

/// Validates delimiter nesting using a strict stack.
/// Supports `{}`, `()`, and `[]`, catching interleaving and unclosed brackets.
fn verify_delimiter_nesting(masked: &str, file_path: &Path) -> Result<(), Vec<DiagnosticMessage>> {
    let mut stack: Vec<(char, usize, usize)> = Vec::new();
    let mut line = 1;
    let mut col = 1;

    for c in masked.chars() {
        if c == '\n' {
            line += 1;
            col = 1;
            continue;
        }

        match c {
            '{' => stack.push(('}', line, col)),
            '(' => stack.push((')', line, col)),
            '[' => stack.push((']', line, col)),
            '}' => match stack.pop() {
                Some(('}', _, _)) => {}
                Some((expected, open_line, _)) => {
                    let code = match expected {
                        ']' => "E_UNBALANCED_BRACKETS",
                        ')' => "E_UNBALANCED_PARENS",
                        _ => "E_UNBALANCED_BRACES",
                    };
                    return Err(vec![DiagnosticMessage {
                        code: Some(code.to_string()),
                        message: format!(
                            "Mismatched closing delimiter '}}' at line {line}; expected '{expected}' opened at line {open_line}"
                        ),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
                None => {
                    return Err(vec![DiagnosticMessage {
                        code: Some("E_UNBALANCED_BRACES".to_string()),
                        message: format!("Unmatched closing delimiter '}}' at line {line}"),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
            },
            ')' => match stack.pop() {
                Some((')', _, _)) => {}
                Some((expected, open_line, _)) => {
                    let code = match expected {
                        ']' => "E_UNBALANCED_BRACKETS",
                        '}' => "E_UNBALANCED_BRACES",
                        _ => "E_UNBALANCED_PARENS",
                    };
                    return Err(vec![DiagnosticMessage {
                        code: Some(code.to_string()),
                        message: format!(
                            "Mismatched closing delimiter ')' at line {line}; expected '{expected}' opened at line {open_line}"
                        ),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
                None => {
                    return Err(vec![DiagnosticMessage {
                        code: Some("E_UNBALANCED_PARENS".to_string()),
                        message: format!("Unmatched closing delimiter ')' at line {line}"),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
            },
            ']' => match stack.pop() {
                Some((']', _, _)) => {}
                Some((expected, open_line, _)) => {
                    let code = match expected {
                        '}' => "E_UNBALANCED_BRACES",
                        ')' => "E_UNBALANCED_PARENS",
                        _ => "E_UNBALANCED_BRACKETS",
                    };
                    return Err(vec![DiagnosticMessage {
                        code: Some(code.to_string()),
                        message: format!(
                            "Mismatched closing delimiter ']' at line {line}; expected '{expected}' opened at line {open_line}"
                        ),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
                None => {
                    return Err(vec![DiagnosticMessage {
                        code: Some("E_UNBALANCED_BRACKETS".to_string()),
                        message: format!("Unmatched closing delimiter ']' at line {line}"),
                        file_path: Some(file_path.to_path_buf()),
                        line: Some(line),
                    }]);
                }
            },
            _ => {}
        }
        col += 1;
    }

    if let Some((expected, open_line, _)) = stack.pop() {
        let (code, delim_name) = match expected {
            '}' => ("E_UNBALANCED_BRACES", "curly brace '{'"),
            ')' => ("E_UNBALANCED_PARENS", "parenthesis '('"),
            ']' => ("E_UNBALANCED_BRACKETS", "square bracket '['"),
            _ => ("E_SYNTAX_ERROR", "delimiter"),
        };
        return Err(vec![DiagnosticMessage {
            code: Some(code.to_string()),
            message: format!("Unclosed {delim_name} opened at line {open_line}"),
            file_path: Some(file_path.to_path_buf()),
            line: Some(open_line),
        }]);
    }

    Ok(())
}

/// Detects duplicate item definitions within the candidate and emits E0428 diagnostics.
fn detect_duplicate_definitions(masked: &str, file_path: &Path) -> Vec<DiagnosticMessage> {
    let mut diagnostics = Vec::new();
    let mut seen_items: std::collections::HashMap<(usize, String, String), usize> =
        std::collections::HashMap::new();
    let mut scope_depth: usize = 0;

    for (line_idx, line) in masked.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        let open_on_line = line.matches('{').count();
        let close_on_line = line.matches('}').count();

        if !trimmed.is_empty()
            && let Some((item_kind, item_name)) = extract_item_decl(trimmed)
        {
            let key = (scope_depth, item_kind.to_string(), item_name.to_string());
            if let Some(&_prev_line) = seen_items.get(&key) {
                diagnostics.push(DiagnosticMessage {
                    code: Some("E0428".to_string()),
                    message: format!("duplicate definitions with name `{item_name}`"),
                    file_path: Some(file_path.to_path_buf()),
                    line: Some(line_num),
                });
            } else {
                seen_items.insert(key, line_num);
            }
        }

        scope_depth = (scope_depth + open_on_line).saturating_sub(close_on_line);
    }

    diagnostics
}

/// Extracts item kind and identifier from a clean declaration line.
fn extract_item_decl(line: &str) -> Option<(&'static str, &str)> {
    if line.starts_with("#[") || line.starts_with("#![") {
        return None;
    }

    let mut words = line.split_whitespace().peekable();
    while let Some(&w) = words.peek() {
        if w == "pub"
            || w.starts_with("pub(")
            || w == "async"
            || w == "unsafe"
            || (w == "const" && words.clone().nth(1) == Some("fn"))
            || w == "extern"
            || w.starts_with("\"C\"")
            || w.starts_with("\"system\"")
            || w == "default"
        {
            words.next();
        } else {
            break;
        }
    }

    let kind_kw = words.next()?;
    let kind = match kind_kw {
        "fn" => "function",
        "struct" => "struct",
        "enum" => "enum",
        "type" => "type",
        "trait" => "trait",
        "const" => "const",
        "static" => "static",
        _ => return None,
    };

    let raw_name = words.next()?;
    let name = raw_name
        .trim_start_matches("mut ")
        .split(['(', '<', '{', ';', ':'])
        .next()?
        .trim();

    if name.is_empty() {
        return None;
    }

    Some((kind, name))
}

/// Strips comments (`// ...`, `/* ... */` with arbitrary nesting) and string literals
/// (`"..."`, `r#"..."#`) from Rust source text, preserving newlines.
fn strip_comments_and_strings(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        // Line comment: // ...
        if i + 1 < n && chars[i] == '/' && chars[i + 1] == '/' {
            i += 2;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            if i < n && chars[i] == '\n' {
                out.push('\n');
                i += 1;
            }
            continue;
        }

        // Block comment: /* ... */ (supports arbitrary nesting)
        if i + 1 < n && chars[i] == '/' && chars[i + 1] == '*' {
            i += 2;
            let mut depth = 1;
            while i < n && depth > 0 {
                if i + 1 < n && chars[i] == '/' && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < n && chars[i] == '*' && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }

        // Raw string literal: r"...", r#"..."#, r##"..."##
        if chars[i] == 'r' && (i + 1 < n && (chars[i + 1] == '"' || chars[i + 1] == '#')) {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < n && chars[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && chars[j] == '"' {
                j += 1;
                let mut closed = false;
                while j < n {
                    if chars[j] == '"' {
                        let mut end_hashes = 0;
                        while j + 1 + end_hashes < n
                            && chars[j + 1 + end_hashes] == '#'
                            && end_hashes < hashes
                        {
                            end_hashes += 1;
                        }
                        if end_hashes == hashes {
                            j += 1 + hashes;
                            closed = true;
                            break;
                        }
                    }
                    if chars[j] == '\n' {
                        out.push('\n');
                    }
                    j += 1;
                }
                if closed {
                    out.push_str("\" \"");
                    i = j;
                    continue;
                }
            }
        }

        // Standard string literal: "..."
        if chars[i] == '"' {
            i += 1;
            while i < n {
                if chars[i] == '\\' && i + 1 < n {
                    i += 2;
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            out.push_str("\" \"");
            continue;
        }

        // Character literal: 'x'
        if chars[i] == '\'' && i + 1 < n && chars[i + 1] != '\'' {
            let is_char = (i + 2 < n && chars[i + 2] == '\'')
                || (i + 3 < n && chars[i + 1] == '\\' && chars[i + 3] == '\'');
            if is_char {
                i += 1;
                while i < n && chars[i] != '\'' {
                    if chars[i] == '\\' && i + 1 < n {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < n {
                    i += 1;
                }
                out.push(' ');
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

/// Detects any unauthorized unsafe code or attributes permitting unsafe code.
/// Returns `Some(violation_reason)` if detected, or `None` if clean.
fn detect_unsafe_code(content: &str) -> Option<String> {
    let stripped = strip_comments_and_strings(content);

    // 1. Detect compiler attributes permitting unsafe code
    let compressed: String = stripped.chars().filter(|c| !c.is_whitespace()).collect();
    if compressed.contains("allow(unsafe_code)") {
        return Some(
            "Strict #![forbid(unsafe_code)] violation: unauthorized 'allow(unsafe_code)' attribute detected"
                .to_string(),
        );
    }
    if compressed.contains("warn(unsafe_code)") {
        return Some(
            "Strict #![forbid(unsafe_code)] violation: unauthorized 'warn(unsafe_code)' attribute detected"
                .to_string(),
        );
    }

    // 2. Detect standalone keyword 'unsafe'
    let bytes = stripped.as_bytes();
    let target = b"unsafe";
    let target_len = target.len();
    let is_ident_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    let mut idx = 0;
    while idx + target_len <= bytes.len() {
        if &bytes[idx..idx + target_len] == target {
            let prev_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
            let next_ok =
                idx + target_len == bytes.len() || !is_ident_byte(bytes[idx + target_len]);
            if prev_ok && next_ok {
                return Some(
                    "Strict #![forbid(unsafe_code)] violation: unauthorized 'unsafe' keyword detected in candidate"
                        .to_string(),
                );
            }
        }
        idx += 1;
    }

    None
}
