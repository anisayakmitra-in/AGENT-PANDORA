#![forbid(unsafe_code)]

use pandora_types::{HarnessLibraryEntry, HarnessTree, HarnessTreeDiff};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Mode of recursive task-level harness refinement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecursionMode {
    /// Parallel exploration: Probes k general and k domain candidates simultaneously,
    /// then produces composite refinements from execution contrasts.
    Parallel,
    /// Sequential exploration: Carries the refined harness and accumulated evidence
    /// through k successive Propose-Probe-Compose steps.
    Sequential,
}

impl RecursionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Sequential => "sequential",
        }
    }
}

/// Category of candidate harness proposal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    /// Invariant: General operational heuristics, syntax guardrails, and error recovery.
    Invariant,
    /// Domain-specific: Task- or domain-targeted heuristics, schema adaptations, and APIs.
    DomainSpecific,
}

impl ProposalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invariant => "invariant",
            Self::DomainSpecific => "domain_specific",
        }
    }
}

/// A candidate harness variation proposed during task-level recursion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HarnessCandidate {
    pub id: String,
    pub kind: ProposalKind,
    pub rationale: String,
    pub diff: HarnessTreeDiff,
    pub resulting_tree: HarnessTree,
}

/// Execution receipt after probing a candidate harness on a task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProbeOutcome {
    pub candidate_id: String,
    pub tree_digest: String,
    pub success: bool,
    pub passed_tests: usize,
    pub total_tests: usize,
    pub tokens_used: u64,
    pub execution_duration_ms: u64,
    pub diagnostics: Vec<String>,
    pub error: Option<String>,
}

impl ProbeOutcome {
    /// Compute a normalized pass score (0..100).
    pub fn pass_score(&self) -> u32 {
        if self.total_tests == 0 {
            if self.success { 100 } else { 0 }
        } else {
            ((self.passed_tests as f64 / self.total_tests as f64) * 100.0).round() as u32
        }
    }

    /// True if candidate is strictly non-regressive relative to another outcome.
    pub fn dominates(&self, other: &Self) -> bool {
        self.pass_score() >= other.pass_score()
            && self.tokens_used <= other.tokens_used
            && self.execution_duration_ms <= other.execution_duration_ms
            && (self.pass_score() > other.pass_score()
                || self.tokens_used < other.tokens_used
                || self.execution_duration_ms < other.execution_duration_ms)
    }
}

/// Synthesis record resulting from the Compose step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposeSynthesis {
    pub round: usize,
    pub mode: RecursionMode,
    pub composite_tree: HarnessTree,
    pub applied_diff: HarnessTreeDiff,
    pub candidates_evaluated: usize,
    pub notes: String,
    pub digest: String,
}

/// Stateful Recursive Harnessing Runner implementing the Propose-Probe-Compose cycle.
#[derive(Clone, Debug)]
pub struct RecursiveHarnessRunner {
    pub session_id: String,
    pub base_tree: HarnessTree,
    pub current_tree: HarnessTree,
    pub history: Vec<ComposeSynthesis>,
    pub library: Vec<HarnessLibraryEntry>,
}

impl RecursiveHarnessRunner {
    pub fn new(session_id: impl Into<String>, base_tree: HarnessTree) -> Self {
        let current = base_tree.clone();
        Self {
            session_id: session_id.into(),
            base_tree,
            current_tree: current,
            history: Vec::new(),
            library: Vec::new(),
        }
    }

    /// Propose a candidate harness mutation.
    pub fn propose(
        &self,
        candidate_id: impl Into<String>,
        kind: ProposalKind,
        rationale: impl Into<String>,
        diff: HarnessTreeDiff,
    ) -> Result<HarnessCandidate, String> {
        let resulting_tree = self
            .current_tree
            .apply_diff(&diff)
            .map_err(|e| format!("failed to apply diff to current harness tree: {e}"))?;

        Ok(HarnessCandidate {
            id: candidate_id.into(),
            kind,
            rationale: rationale.into(),
            diff,
            resulting_tree,
        })
    }

    /// Compose a refined harness by contrasting probe outcomes across candidates.
    pub fn compose(
        &mut self,
        round: usize,
        mode: RecursionMode,
        candidates: &[HarnessCandidate],
        outcomes: &[ProbeOutcome],
    ) -> Result<ComposeSynthesis, String> {
        if candidates.is_empty() || outcomes.is_empty() {
            return Err("cannot compose empty candidate or outcome set".to_owned());
        }

        // Map candidates by ID
        let cand_map: BTreeMap<&str, &HarnessCandidate> =
            candidates.iter().map(|c| (c.id.as_str(), c)).collect();

        // Sort outcomes by pass score desc, tokens asc, duration asc
        let mut sorted_outcomes = outcomes.to_vec();
        sorted_outcomes.sort_by(|a, b| {
            b.pass_score()
                .cmp(&a.pass_score())
                .then_with(|| a.tokens_used.cmp(&b.tokens_used))
                .then_with(|| a.execution_duration_ms.cmp(&b.execution_duration_ms))
        });

        // Accumulate non-conflicting edits from top-performing candidates
        let mut combined_diff = HarnessTreeDiff::default();
        let mut applied_notes = Vec::new();

        for outcome in &sorted_outcomes {
            if let Some(candidate) = cand_map.get(outcome.candidate_id.as_str()) {
                let mut can_admit = false;
                for (path, content) in &candidate.diff.added {
                    if !combined_diff.added.contains_key(path)
                        && !combined_diff.modified.contains_key(path)
                        && !combined_diff.removed.contains(path)
                    {
                        combined_diff.added.insert(path.clone(), content.clone());
                        can_admit = true;
                    }
                }
                for (path, diff_pair) in &candidate.diff.modified {
                    if !combined_diff.added.contains_key(path)
                        && !combined_diff.modified.contains_key(path)
                        && !combined_diff.removed.contains(path)
                    {
                        combined_diff
                            .modified
                            .insert(path.clone(), diff_pair.clone());
                        can_admit = true;
                    }
                }
                for path in &candidate.diff.removed {
                    if !combined_diff.added.contains_key(path)
                        && !combined_diff.modified.contains_key(path)
                        && !combined_diff.removed.contains(path)
                    {
                        combined_diff.removed.insert(path.clone());
                        can_admit = true;
                    }
                }
                if can_admit {
                    applied_notes.push(format!(
                        "[{}] candidate '{}' (score: {}%, tokens: {})",
                        candidate.kind.as_str(),
                        candidate.id,
                        outcome.pass_score(),
                        outcome.tokens_used
                    ));
                }
            }
        }

        let composite_tree = self
            .current_tree
            .apply_diff(&combined_diff)
            .map_err(|e| format!("composition synthesis conflict: {e}"))?;

        let synthesis = ComposeSynthesis {
            round,
            mode,
            composite_tree: composite_tree.clone(),
            applied_diff: combined_diff,
            candidates_evaluated: candidates.len(),
            notes: applied_notes.join("; "),
            digest: composite_tree.digest(),
        };

        self.current_tree = composite_tree;
        self.history.push(synthesis.clone());
        Ok(synthesis)
    }

    /// Execute parallel recursion over k budget.
    pub fn run_parallel<F>(
        &mut self,
        k: usize,
        proposals: Vec<(ProposalKind, String, HarnessTreeDiff)>,
        probe_fn: F,
    ) -> Result<HarnessTree, String>
    where
        F: Fn(&HarnessCandidate) -> ProbeOutcome,
    {
        let budget = k.max(1);
        let mut candidates = Vec::new();
        for (i, (kind, rationale, diff)) in proposals.into_iter().take(budget * 2).enumerate() {
            let cid = format!("cand-p{}-{}-{}", i + 1, kind.as_str(), i % budget);
            let candidate = self.propose(cid, kind, rationale, diff)?;
            candidates.push(candidate);
        }

        let mut outcomes = Vec::new();
        for candidate in &candidates {
            let outcome = probe_fn(candidate);
            outcomes.push(outcome);
        }

        self.compose(1, RecursionMode::Parallel, &candidates, &outcomes)?;
        Ok(self.current_tree.clone())
    }

    /// Execute sequential recursion over k successive steps.
    pub fn run_sequential<F>(
        &mut self,
        k: usize,
        step_proposals: Vec<Vec<(ProposalKind, String, HarnessTreeDiff)>>,
        probe_fn: F,
    ) -> Result<HarnessTree, String>
    where
        F: Fn(&HarnessCandidate) -> ProbeOutcome,
    {
        let rounds = k.max(1);
        for round in 1..=rounds {
            let proposals = step_proposals.get(round - 1).cloned().unwrap_or_default();
            if proposals.is_empty() {
                break;
            }

            let mut candidates = Vec::new();
            for (i, (kind, rationale, diff)) in proposals.into_iter().enumerate() {
                let cid = format!("cand-s{round}-{}-{}", kind.as_str(), i + 1);
                let candidate = self.propose(cid, kind, rationale, diff)?;
                candidates.push(candidate);
            }

            let mut outcomes = Vec::new();
            for candidate in &candidates {
                let outcome = probe_fn(candidate);
                outcomes.push(outcome);
            }

            self.compose(round, RecursionMode::Sequential, &candidates, &outcomes)?;
        }

        Ok(self.current_tree.clone())
    }

    /// Reflect and adopt current refined harness into the persistent improvement library.
    pub fn reflect_and_adopt(
        &mut self,
        task_id: impl Into<String>,
        pass_rate: u32,
        token_cost: u64,
        latency_ms: u64,
        reflection_notes: impl Into<String>,
    ) -> HarnessLibraryEntry {
        let entry_id = format!("lib-{}-{}", self.session_id, self.library.len() + 1);
        let tree = self.current_tree.clone();
        let sig = tree.digest();
        let entry = HarnessLibraryEntry {
            id: entry_id,
            task_id: task_id.into(),
            tree,
            adopted_at_utc: "2026-10-26T12:00:00Z".to_owned(),
            reflection_notes: reflection_notes.into(),
            pass_rate: pass_rate.min(100),
            token_cost,
            latency_ms,
            signature: sig,
        };

        self.library.push(entry.clone());
        entry
    }
}

/// Factory creating a default baseline 8-component Pandora harness tree.
pub fn default_baseline_harness_tree() -> HarnessTree {
    let mut tree = HarnessTree::new();

    // c1: System Prompt
    tree.insert(
        "prompts/system.md",
        "# PANDORA System Prompt\n\
         You are Pandora, an autonomous agent engineered for governed, verifiable execution.\n\
         Always verify hypotheses with concrete compiler receipts, test assertions, and strict security bounds.",
    )
    .expect("valid baseline prompt");

    // c2: Guardrails
    tree.insert(
        "guardrails/boundaries.md",
        "# Constitutional Guardrails\n\
         - Destructive commands requiring human confirmation: git reset --hard, del /s, rmdir /s\n\
         - Memory integrity: All SOPs must be signed and validated before persistence\n\
         - Strict confinement: No path traversal out of designated workspace",
    )
    .expect("valid baseline guardrails");

    // c3: Memory Context
    tree.insert(
        "memory/context.md",
        "# Memory & Working Context\n\
         - L0 Constitutional boundaries\n\
         - L1 Intent index (repo, procedure, coding, user)\n\
         - L2 Persistent invariants\n\
         - L3 Reusable Task SOPs\n\
         - L4 Archived historical sessions",
    )
    .expect("valid baseline memory");

    // c4: Skills
    tree.insert(
        "skills/rust_verification/SKILL.md",
        "# Rust Verification Skill\n\
         Ensure `cargo fmt --all -- --check`, `cargo clippy -D warnings`, and `cargo test` pass cleanly.",
    )
    .expect("valid baseline skill");

    // c5: Tool Guidance
    tree.insert(
        "tools/guidance.md",
        "# Tool Guidance\n\
         - Prefer targeted reading before full-file rewrites\n\
         - Preserve existing comments and docstrings\n\
         - Verify CLI exit codes deterministically",
    )
    .expect("valid baseline tool guidance");

    // c6: Scripts
    tree.insert(
        "scripts/preflight.py",
        "import sys\nprint('Preflight checks passed.')\nsys.exit(0)\n",
    )
    .expect("valid baseline script");

    // c7: Subagents
    tree.insert(
        "subagents/specialists.md",
        "# Subagent Topologies\n\
         - DeepCoder: Systems and algorithmic implementation\n\
         - Researcher: Read-only codebase exploration and documentation retrieval",
    )
    .expect("valid baseline subagent");

    // c8: Plugins
    tree.insert(
        "plugins/security_guard/plugin.json",
        "{\n  \"name\": \"security_guard\",\n  \"version\": \"1.0.0\",\n  \"hooks\": [\"pre_tool_call\", \"post_tool_call\"]\n}",
    )
    .expect("valid baseline plugin");

    tree
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::HarnessComponent;

    #[test]
    fn default_baseline_tree_covers_all_components() {
        let tree = default_baseline_harness_tree();
        assert_eq!(tree.len(), 8);
        let breakdown = tree.component_breakdown();
        for comp in HarnessComponent::ALL {
            assert_eq!(breakdown.get(&comp).copied(), Some(1));
        }
    }

    #[test]
    fn parallel_recursion_composes_non_conflicting_edits() {
        let base = default_baseline_harness_tree();
        let mut runner = RecursiveHarnessRunner::new("test-session", base);

        let mut diff1 = HarnessTreeDiff::default();
        diff1.added.insert(
            "skills/benchmark_eval/SKILL.md".to_owned(),
            "# Benchmark Evaluation Skill".to_owned(),
        );

        let mut diff2 = HarnessTreeDiff::default();
        diff2.added.insert(
            "tools/benchmark_tools.md".to_owned(),
            "# Benchmark Tools Guidance".to_owned(),
        );

        let proposals = vec![
            (
                ProposalKind::Invariant,
                "Add benchmark evaluation skill".to_owned(),
                diff1,
            ),
            (
                ProposalKind::DomainSpecific,
                "Add benchmark tools guidance".to_owned(),
                diff2,
            ),
        ];

        let refined = runner
            .run_parallel(2, proposals, |candidate| ProbeOutcome {
                candidate_id: candidate.id.clone(),
                tree_digest: candidate.resulting_tree.digest(),
                success: true,
                passed_tests: 10,
                total_tests: 10,
                tokens_used: 120,
                execution_duration_ms: 45,
                diagnostics: vec!["compilation ok".to_owned()],
                error: None,
            })
            .expect("parallel recursion ok");

        assert!(refined.contains("skills/benchmark_eval/SKILL.md"));
        assert!(refined.contains("tools/benchmark_tools.md"));
        assert_eq!(runner.history.len(), 1);
        assert_eq!(runner.history[0].mode, RecursionMode::Parallel);
    }

    #[test]
    fn sequential_recursion_accumulates_improvements_across_steps() {
        let base = default_baseline_harness_tree();
        let mut runner = RecursiveHarnessRunner::new("seq-session", base);

        let mut diff_step1 = HarnessTreeDiff::default();
        diff_step1.modified.insert(
            "prompts/system.md".to_owned(),
            (
                "old".to_owned(),
                "# Refined Pandora Prompt Step 1".to_owned(),
            ),
        );

        let mut diff_step2 = HarnessTreeDiff::default();
        diff_step2.added.insert(
            "scripts/auto_lint.py".to_owned(),
            "print('lint ok')\n".to_owned(),
        );

        let steps = vec![
            vec![(
                ProposalKind::Invariant,
                "Refine prompt".to_owned(),
                diff_step1,
            )],
            vec![(
                ProposalKind::DomainSpecific,
                "Add linter".to_owned(),
                diff_step2,
            )],
        ];

        let refined = runner
            .run_sequential(2, steps, |candidate| ProbeOutcome {
                candidate_id: candidate.id.clone(),
                tree_digest: candidate.resulting_tree.digest(),
                success: true,
                passed_tests: 5,
                total_tests: 5,
                tokens_used: 80,
                execution_duration_ms: 30,
                diagnostics: Vec::new(),
                error: None,
            })
            .expect("sequential recursion ok");

        assert_eq!(
            refined.get("prompts/system.md"),
            Some("# Refined Pandora Prompt Step 1")
        );
        assert!(refined.contains("scripts/auto_lint.py"));
        assert_eq!(runner.history.len(), 2);

        // Reflect and adopt
        let library_entry = runner.reflect_and_adopt(
            "task-alpha",
            100,
            160,
            60,
            "Successfully refined prompt and linting script",
        );
        assert_eq!(runner.library.len(), 1);
        assert_eq!(library_entry.pass_rate, 100);
        assert_eq!(library_entry.task_id, "task-alpha");
    }
}
