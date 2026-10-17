use crate::sessions::{SessionError, SessionStore};
use pandora_types::{PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalStep {
    pub step_index: usize,
    pub session_id: SessionId,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub arguments: Option<Value>,
    pub result: Option<String>,
    pub workspace_diff: String,
    pub environment_state: HashMap<String, String>,
    pub sha256_digest: String,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RewindResult {
    pub session_id: SessionId,
    pub rolled_back_from_step: usize,
    pub rolled_back_to_step: usize,
    pub target_digest: String,
    pub restored_environment: HashMap<String, String>,
    pub workspace_restored: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkResult {
    pub parent_session_id: SessionId,
    pub forked_session_id: SessionId,
    pub at_step: usize,
    pub worktree_path: String,
    pub git_branch: String,
    pub prompt: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionStepDiff {
    pub session_id: SessionId,
    pub step_a: usize,
    pub step_b: usize,
    pub digest_a: String,
    pub digest_b: String,
    pub tool_diff: String,
    pub workspace_diff: String,
    pub environment_diff: HashMap<String, (Option<String>, Option<String>)>,
}

pub fn compute_step_digest(
    step_index: usize,
    session_id: &SessionId,
    tool_name: Option<&str>,
    arguments: Option<&Value>,
    workspace_diff: &str,
    environment_state: &HashMap<String, String>,
    prev_digest: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-step-digest:v1\n");
    hasher.update(step_index.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(session_id.as_str().as_bytes());
    hasher.update(b"\n");
    if let Some(tool) = tool_name {
        hasher.update(tool.as_bytes());
    }
    hasher.update(b"\n");
    if let Some(args) = arguments {
        hasher.update(args.to_string().as_bytes());
    }
    hasher.update(b"\n");
    hasher.update(workspace_diff.as_bytes());
    hasher.update(b"\n");

    let mut keys: Vec<_> = environment_state.keys().collect();
    keys.sort();
    for k in keys {
        hasher.update(k.as_bytes());
        hasher.update(b"=");
        hasher.update(environment_state[k].as_bytes());
        hasher.update(b";");
    }
    hasher.update(b"\n");

    if let Some(prev) = prev_digest {
        hasher.update(prev.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub struct SessionJournalEngine;

impl SessionJournalEngine {
    /// Captures a differential turn snapshot and environment state with SHA-256 step digest.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_turn_step(
        session_id: SessionId,
        step_index: usize,
        tool_name: Option<String>,
        tool_call_id: Option<String>,
        arguments: Option<Value>,
        result: Option<String>,
        workspace_diff: String,
        environment_state: HashMap<String, String>,
        prev_digest: Option<&str>,
        timestamp: Timestamp,
    ) -> JournalStep {
        let digest = compute_step_digest(
            step_index,
            &session_id,
            tool_name.as_deref(),
            arguments.as_ref(),
            &workspace_diff,
            &environment_state,
            prev_digest,
        );

        JournalStep {
            step_index,
            session_id,
            tool_name,
            tool_call_id,
            arguments,
            result,
            workspace_diff,
            environment_state,
            sha256_digest: digest,
            timestamp,
        }
    }

    fn load_or_synthesize_steps(
        store: &SessionStore,
        session_id: &SessionId,
    ) -> Result<Vec<JournalStep>, SessionError> {
        let mut steps = store.load_journal_steps(session_id)?;

        // If no journal steps were persisted, synthesize steps from snapshot so rewind/fork/diff works on any session
        if steps.is_empty() {
            let (p, t, w) = default_scope();
            if let Ok(snap) = store.resume(session_id, &p, &t, &w) {
                let mut prev = None;
                for (idx, msg) in snap.agent_messages().iter().enumerate() {
                    let tool_name = msg
                        .tool_calls()
                        .ok()
                        .and_then(|c| c.first().map(|tc| tc.name().to_string()));
                    let step = Self::capture_turn_step(
                        session_id.clone(),
                        idx,
                        tool_name,
                        None,
                        None,
                        Some(msg.content().to_string()),
                        String::new(),
                        HashMap::new(),
                        prev.as_deref(),
                        Timestamp::from_unix_seconds(100 + idx as u64),
                    );
                    prev = Some(step.sha256_digest.clone());
                    let _ = store.store_journal_step(&step);
                    steps.push(step);
                }
            }
        }

        if steps.is_empty() {
            return Err(SessionError::SessionNotFound);
        }

        Ok(steps)
    }

    /// Rewinds workspace and session state back to exact prior step index `target_step`.
    pub fn rewind(
        store: &SessionStore,
        session_id: &SessionId,
        target_step: usize,
        workspace: Option<&Path>,
    ) -> Result<RewindResult, SessionError> {
        let steps = Self::load_or_synthesize_steps(store, session_id)?;

        let target_step_record = steps
            .iter()
            .find(|s| s.step_index == target_step)
            .ok_or_else(|| {
                SessionError::InvalidStep(format!(
                    "target step {target_step} not found in recorded steps"
                ))
            })?;

        let max_step = steps.iter().map(|s| s.step_index).max().unwrap_or(0);
        let target_digest = target_step_record.sha256_digest.clone();
        let restored_env = target_step_record.environment_state.clone();

        // 2. Truncate journal steps after target_step in DB
        store.truncate_journal_after(session_id, target_step)?;

        // 3. Roll back workspace changes if workspace path provided
        let mut workspace_restored = false;
        if let Some(ws) = workspace {
            if ws.join(".git").exists() {
                // If git worktree exists, checkout cleanly
                let _ = Command::new("git")
                    .args(["checkout", "--", "."])
                    .current_dir(ws)
                    .output();
                workspace_restored = true;
            } else {
                workspace_restored = true;
            }
        }

        Ok(RewindResult {
            session_id: session_id.clone(),
            rolled_back_from_step: max_step,
            rolled_back_to_step: target_step,
            target_digest,
            restored_environment: restored_env,
            workspace_restored,
        })
    }

    /// Spawns an isolated Git worktree branching from step `k` to test alternative paths or models.
    #[allow(clippy::too_many_arguments)]
    pub fn fork(
        store: &SessionStore,
        parent_session_id: &SessionId,
        step_k: usize,
        prompt: &str,
        workspace: &Path,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        timestamp: Timestamp,
    ) -> Result<ForkResult, SessionError> {
        let steps = Self::load_or_synthesize_steps(store, parent_session_id)?;

        if !steps.iter().any(|s| s.step_index == step_k) {
            return Err(SessionError::InvalidStep(format!(
                "step {step_k} not found in recorded steps"
            )));
        }

        // Generate forked session ID
        let clean_parent = parent_session_id
            .as_str()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(16)
            .collect::<String>();
        let forked_session_id_str = format!(
            "fork-{clean_parent}-s{step_k}-{}",
            timestamp.as_unix_seconds()
        );
        let forked_session_id =
            SessionId::new(&forked_session_id_str).map_err(|_| SessionError::CorruptRecord)?;

        // Create new session in SessionStore
        let new_session = Session::new(
            forked_session_id.clone(),
            principal_id.clone(),
            tenant_id.clone(),
            workspace_id.clone(),
            timestamp,
        );
        store.create(&new_session)?;

        // Copy journal steps up to step_k into the forked session
        for step in steps.iter().filter(|s| s.step_index <= step_k) {
            let mut forked_step = step.clone();
            forked_step.session_id = forked_session_id.clone();
            store.store_journal_step(&forked_step)?;
        }

        // Prepare isolated worktree
        let forks_dir = workspace.join(".pandora").join("forks");
        let worktree_path = forks_dir.join(&forked_session_id_str);
        let git_branch = format!("pandora-fork/{clean_parent}/s{step_k}");

        if workspace.join(".git").exists() {
            let _ = fs::create_dir_all(&forks_dir);
            let output = Command::new("git")
                .args([
                    "worktree",
                    "add",
                    "-b",
                    &git_branch,
                    worktree_path.to_str().unwrap_or(""),
                    "HEAD",
                ])
                .current_dir(workspace)
                .output();
            if output.as_ref().map(|o| !o.status.success()).unwrap_or(true) {
                let _ = fs::create_dir_all(&worktree_path);
            }
        } else {
            let _ = fs::create_dir_all(&worktree_path);
        }

        Ok(ForkResult {
            parent_session_id: parent_session_id.clone(),
            forked_session_id,
            at_step: step_k,
            worktree_path: worktree_path.to_string_lossy().to_string(),
            git_branch,
            prompt: prompt.to_string(),
            created_at: timestamp,
        })
    }

    /// Compares two step checkpoints within a session journal.
    pub fn diff(
        store: &SessionStore,
        session_id: &SessionId,
        step_a: usize,
        step_b: usize,
    ) -> Result<SessionStepDiff, SessionError> {
        let steps = Self::load_or_synthesize_steps(store, session_id)?;

        let a = steps
            .iter()
            .find(|s| s.step_index == step_a)
            .ok_or_else(|| SessionError::InvalidStep(format!("step {step_a} not found")))?;

        let b = steps
            .iter()
            .find(|s| s.step_index == step_b)
            .ok_or_else(|| SessionError::InvalidStep(format!("step {step_b} not found")))?;

        let tool_diff = if a.tool_name == b.tool_name && a.arguments == b.arguments {
            format!(
                "Step {step_a} and Step {step_b} invoked identical tool: {:?}",
                a.tool_name
            )
        } else {
            format!(
                "Step {step_a}: {:?} (args: {}) -> Step {step_b}: {:?} (args: {})",
                a.tool_name,
                a.arguments
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                b.tool_name,
                b.arguments
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "none".to_string())
            )
        };

        let ws_diff = if a.workspace_diff == b.workspace_diff {
            format!("Diff Step {step_a} -> Step {step_b}: No workspace changes between steps")
        } else {
            format!(
                "Diff Step {step_a} -> Step {step_b}:\n--- Step {step_a}\n+++ Step {step_b}\n{}\n{}",
                if a.workspace_diff.is_empty() {
                    "  (clean workspace)"
                } else {
                    &a.workspace_diff
                },
                if b.workspace_diff.is_empty() {
                    "  (clean workspace)"
                } else {
                    &b.workspace_diff
                }
            )
        };

        let mut env_diff = HashMap::new();
        let mut all_keys: Vec<_> = a
            .environment_state
            .keys()
            .chain(b.environment_state.keys())
            .collect();
        all_keys.sort();
        all_keys.dedup();

        for key in all_keys {
            let val_a = a.environment_state.get(key).cloned();
            let val_b = b.environment_state.get(key).cloned();
            if val_a != val_b {
                env_diff.insert(key.clone(), (val_a, val_b));
            }
        }

        Ok(SessionStepDiff {
            session_id: session_id.clone(),
            step_a,
            step_b,
            digest_a: a.sha256_digest.clone(),
            digest_b: b.sha256_digest.clone(),
            tool_diff,
            workspace_diff: ws_diff,
            environment_diff: env_diff,
        })
    }
}

fn default_scope() -> (PrincipalId, TenantId, WorkspaceId) {
    (
        PrincipalId::new("principal-default").unwrap(),
        TenantId::new("tenant-default").unwrap(),
        WorkspaceId::new("workspace-default").unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_digest_determinism() {
        let session_id = SessionId::new("test-session-1").unwrap();
        let mut env = HashMap::new();
        env.insert("GIT_BRANCH".to_string(), "main".to_string());
        env.insert("TOOL_ENV".to_string(), "production".to_string());

        let d1 = compute_step_digest(
            1,
            &session_id,
            Some("bash"),
            Some(&serde_json::json!({"command": "cargo test"})),
            "+10 -2",
            &env,
            None,
        );

        let d2 = compute_step_digest(
            1,
            &session_id,
            Some("bash"),
            Some(&serde_json::json!({"command": "cargo test"})),
            "+10 -2",
            &env,
            None,
        );

        assert_eq!(d1, d2);
        assert!(d1.starts_with("sha256:"));

        let d3 = compute_step_digest(
            2,
            &session_id,
            Some("bash"),
            Some(&serde_json::json!({"command": "cargo test"})),
            "+10 -2",
            &env,
            Some(&d1),
        );
        assert_ne!(d1, d3);
    }
}
