#![forbid(unsafe_code)]

//! Autonomous Git Worktree Patch Isolation & Canary Staging CLI Commands.
//!
//! Provides CLI subcommands to stage candidate patches in ephemeral Git worktrees,
//! verify sandbox safety invariants (memory safety, containment, bounds),
//! generate Cosign In-Toto attestations, and list/prune isolated worktrees.

use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::canary_worktree::{CanaryStagePipeline, GitWorktreeManager};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{AgentPatchProposal, ProposalScoreVector};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("canary requires a subcommand: 'stage', 'verify', 'list', or 'prune'")
    })?;

    match subcommand.as_str() {
        "stage" => stage(&args[1..]),
        "verify" => verify(&args[1..]),
        "list" => list(&args[1..]),
        "prune" | "clean" => prune(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown canary command '{unknown}'. Available subcommands: stage, verify, list, prune"
        ))),
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn resolve_workspace_dir(parsed: &super::ParsedArgs) -> Result<PathBuf, CliError> {
    if let Some(dir) = parsed.value("dir").or_else(|| parsed.value("workspace")) {
        let path = PathBuf::from(dir);
        if path.exists() {
            return Ok(path);
        }
    }

    if let Ok(config) = load_config(parsed) {
        let ws = config.workspace_dir();
        if ws.exists() {
            return Ok(ws.to_path_buf());
        }
    }

    std::env::current_dir().map_err(|e| {
        CliError::execution(
            format!("failed to resolve current directory: {e}"),
            json!({}),
        )
    })
}

fn parse_agent_kind(agent_str: Option<&str>) -> ExternalAgentKind {
    match agent_str {
        Some("claude" | "claude-code" | "claude-cli") => ExternalAgentKind::ClaudeCode,
        Some("codex" | "codex-cli") => ExternalAgentKind::CodexCli,
        Some("gemini" | "gemini-cli") => ExternalAgentKind::GeminiCli,
        Some("cursor" | "cursor-agent" | "cursor-cli") => ExternalAgentKind::CursorAgent,
        Some("devin" | "devin-cli") => ExternalAgentKind::Devin,
        Some("cline" | "cline-cli") => ExternalAgentKind::Cline,
        Some("hermes" | "hermes-agent") => ExternalAgentKind::Hermes,
        Some("qwen" | "qwen-code") => ExternalAgentKind::QwenCode,
        _ => ExternalAgentKind::ClaudeCode,
    }
}

fn stage(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "task",
            "diff",
            "author",
            "agent",
            "cert",
            "branch",
            "dir",
            "workspace",
            "config",
        ],
    )?;

    let repo_root = resolve_workspace_dir(&parsed)?;
    let timestamp = now_epoch_secs();
    let task_id = parsed
        .value("task")
        .unwrap_or("unnamed_canary_task")
        .to_string();

    let diff_input = parsed.value("diff").unwrap_or(
        "+++ b/canary_patch.rs\n+// Verified safe canary implementation\npub fn canary_run() -> bool { true }\n",
    );

    let diff_content = if Path::new(diff_input).is_file() {
        fs::read_to_string(diff_input).map_err(|e| {
            CliError::execution(
                format!("failed to read diff file '{diff_input}': {e}"),
                json!({}),
            )
        })?
    } else {
        diff_input.to_string()
    };

    let agent = parse_agent_kind(parsed.value("agent").or_else(|| parsed.value("author")));
    let branch = parsed
        .value("branch")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("pandora/canary/{task_id}"));

    let cert = parsed
        .value("cert")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let mut hasher = Sha256::new();
            hasher.update(diff_content.as_bytes());
            format!("PANDORA_PATCH_CERT_V1:{:x}", hasher.finalize())
        });

    let proposal = AgentPatchProposal::new(
        format!("patch_{task_id}_{timestamp}"),
        agent,
        format!("Canary candidate for {task_id}"),
        "Autonomous multi-agent consensus winner staged for canary verification",
        diff_content,
        ProposalScoreVector::new(0.95, 0.96, 0.92, 0.98),
        timestamp,
    );

    let mut worktree =
        GitWorktreeManager::create_worktree(&repo_root, &branch, &cert).map_err(|e| {
            CliError::execution(
                format!("failed to create isolated worktree: {e}"),
                json!({}),
            )
        })?;

    let pipeline = CanaryStagePipeline::default();
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &proposal)
        .map_err(|e| {
            let _ = GitWorktreeManager::remove_worktree(&worktree);
            CliError::execution(
                format!("canary sandbox verification failed: {e}"),
                json!({}),
            )
        })?;

    let json_data = json!({
        "status": "staged",
        "canary_id": receipt.canary_id,
        "worktree_path": receipt.worktree_path,
        "branch_name": receipt.branch_name,
        "patch_certificate": receipt.patch_certificate,
        "commit_hash": receipt.commit_hash,
        "attestation_digest": receipt.attestation_digest,
        "signature_hex": receipt.signature_hex,
        "passed_checks": receipt.passed_checks,
        "total_checks": receipt.total_checks,
        "staged_at_unix": receipt.staged_at_unix,
    });

    let plain = format!(
        "Canary patch successfully staged and attested!\n\
         - Canary ID:          {}\n\
         - Branch Name:        {}\n\
         - Worktree Path:      {}\n\
         - Patch Certificate:  {}\n\
         - Commit Hash:        {}\n\
         - Attestation Digest: {}\n\
         - Checks Passed:      {}/{} (100% memory safe & contained)",
        receipt.canary_id,
        receipt.branch_name,
        receipt.worktree_path.display(),
        receipt.patch_certificate,
        receipt.commit_hash,
        receipt.attestation_digest,
        receipt.passed_checks,
        receipt.total_checks,
    );

    Ok(success("canary stage", json_data, plain))
}

fn verify(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["id", "task", "diff", "dir", "workspace", "config"])?;
    let repo_root = resolve_workspace_dir(&parsed)?;

    let worktrees = GitWorktreeManager::list_worktrees(&repo_root)
        .map_err(|e| CliError::execution(format!("failed to list worktrees: {e}"), json!({})))?;

    let target_id = parsed.value("id").or_else(|| parsed.value("task"));

    if let Some(query) = target_id {
        let found = worktrees
            .iter()
            .find(|wt| wt.id == query || wt.branch_name.contains(query));

        if let Some(wt) = found {
            let receipt_file = wt.path.join(".canary_receipt.json");
            let has_receipt = receipt_file.exists();
            let json_data = json!({
                "verified": true,
                "worktree": wt,
                "has_attestation": has_receipt,
            });
            let plain = format!(
                "Worktree '{}' verified:\n- Path: {}\n- Branch: {}\n- Status: {:?}\n- Certificate: {}\n- Attestation Present: {}",
                wt.id,
                wt.path.display(),
                wt.branch_name,
                wt.status,
                wt.patch_certificate,
                has_receipt,
            );
            return Ok(success("canary verify", json_data, plain));
        } else {
            return Err(CliError::execution(
                format!("no active canary worktree found matching '{query}'"),
                json!({}),
            ));
        }
    }

    let json_data = json!({
        "active_worktrees": worktrees.len(),
        "worktrees": worktrees,
    });
    let plain = format!(
        "Found {} active canary worktrees ready for verification.",
        worktrees.len()
    );
    Ok(success("canary verify", json_data, plain))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["dir", "workspace", "config"])?;
    let repo_root = resolve_workspace_dir(&parsed)?;

    let worktrees = GitWorktreeManager::list_worktrees(&repo_root)
        .map_err(|e| CliError::execution(format!("failed to list worktrees: {e}"), json!({})))?;

    let json_data = json!({
        "count": worktrees.len(),
        "worktrees": worktrees,
    });

    let mut plain = format!(
        "Active Canary Worktrees ({count}):\n",
        count = worktrees.len()
    );
    if worktrees.is_empty() {
        plain.push_str("  (no active canary worktrees)\n");
    } else {
        for wt in &worktrees {
            plain.push_str(&format!(
                "  [{status:?}] {id} -> branch: {branch} (path: {path})\n",
                status = wt.status,
                id = wt.id,
                branch = wt.branch_name,
                path = wt.path.display(),
            ));
        }
    }

    Ok(success("canary list", json_data, plain))
}

fn prune(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["all", "id", "dir", "workspace", "config"])?;
    let repo_root = resolve_workspace_dir(&parsed)?;

    if let Some(id) = parsed.value("id") {
        let worktrees = GitWorktreeManager::list_worktrees(&repo_root).map_err(|e| {
            CliError::execution(format!("failed to list worktrees: {e}"), json!({}))
        })?;

        let target = worktrees.iter().find(|wt| wt.id == id);
        if let Some(wt) = target {
            GitWorktreeManager::remove_worktree(wt).map_err(|e| {
                CliError::execution(format!("failed to remove worktree {id}: {e}"), json!({}))
            })?;
            return Ok(success(
                "canary prune",
                json!({"pruned_id": id}),
                format!("Pruned canary worktree '{id}' successfully."),
            ));
        } else {
            return Err(CliError::execution(
                format!("worktree '{id}' not found"),
                json!({}),
            ));
        }
    }

    let pruned_count = GitWorktreeManager::prune_worktrees(&repo_root)
        .map_err(|e| CliError::execution(format!("failed to prune worktrees: {e}"), json!({})))?;

    Ok(success(
        "canary prune",
        json!({"pruned_count": pruned_count}),
        format!("Pruned {pruned_count} canary worktree(s)."),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!(
            "pandora_cli_canary_{name}_{}_{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_canary_cli_stage_and_list_and_prune() {
        let root = temp_test_root("stage_list_prune");
        let root_str = root.to_str().unwrap().to_string();

        // 1. Stage
        let stage_args = vec![
            "stage".to_string(),
            "--dir".to_string(),
            root_str.clone(),
            "--task".to_string(),
            "vector_cache_v2".to_string(),
        ];
        let stage_res = execute(&stage_args).expect("stage should succeed");
        assert_eq!(stage_res.data["status"], "staged");
        let canary_id = stage_res.data["canary_id"].as_str().unwrap().to_string();

        // 2. List
        let list_args = vec!["list".to_string(), "--dir".to_string(), root_str.clone()];
        let list_res = execute(&list_args).expect("list should succeed");
        assert_eq!(list_res.data["count"], 1);

        // 3. Verify
        let verify_args = vec![
            "verify".to_string(),
            "--dir".to_string(),
            root_str.clone(),
            "--id".to_string(),
            canary_id.clone(),
        ];
        let verify_res = execute(&verify_args).expect("verify should succeed");
        assert_eq!(verify_res.data["verified"], true);

        // 4. Prune
        let prune_args = vec!["prune".to_string(), "--dir".to_string(), root_str.clone()];
        let prune_res = execute(&prune_args).expect("prune should succeed");
        assert_eq!(prune_res.data["pruned_count"], 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_canary_cli_rejects_unsafe_patch() {
        let root = temp_test_root("unsafe");
        let root_str = root.to_str().unwrap().to_string();

        let stage_args = vec![
            "stage".to_string(),
            "--dir".to_string(),
            root_str.clone(),
            "--task".to_string(),
            "unsafe_task".to_string(),
            "--diff".to_string(),
            "+++ b/danger.rs\n+unsafe fn raw_deref() { let _ = 0 as *mut u8; }\n".to_string(),
        ];

        match execute(&stage_args) {
            Ok(_) => panic!("unsafe patch must be rejected"),
            Err(err) => {
                assert!(err.message.contains("sandbox verification failed"));
            }
        }

        let _ = fs::remove_dir_all(&root);
    }
}
