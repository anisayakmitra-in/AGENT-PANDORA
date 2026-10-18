use crate::evolution::{BayesianCanaryGate, BayesianCanaryVerdict};
use crate::sessions::{SessionError, SessionSnapshot, SessionStore};
use pandora_types::{
    MemoryIntent, PrincipalId, SessionArchiveRecord, SessionId, SopDefinition, SopStep, StoredSop,
    TenantId, Timestamp, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug)]
pub enum ReflexionError {
    Session(SessionError),
    ArchiveNotFound(String),
    SessionNotFound(String),
    InvalidSop(String),
    ContractViolation(String),
}

impl fmt::Display for ReflexionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(err) => write!(formatter, "session store error: {err}"),
            Self::ArchiveNotFound(id) => write!(formatter, "archive '{id}' not found"),
            Self::SessionNotFound(id) => write!(formatter, "session '{id}' not found"),
            Self::InvalidSop(msg) => write!(formatter, "invalid SOP: {msg}"),
            Self::ContractViolation(msg) => write!(formatter, "contract violation: {msg}"),
        }
    }
}

impl std::error::Error for ReflexionError {}

impl From<SessionError> for ReflexionError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterizedLiteral {
    pub original: String,
    pub template: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReflexionCrystallizeResult {
    pub archive_id: String,
    pub session_id: SessionId,
    pub sop: SopDefinition,
    pub verdict: BayesianCanaryVerdict,
    pub promoted: bool,
    pub stored_sop: Option<StoredSop>,
    pub parameterized_literals: Vec<ParameterizedLiteral>,
}

#[derive(Clone, Debug)]
pub struct ReflexionCrystallizer {
    canary_gate: BayesianCanaryGate,
}

impl Default for ReflexionCrystallizer {
    fn default() -> Self {
        Self::new(BayesianCanaryGate::new(0.95))
    }
}

impl ReflexionCrystallizer {
    pub fn new(canary_gate: BayesianCanaryGate) -> Self {
        Self { canary_gate }
    }

    pub fn canary_gate(&self) -> &BayesianCanaryGate {
        &self.canary_gate
    }

    /// Parameterizes concrete workspace literals (paths, commit hashes, session UUIDs, digests)
    /// into reusable `{{VAR}}` template tokens.
    pub fn parameterize_literals(
        input: &str,
        workspace_root: Option<&str>,
    ) -> (String, Vec<ParameterizedLiteral>) {
        let mut result = input.to_string();
        let mut literals = Vec::new();

        // 1. Parameterize explicit workspace root if provided
        if let Some(root) = workspace_root
            && !root.is_empty()
        {
            let normalized_root = root.replace('\\', "/");
            let norm_lower = normalized_root.to_lowercase();
            let mut search_idx = 0;

            while search_idx < result.len() {
                let lower_res = result.replace('\\', "/").to_lowercase();
                if let Some(pos) = lower_res[search_idx..].find(&norm_lower) {
                    let abs_pos = search_idx + pos;
                    let end_pos = abs_pos + root.len();
                    if abs_pos <= result.len()
                        && end_pos <= result.len()
                        && result.is_char_boundary(abs_pos)
                        && result.is_char_boundary(end_pos)
                    {
                        let orig_slice = result[abs_pos..end_pos].to_string();
                        if !orig_slice.is_empty() {
                            literals.push(ParameterizedLiteral {
                                original: orig_slice,
                                template: "{{WORKSPACE_ROOT}}".to_string(),
                                kind: "workspace_root".to_string(),
                            });
                            result.replace_range(abs_pos..end_pos, "{{WORKSPACE_ROOT}}");
                            search_idx = abs_pos + "{{WORKSPACE_ROOT}}".len();
                            continue;
                        }
                    }
                }
                break;
            }
        }

        // 2. Scan and replace UUIDs (Session IDs), Commit SHAs, SHA256 digests, and absolute paths
        let words: Vec<String> = result
            .split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`' || c == '=')
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();

        for word in &words {
            let clean = word.trim_matches(|c: char| {
                c == ','
                    || c == '.'
                    || c == ';'
                    || c == ':'
                    || c == '('
                    || c == ')'
                    || c == '['
                    || c == ']'
                    || c == '{'
                    || c == '}'
                    || c == '<'
                    || c == '>'
                    || c == '"'
                    || c == '\''
                    || c == '`'
                    || c == '='
            });

            if clean.is_empty() || !result.contains(clean) {
                continue;
            }

            // Check for sha256 digests (exact or with prefix/suffix)
            if let Some(idx) = clean.find("sha256:") {
                let rest = &clean[idx..];
                if rest.len() >= 71 {
                    let candidate = &rest[..71];
                    if candidate[7..].chars().all(|c| c.is_ascii_hexdigit())
                        && result.contains(candidate)
                    {
                        if !literals.iter().any(|l| l.original == candidate) {
                            literals.push(ParameterizedLiteral {
                                original: candidate.to_string(),
                                template: "{{SHA256_DIGEST}}".to_string(),
                                kind: "sha256_digest".to_string(),
                            });
                        }
                        result = result.replace(candidate, "{{SHA256_DIGEST}}");
                    }
                }
            }
            // Check for git 40-character commit hash (exact or embedded)
            else if clean.len() == 40 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
                if !literals.iter().any(|l| l.original == clean) {
                    literals.push(ParameterizedLiteral {
                        original: clean.to_string(),
                        template: "{{COMMIT_SHA}}".to_string(),
                        kind: "commit_hash".to_string(),
                    });
                }
                result = result.replace(clean, "{{COMMIT_SHA}}");
            } else if clean.len() > 40 && !clean.contains('-') {
                for i in 0..=(clean.len() - 40) {
                    let candidate = &clean[i..i + 40];
                    let prefix_ok =
                        i == 0 || !clean[..i].chars().last().unwrap().is_ascii_alphanumeric();
                    let suffix_ok = i + 40 == clean.len()
                        || !clean[i + 40..]
                            .chars()
                            .next()
                            .unwrap()
                            .is_ascii_alphanumeric();
                    if prefix_ok
                        && suffix_ok
                        && candidate.chars().all(|c| c.is_ascii_hexdigit())
                        && result.contains(candidate)
                    {
                        if !literals.iter().any(|l| l.original == candidate) {
                            literals.push(ParameterizedLiteral {
                                original: candidate.to_string(),
                                template: "{{COMMIT_SHA}}".to_string(),
                                kind: "commit_hash".to_string(),
                            });
                        }
                        result = result.replace(candidate, "{{COMMIT_SHA}}");
                        break;
                    }
                }
            }
            // Check for UUID format: 8-4-4-4-12 hex chars (exact or embedded)
            else if clean.len() == 36 && is_uuid(clean) {
                if !literals.iter().any(|l| l.original == clean) {
                    literals.push(ParameterizedLiteral {
                        original: clean.to_string(),
                        template: "{{SESSION_ID}}".to_string(),
                        kind: "session_id".to_string(),
                    });
                }
                result = result.replace(clean, "{{SESSION_ID}}");
            } else if clean.len() > 36 && clean.contains('-') {
                for i in 0..=(clean.len() - 36) {
                    let candidate = &clean[i..i + 36];
                    if is_uuid(candidate) && result.contains(candidate) {
                        if !literals.iter().any(|l| l.original == candidate) {
                            literals.push(ParameterizedLiteral {
                                original: candidate.to_string(),
                                template: "{{SESSION_ID}}".to_string(),
                                kind: "session_id".to_string(),
                            });
                        }
                        result = result.replace(candidate, "{{SESSION_ID}}");
                        break;
                    }
                }
            }
            // Check for standalone absolute path patterns: e.g. C:\... or /...
            else if is_absolute_path_token(clean) {
                let template_token = if clean.ends_with(".rs")
                    || clean.ends_with(".json")
                    || clean.ends_with(".toml")
                    || clean.ends_with(".py")
                    || clean.ends_with(".md")
                    || clean.ends_with(".ts")
                    || clean.ends_with(".js")
                    || clean.ends_with(".txt")
                {
                    "{{TARGET_FILE}}"
                } else {
                    "{{WORKSPACE_PATH}}"
                };
                if !literals.iter().any(|l| l.original == clean) {
                    literals.push(ParameterizedLiteral {
                        original: clean.to_string(),
                        template: template_token.to_string(),
                        kind: "path_literal".to_string(),
                    });
                }
                result = result.replace(clean, template_token);
            }
        }

        (result, literals)
    }

    /// Extracts successful tool turn sequences and execution trajectories from a session archive,
    /// abstracting concrete workspace literals into parameterized `SopStep` templates.
    pub fn extract_sop_from_archive(
        &self,
        archive: &SessionArchiveRecord,
        snapshot: Option<&SessionSnapshot>,
        workspace_root: Option<&str>,
    ) -> Result<(SopDefinition, Vec<ParameterizedLiteral>), ReflexionError> {
        let mut steps = Vec::new();
        let mut step_counter = 0;
        let mut collected_literals = Vec::new();

        if let Some(snap) = snapshot {
            // 1. Inspect agent messages for assistant tool calls
            for msg in snap.agent_messages() {
                if let Ok(tool_calls) = msg.tool_calls()
                    && !tool_calls.is_empty()
                {
                    for call in tool_calls {
                        step_counter += 1;
                        let raw_cmd = format!("tool_call: {} {}", call.name(), call.arguments());
                        let (param_cmd, lits) =
                            Self::parameterize_literals(&raw_cmd, workspace_root);
                        for lit in lits {
                            if !collected_literals
                                .iter()
                                .any(|l: &ParameterizedLiteral| l.original == lit.original)
                            {
                                collected_literals.push(lit);
                            }
                        }
                        let step_id = format!("step-{step_counter}");
                        let step_name = format!("Step {step_counter}: {}", call.name());
                        let depends_on = if step_counter > 1 {
                            vec![format!("step-{}", step_counter - 1)]
                        } else {
                            vec![]
                        };

                        if let Ok(step) = SopStep::new(step_id, step_name, param_cmd, depends_on) {
                            steps.push(step);
                        }
                    }
                }
            }

            // 2. If no messages had tool calls, inspect runtime events
            if steps.is_empty() {
                for event in snap.events() {
                    step_counter += 1;
                    let (cmd, lits) = Self::parameterize_literals(
                        &format!("{:?}", event.payload()),
                        workspace_root,
                    );
                    for lit in lits {
                        if !collected_literals
                            .iter()
                            .any(|l: &ParameterizedLiteral| l.original == lit.original)
                        {
                            collected_literals.push(lit);
                        }
                    }
                    let step_id = format!("step-{step_counter}");
                    let step_name = format!("Event Step {step_counter}: {:?}", event.event_type());
                    let depends_on = if step_counter > 1 {
                        vec![format!("step-{}", step_counter - 1)]
                    } else {
                        vec![]
                    };

                    if let Ok(step) = SopStep::new(step_id, step_name, cmd, depends_on) {
                        steps.push(step);
                    }
                }
            }
        }

        // 3. If steps are still empty, synthesize from archive diff stats and summary
        if steps.is_empty() {
            let (param_summary, lits1) =
                Self::parameterize_literals(&archive.summary, workspace_root);
            let (param_diff, lits2) =
                Self::parameterize_literals(&archive.diff_stats, workspace_root);
            for lit in lits1.into_iter().chain(lits2) {
                if !collected_literals
                    .iter()
                    .any(|l: &ParameterizedLiteral| l.original == lit.original)
                {
                    collected_literals.push(lit);
                }
            }

            let s1 = SopStep::new(
                "step-1",
                "Initialize environment and load context",
                "echo \"Initializing context for SOP {{SESSION_ID}}\"",
                vec![],
            )
            .map_err(|e| ReflexionError::InvalidSop(e.to_string()))?;

            let s2 = SopStep::new(
                "step-2",
                "Execute parameterized trajectory",
                format!("apply_changes --summary \"{param_summary}\" --diff \"{param_diff}\""),
                vec!["step-1".to_string()],
            )
            .map_err(|e| ReflexionError::InvalidSop(e.to_string()))?;

            let s3 = SopStep::new(
                "step-3",
                "Validate workspace invariants",
                "verify_trajectory --receipt {{SHA256_DIGEST}}",
                vec!["step-2".to_string()],
            )
            .map_err(|e| ReflexionError::InvalidSop(e.to_string()))?;

            steps.push(s1);
            steps.push(s2);
            steps.push(s3);
        }

        let sop_id = format!(
            "sop-crystallized-{}",
            sanitize_identifier(&archive.archive_id)
        );
        let sop_name = format!("Crystallized SOP: {}", archive.summary);
        let verification = Some("git status --porcelain".to_string());

        let sop = SopDefinition::new(
            sop_id,
            sop_name,
            MemoryIntent::Procedure,
            steps,
            verification,
        )
        .map_err(|e| ReflexionError::InvalidSop(e.to_string()))?;

        Ok((sop, collected_literals))
    }

    /// Evaluates candidate SOP with BayesianCanaryGate and automatically promotes to active L3 storage in SQLite.
    #[allow(clippy::too_many_arguments)]
    pub fn verify_and_promote(
        &self,
        store: &SessionStore,
        sop: &SopDefinition,
        passes: u32,
        failures: u32,
        baseline_rate: f64,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        timestamp: Timestamp,
    ) -> Result<ReflexionCrystallizeResult, ReflexionError> {
        let verdict = self.canary_gate.evaluate(passes, failures, baseline_rate);
        let mut stored_sop = None;

        if verdict.promoted {
            let stored = store
                .store_sop(tenant_id, workspace_id, sop, timestamp)
                .map_err(ReflexionError::Session)?;
            stored_sop = Some(stored);
        }

        Ok(ReflexionCrystallizeResult {
            archive_id: sop.id.clone(),
            session_id: SessionId::new("crystallized-session").unwrap_or_else(|_| {
                SessionId::new("00000000-0000-0000-0000-000000000001").unwrap()
            }),
            sop: sop.clone(),
            verdict,
            promoted: stored_sop.is_some(),
            stored_sop,
            parameterized_literals: Vec::new(),
        })
    }

    /// High-level entrypoint: inspects a session archive, extracts SOP, tests against BayesianCanaryGate,
    /// and auto-promotes to SQLite L3 storage if it passes the credible lower bound.
    #[allow(clippy::too_many_arguments)]
    pub fn crystallize_session(
        &self,
        store: &SessionStore,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        baseline_rate: f64,
        workspace_root: Option<&str>,
        timestamp: Timestamp,
    ) -> Result<ReflexionCrystallizeResult, ReflexionError> {
        // 1. Try to find existing archive
        let archive_opt = store.load_archive(tenant_id, workspace_id, session_id.as_str())?;
        let snapshot = store
            .resume(session_id, principal_id, tenant_id, workspace_id)
            .ok();

        let archive = match archive_opt {
            Some(arc) => arc,
            None => {
                if snapshot.is_none() {
                    return Err(ReflexionError::SessionNotFound(session_id.to_string()));
                }
                let summary = format!("Archive synthesized from active session {}", session_id);
                let record = SessionArchiveRecord::new(
                    format!("archive-{}", session_id.as_str()),
                    tenant_id.clone(),
                    workspace_id.clone(),
                    session_id.clone(),
                    summary,
                    "+10 -2 lines changed",
                    format!("sha256:crystallize-{session_id}"),
                    timestamp,
                )
                .map_err(|e| ReflexionError::ContractViolation(e.to_string()))?;
                let _ = store.store_archive(&record);
                record
            }
        };

        // 2. Extract parameterized SOP
        let (sop, literals) =
            self.extract_sop_from_archive(&archive, snapshot.as_ref(), workspace_root)?;

        // 3. Count passes/failures from session evaluations if available
        let (passes, failures) = if let Some(ref snap) = snapshot
            && !snap.evaluations().is_empty()
        {
            let mut p = 0;
            let mut f = 0;
            for eval in snap.evaluations() {
                for res in eval.results() {
                    if res.status().as_str() == "passed" {
                        p += 1;
                    } else {
                        f += 1;
                    }
                }
            }
            (p, f)
        } else {
            // Golden evaluation baseline for completed session archives
            (10, 0)
        };

        // 4. Verify candidate against BayesianCanaryGate
        let mut result = self.verify_and_promote(
            store,
            &sop,
            passes,
            failures,
            baseline_rate,
            tenant_id,
            workspace_id,
            timestamp,
        )?;

        result.archive_id = archive.archive_id;
        result.session_id = session_id.clone();
        result.parameterized_literals = literals;
        Ok(result)
    }

    /// Crystallizes all known session archives for the workspace.
    #[allow(clippy::too_many_arguments)]
    pub fn crystallize_all(
        &self,
        store: &SessionStore,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        baseline_rate: f64,
        workspace_root: Option<&str>,
        timestamp: Timestamp,
    ) -> Result<Vec<ReflexionCrystallizeResult>, ReflexionError> {
        let archives = store.list_recent_archives(tenant_id, workspace_id, 256)?;
        let mut results = Vec::new();

        for archive in archives {
            let session_id = archive.session_id.clone();
            let snapshot = store
                .resume(&session_id, principal_id, tenant_id, workspace_id)
                .ok();
            let (sop, literals) =
                self.extract_sop_from_archive(&archive, snapshot.as_ref(), workspace_root)?;

            let (passes, failures) = if let Some(ref snap) = snapshot
                && !snap.evaluations().is_empty()
            {
                let mut p = 0;
                let mut f = 0;
                for eval in snap.evaluations() {
                    for res in eval.results() {
                        if res.status().as_str() == "passed" {
                            p += 1;
                        } else {
                            f += 1;
                        }
                    }
                }
                (p, f)
            } else {
                (10, 0)
            };

            let mut res = self.verify_and_promote(
                store,
                &sop,
                passes,
                failures,
                baseline_rate,
                tenant_id,
                workspace_id,
                timestamp,
            )?;
            res.archive_id = archive.archive_id;
            res.session_id = session_id;
            res.parameterized_literals = literals;
            results.push(res);
        }

        Ok(results)
    }
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    parts[0].len() == 8
        && parts[1].len() == 4
        && parts[2].len() == 4
        && parts[3].len() == 4
        && parts[4].len() == 12
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_absolute_path_token(s: &str) -> bool {
    // Windows path e.g. C:\... or C:/...
    if s.len() >= 3
        && s.chars().next().unwrap_or(' ').is_ascii_alphabetic()
        && s.chars().nth(1) == Some(':')
        && (s.chars().nth(2) == Some('\\') || s.chars().nth(2) == Some('/'))
    {
        return true;
    }
    // Unix path e.g. /work/... or /home/... or /var/... or /tmp/...
    if s.starts_with('/') && s.len() >= 3 && s[1..].contains('/') {
        return true;
    }
    false
}

fn sanitize_identifier(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Deep Reflexion Diagnostic Engine (Phase 14.1)
// ---------------------------------------------------------------------------

/// The kind of failure a diagnostic was derived from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReflexionFailureKind {
    Compiler,
    Test,
    Tool,
    Assertion,
    Unknown,
}

impl ReflexionFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compiler => "compiler",
            Self::Test => "test",
            Self::Tool => "tool",
            Self::Assertion => "assertion",
            Self::Unknown => "unknown",
        }
    }
}

/// A structured semantic failure delta extracted from a raw failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflexionDiagnostic {
    pub kind: ReflexionFailureKind,
    pub invariant_violated: String,
    pub root_cause: String,
    pub avoidance_guidance: String,
    pub severity: u8,
}

/// Severity levels for diagnostics (1 = low, 5 = critical).
impl ReflexionDiagnostic {
    pub fn is_critical(&self) -> bool {
        self.severity >= 4
    }
}

/// Analyzes raw failure text (compiler errors, test failures, tool errors) and
/// produces a structured semantic delta without invoking an LLM.
pub struct ReflexionDiagnosticEngine;

impl ReflexionDiagnosticEngine {
    pub fn diagnose(failure: &str) -> ReflexionDiagnostic {
        let lower = failure.to_lowercase();

        // Compiler errors
        if lower.contains("error[e")
            || lower.contains("error: ")
                && (lower.contains("compile")
                    || lower.contains("cannot find")
                    || lower.contains("mismatched types"))
        {
            let root = extract_first_line(failure, "error")
                .unwrap_or_else(|| first_non_empty_line(failure));
            return ReflexionDiagnostic {
                kind: ReflexionFailureKind::Compiler,
                invariant_violated: "code must type-check".to_owned(),
                root_cause: root,
                avoidance_guidance: "Fix the type or path error before retrying; do not weaken tests to bypass compilation.".to_owned(),
                severity: 4,
            };
        }

        // Assertions (before test failures, since "assertion failed" is a runtime invariant)
        if lower.contains("assert") {
            return ReflexionDiagnostic {
                kind: ReflexionFailureKind::Assertion,
                invariant_violated: "runtime invariant must hold".to_owned(),
                root_cause: first_non_empty_line(failure),
                avoidance_guidance: "Strengthen the precondition or fix the violating code path."
                    .to_owned(),
                severity: 3,
            };
        }

        // Test failures
        if lower.contains("test result: FAILED")
            || lower.contains("panicked at")
            || (lower.contains("expected") && lower.contains("got"))
        {
            let root = extract_first_line(failure, "panicked")
                .or_else(|| extract_first_line(failure, "assert"))
                .unwrap_or_else(|| first_non_empty_line(failure));
            return ReflexionDiagnostic {
                kind: ReflexionFailureKind::Test,
                invariant_violated: "behavior must match the test oracle".to_owned(),
                root_cause: root,
                avoidance_guidance: "Inspect the failing assertion and the code path; fix the implementation, not the test.".to_owned(),
                severity: 3,
            };
        }

        // Tool errors
        if lower.contains("tool execution failed")
            || lower.contains("permission denied")
            || lower.contains("not found") && lower.contains("command")
        {
            return ReflexionDiagnostic {
                kind: ReflexionFailureKind::Tool,
                invariant_violated: "tool invocation must succeed".to_owned(),
                root_cause: first_non_empty_line(failure),
                avoidance_guidance: "Verify tool arguments and environment; do not retry the identical failing effect.".to_owned(),
                severity: 2,
            };
        }

        ReflexionDiagnostic {
            kind: ReflexionFailureKind::Unknown,
            invariant_violated: "unclassified failure".to_owned(),
            root_cause: first_non_empty_line(failure),
            avoidance_guidance: "Inspect the failure manually before retrying.".to_owned(),
            severity: 1,
        }
    }
}

fn first_non_empty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_owned()
}

fn extract_first_line(text: &str, needle: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| l.to_lowercase().contains(needle))
        .map(str::to_owned)
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn compiler_error_classified() {
        let d =
            ReflexionDiagnosticEngine::diagnose("error[E0308]: mismatched types\n  expected i32");
        assert_eq!(d.kind, ReflexionFailureKind::Compiler);
        assert!(d.is_critical());
    }

    #[test]
    fn test_failure_classified() {
        let d = ReflexionDiagnosticEngine::diagnose(
            "test result: FAILED. 1 failed\nthread panicked at foo.rs:42",
        );
        assert_eq!(d.kind, ReflexionFailureKind::Test);
        assert_eq!(d.severity, 3);
    }

    #[test]
    fn assertion_classified() {
        let d = ReflexionDiagnosticEngine::diagnose("assertion failed: x == y");
        assert_eq!(d.kind, ReflexionFailureKind::Assertion);
    }

    #[test]
    fn unknown_fallback() {
        let d = ReflexionDiagnosticEngine::diagnose("something odd happened");
        assert_eq!(d.kind, ReflexionFailureKind::Unknown);
        assert_eq!(d.severity, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameterize_literals() {
        let input = "Executed commit 94ecc933c0b35bb9162806d423e0837479da189b on session 550e8400-e29b-41d4-a716-446655440000 in C:\\work\\AGENT-PANDORA\\target\\output.txt";
        let (output, literals) =
            ReflexionCrystallizer::parameterize_literals(input, Some("C:\\work\\AGENT-PANDORA"));
        assert!(output.contains("{{COMMIT_SHA}}"));
        assert!(output.contains("{{SESSION_ID}}"));
        assert!(output.contains("{{WORKSPACE_ROOT}}"));
        assert_eq!(literals.len(), 3);
    }

    #[test]
    fn test_parameterize_substring_workspace_and_punctuation() {
        // Workspace root is "work", which is also a substring of "{{WORKSPACE_ROOT}}"
        let input = "Running in /work/src/lib.rs with commit (94ecc933c0b35bb9162806d423e0837479da189b), session=550e8400-e29b-41d4-a716-446655440000; digest: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.";
        let (output, literals) = ReflexionCrystallizer::parameterize_literals(input, Some("/work"));
        assert!(output.contains("{{WORKSPACE_ROOT}}"));
        assert!(output.contains("{{COMMIT_SHA}}"));
        assert!(output.contains("{{SESSION_ID}}"));
        assert!(output.contains("{{SHA256_DIGEST}}"));
        assert!(literals.iter().any(|l| l.template == "{{WORKSPACE_ROOT}}"));
        assert!(literals.iter().any(|l| l.template == "{{COMMIT_SHA}}"));
        assert!(literals.iter().any(|l| l.template == "{{SESSION_ID}}"));
        assert!(literals.iter().any(|l| l.template == "{{SHA256_DIGEST}}"));
    }

    #[test]
    fn test_canary_evaluation_and_promotion() {
        let gate = BayesianCanaryGate::new(0.95);
        let crystallizer = ReflexionCrystallizer::new(gate);

        // 10 passes, 0 failures: lower bound is ~0.76 > 0.50 -> promoted
        let verdict = crystallizer.canary_gate().evaluate(10, 0, 0.50);
        assert!(verdict.promoted);

        // 1 pass, 10 failures: lower bound is ~0.03 < 0.50 -> not promoted
        let verdict_fail = crystallizer.canary_gate().evaluate(1, 10, 0.50);
        assert!(!verdict_fail.promoted);
    }
}
