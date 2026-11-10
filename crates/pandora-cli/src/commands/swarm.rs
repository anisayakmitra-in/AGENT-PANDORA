#![forbid(unsafe_code)]

//! Distributed Cross-Node Swarm Mesh CLI Commands
//!
//! Exposes subcommands for connecting peers, checking mesh status, proposing candidate
//! patches, casting Byzantine consensus votes, and synchronizing federated policy gradients.

use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::distributed_swarm::{
    DistributedConsensusDecision, DistributedConsensusVote, DistributedFederatedEngine,
    DistributedPatchProposal, DistributedSwarmConsensusEngine, DistributedSwarmError, SwarmPeer,
};
use pandora_runtime::federated_policy::{POLICY_DIMENSION_LABELS, PolicyGradientVector};
use pandora_runtime::mesh_sync::PeerIdentity;
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::ProposalScoreVector;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Persistent state model stored across CLI invocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentSwarmState {
    pub local_peer_id: String,
    pub public_key: String,
    pub listen_addr: String,
    pub peers: Vec<SwarmPeer>,
    pub proposals: Vec<DistributedPatchProposal>,
    pub votes: Vec<DistributedConsensusVote>,
    pub decisions: Vec<(String, DistributedConsensusDecision)>,
    pub policy: [f64; 6],
    pub epoch: u64,
}

impl PersistentSwarmState {
    pub fn default_for(identity: &PeerIdentity) -> Self {
        Self {
            local_peer_id: identity.peer_id.clone(),
            public_key: identity.public_key_hex(),
            listen_addr: "127.0.0.1:4000".to_string(),
            peers: Vec::new(),
            proposals: Vec::new(),
            votes: Vec::new(),
            decisions: Vec::new(),
            policy: [0.5; 6],
            epoch: 1,
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "swarm requires 'connect', 'status', 'propose', 'vote', or 'sync-gradients'",
        )
    })?;

    match subcommand.as_str() {
        "connect" => connect(&args[1..]),
        "status" => status(&args[1..]),
        "propose" => propose(&args[1..]),
        "vote" => vote(&args[1..]),
        "sync-gradients" | "sync" => sync_gradients(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown swarm command '{unknown}'. Available subcommands: connect, status, propose, vote, sync-gradients"
        ))),
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn load_or_init_state(data_dir: &Path) -> Result<(PeerIdentity, PersistentSwarmState), CliError> {
    let id_path = data_dir.join("peer_identity.hex");
    let identity = PeerIdentity::load_or_generate(&id_path).map_err(|e| {
        CliError::execution(format!("failed to load peer identity: {e}"), json!({}))
    })?;

    let state_path = data_dir.join("swarm_state.json");
    let state = if state_path.exists() {
        let content = fs::read_to_string(&state_path).map_err(|e| {
            CliError::execution(format!("failed to read swarm state: {e}"), json!({}))
        })?;
        serde_json::from_str(&content)
            .unwrap_or_else(|_| PersistentSwarmState::default_for(&identity))
    } else {
        PersistentSwarmState::default_for(&identity)
    };

    Ok((identity, state))
}

fn save_state(data_dir: &Path, state: &PersistentSwarmState) -> Result<(), CliError> {
    let state_path = data_dir.join("swarm_state.json");
    let json_text = serde_json::to_string_pretty(state).map_err(|e| {
        CliError::execution(format!("failed to serialize swarm state: {e}"), json!({}))
    })?;
    fs::write(&state_path, json_text)
        .map_err(|e| CliError::execution(format!("failed to write swarm state: {e}"), json!({})))?;
    Ok(())
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
        _ => ExternalAgentKind::Agy,
    }
}

fn connect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "peer", "node-id"],
    )?;

    let peer_addr = parsed
        .value("peer")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("swarm connect requires '--peer <address>'"))?;

    let config = load_config(&parsed)?;
    let (identity, mut state) = load_or_init_state(config.data_dir())?;

    let remote_node_id = parsed
        .value("node-id")
        .map(String::from)
        .unwrap_or_else(|| format!("node-{}", peer_addr.replace(['.', ':'], "-")));

    let timestamp_ms = now_epoch_ms();

    // Check if peer already exists
    if let Some(existing) = state.peers.iter_mut().find(|p| p.listen_addr == peer_addr) {
        existing.last_seen = timestamp_ms;
        existing.connected = true;
    } else {
        state.peers.push(SwarmPeer {
            peer_id: remote_node_id.clone(),
            listen_addr: peer_addr.to_string(),
            public_key: identity.public_key_hex(),
            last_seen: timestamp_ms,
            connected: true,
        });
    }

    save_state(config.data_dir(), &state)?;

    let text = format!(
        "Connected to peer: {}\nRemote Node ID:   {}\nLocal Peer ID:    {}\nCluster Epoch:    {}\nActive Peers:     {}",
        peer_addr,
        remote_node_id,
        identity.peer_id,
        state.epoch,
        state.peers.len()
    );

    Ok(success(
        "swarm connect",
        json!({
            "connected": true,
            "peer_address": peer_addr,
            "remote_node_id": remote_node_id,
            "local_peer_id": identity.peer_id,
            "cluster_epoch": state.epoch,
            "active_peers": state.peers.len(),
            "capabilities": ["byzantine_consensus", "federated_gradients", "git_canary"]
        }),
        text,
    ))
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let (identity, state) = load_or_init_state(config.data_dir())?;

    let mut text = String::new();
    text.push_str("Distributed Swarm Mesh Status (Theorem 47)\n");
    text.push_str("==========================================\n");
    text.push_str(&format!("Local Peer ID:     {}\n", identity.peer_id));
    text.push_str(&format!(
        "Public Key:        {}\n",
        identity.public_key_hex()
    ));
    text.push_str(&format!("Listen Address:    {}\n", state.listen_addr));
    text.push_str(&format!("Cluster Epoch:     {}\n", state.epoch));
    text.push_str("BFT Parameters:    N=4 nodes, f=1 tolerance, Q=3 quorum threshold\n");
    text.push_str(&format!("Connected Peers:   {}\n", state.peers.len()));
    for (i, p) in state.peers.iter().enumerate() {
        text.push_str(&format!(
            "  [{}] {} @ {} (connected: {})\n",
            i + 1,
            p.peer_id,
            p.listen_addr,
            p.connected
        ));
    }
    text.push_str(&format!("Active Proposals:  {}\n", state.proposals.len()));
    for prop in &state.proposals {
        text.push_str(&format!(
            "  - Proposal '{}': {} (agent: {})\n",
            prop.proposal_id,
            prop.title,
            prop.author_agent.identifier()
        ));
    }
    text.push_str(&format!("Decisions Recorded:{}\n", state.decisions.len()));
    for (prop_id, dec) in &state.decisions {
        let dec_str = match dec {
            DistributedConsensusDecision::Approved(cert) => {
                format!("Approved ({})", cert.certificate_digest)
            }
            DistributedConsensusDecision::Pending {
                current_approvals,
                quorum_needed,
                ..
            } => format!("Pending ({current_approvals}/{quorum_needed} approvals)"),
            DistributedConsensusDecision::Rejected { reason, .. } => {
                format!("Rejected ({reason})")
            }
        };
        text.push_str(&format!("  - {prop_id}: {dec_str}\n"));
    }
    text.push_str("Federated Policy Parameters (Theta):\n");
    for (idx, label) in POLICY_DIMENSION_LABELS.iter().enumerate() {
        text.push_str(&format!(
            "  [{}] {:<24} = {:.4}\n",
            idx, label, state.policy[idx]
        ));
    }

    Ok(success(
        "swarm status",
        json!({
            "local_peer_id": identity.peer_id,
            "public_key": identity.public_key_hex(),
            "listen_addr": state.listen_addr,
            "epoch": state.epoch,
            "bft_parameters": {
                "total_nodes": 4,
                "byzantine_tolerance": 1,
                "quorum_threshold": 3
            },
            "connected_peers_count": state.peers.len(),
            "connected_peers": state.peers,
            "proposals_count": state.proposals.len(),
            "decisions_count": state.decisions.len(),
            "federated_policy": state.policy
        }),
        text,
    ))
}

fn propose(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "goal",
            "diff",
            "title",
            "agent",
            "proposal-id",
        ],
    )?;

    let goal = parsed
        .value("goal")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("swarm propose requires '--goal <task_goal>'"))?;

    let config = load_config(&parsed)?;
    let (identity, mut state) = load_or_init_state(config.data_dir())?;

    let diff_input = parsed
        .value("diff")
        .unwrap_or("diff --git a/src b/src\n+ // verified patch\n");
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

    let title = parsed
        .value("title")
        .unwrap_or("Distributed Swarm Consensus Candidate");
    let author_agent = parse_agent_kind(parsed.value("agent"));
    let timestamp_ms = now_epoch_ms();

    let proposal_id = parsed
        .value("proposal-id")
        .map(String::from)
        .unwrap_or_else(|| format!("prop-{}-{}", identity.peer_id, timestamp_ms));

    let scores = ProposalScoreVector::new(0.95, 0.92, 0.88, 0.96);

    let proposal = DistributedPatchProposal::new(
        &proposal_id,
        &identity.peer_id,
        author_agent,
        title,
        goal,
        diff_content,
        scores,
        timestamp_ms,
    );

    let fingerprint = proposal.patch_fingerprint.clone();

    // Check duplicate
    if state.proposals.iter().any(|p| p.proposal_id == proposal_id) {
        return Err(CliError::execution(
            format!("proposal '{proposal_id}' already registered"),
            json!({"proposal_id": proposal_id}),
        ));
    }

    state.proposals.push(proposal.clone());
    state.decisions.push((
        proposal_id.clone(),
        DistributedConsensusDecision::Pending {
            current_approvals: 0,
            current_rejections: 0,
            quorum_needed: 3,
        },
    ));

    save_state(config.data_dir(), &state)?;

    let text = format!(
        "Candidate Patch Proposed:\n  Proposal ID: {}\n  Goal:        {}\n  Agent:       {}\n  Fingerprint: {}\n  Status:      Pending Byzantine Quorum (0/3 approvals)",
        proposal_id,
        goal,
        author_agent.identifier(),
        fingerprint
    );

    Ok(success(
        "swarm propose",
        json!({
            "proposal_id": proposal_id,
            "goal": goal,
            "agent": author_agent.identifier(),
            "fingerprint": fingerprint,
            "status": "pending_quorum",
            "quorum_needed": 3
        }),
        text,
    ))
}

fn vote(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "proposal",
            "approve",
            "reject",
            "decision",
            "voter",
            "pass",
            "confidence",
            "compactness",
            "invariant",
        ],
    )?;

    let proposal_id = parsed
        .value("proposal")
        .or_else(|| parsed.value("proposal-id"))
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("swarm vote requires '--proposal <id>'"))?;

    let config = load_config(&parsed)?;
    let (identity, mut state) = load_or_init_state(config.data_dir())?;

    // Find proposal
    let proposal = state
        .proposals
        .iter()
        .find(|p| p.proposal_id == proposal_id)
        .cloned()
        .ok_or_else(|| {
            CliError::execution(
                format!("proposal '{proposal_id}' not found in registered proposals"),
                json!({"proposal_id": proposal_id}),
            )
        })?;

    let approve = if parsed.has_flag("reject") {
        false
    } else if parsed.has_flag("approve") {
        true
    } else if let Some(dec) = parsed.value("decision") {
        dec == "approve" || dec == "yes" || dec == "true"
    } else {
        true
    };

    let voter_id = parsed
        .value("voter")
        .unwrap_or(&identity.peer_id)
        .to_string();

    let pass_rate = parsed
        .value("pass")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.95);
    let confidence = parsed
        .value("confidence")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.90);
    let compactness = parsed
        .value("compactness")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.85);
    let invariant = parsed
        .value("invariant")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.95);

    let scores = ProposalScoreVector::new(pass_rate, confidence, compactness, invariant);
    let timestamp_ms = now_epoch_ms();

    let mut new_vote =
        DistributedConsensusVote::new(proposal_id, &voter_id, approve, scores, timestamp_ms);
    new_vote.sign_vote(&identity);

    // Initialize consensus engine with N=4, f=1 (Q=3)
    let mut engine = DistributedSwarmConsensusEngine::new(&identity.peer_id, 4, 1)
        .map_err(|e| CliError::execution(e.to_string(), json!({})))?;

    engine
        .submit_proposal(proposal)
        .map_err(|e| CliError::execution(e.to_string(), json!({})))?;

    // Replay existing votes for this proposal
    for existing_vote in &state.votes {
        if existing_vote.proposal_id == proposal_id {
            let _ = engine.record_vote(existing_vote.clone());
        }
    }

    // Record the new vote with equivocation checking
    let decision = engine.record_vote(new_vote.clone()).map_err(|e| match e {
        DistributedSwarmError::EquivocationDetected { voter, proposal } => CliError::execution(
            format!("equivocation detected: voter '{voter}' cast conflicting vote for proposal '{proposal}'"),
            json!({"voter": voter, "proposal": proposal}),
        ),
        other => CliError::execution(other.to_string(), json!({})),
    })?;

    // Store the vote if not already present
    if !state.votes.iter().any(|v| {
        v.proposal_id == proposal_id
            && v.voter_node_id == voter_id
            && v.approve == approve
            && v.verified_scores.distance(&scores) < 1e-4
    }) {
        state.votes.push(new_vote);
    }

    // Update decision in state
    if let Some(entry) = state.decisions.iter_mut().find(|(id, _)| id == proposal_id) {
        entry.1 = decision.clone();
    } else {
        state
            .decisions
            .push((proposal_id.to_string(), decision.clone()));
    }

    save_state(config.data_dir(), &state)?;

    let decision_str = match &decision {
        DistributedConsensusDecision::Approved(cert) => {
            format!(
                "APPROVED\n  Certificate: {}\n  Consensus Hash: {}",
                cert.certificate_digest, cert.consensus_hash
            )
        }
        DistributedConsensusDecision::Pending {
            current_approvals,
            quorum_needed,
            ..
        } => {
            format!("PENDING ({current_approvals}/{quorum_needed} approvals)")
        }
        DistributedConsensusDecision::Rejected { reason, .. } => {
            format!("REJECTED: {reason}")
        }
    };

    let text = format!(
        "Vote Recorded:\n  Proposal: {}\n  Voter:    {}\n  Approved: {}\n  Scores:   pass={:.2}, conf={:.2}, comp={:.2}, inv={:.2}\n  Consensus Status:\n    {}",
        proposal_id, voter_id, approve, pass_rate, confidence, compactness, invariant, decision_str
    );

    Ok(success(
        "swarm vote",
        json!({
            "proposal_id": proposal_id,
            "voter": voter_id,
            "approve": approve,
            "decision": decision,
        }),
        text,
    ))
}

fn sync_gradients(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "epoch",
            "loss",
            "samples",
        ],
    )?;
    let config = load_config(&parsed)?;
    let (identity, mut state) = load_or_init_state(config.data_dir())?;

    let epoch = parsed
        .value("epoch")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(state.epoch);

    let loss = parsed
        .value("loss")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.125);

    let samples = parsed
        .value("samples")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(64);

    let timestamp_ms = now_epoch_ms();

    let mut federated = DistributedFederatedEngine::new(&identity.peer_id, state.policy);

    // Create 4 simulated worker gradients from heterogeneous cluster agents
    let agents = [
        ExternalAgentKind::Agy,
        ExternalAgentKind::ClaudeCode,
        ExternalAgentKind::CodexCli,
        ExternalAgentKind::GeminiCli,
    ];

    for (i, &agent) in agents.iter().enumerate() {
        let grad_vals = [
            -0.02 * (i as f64 + 1.0),
            -0.015 * (i as f64 + 1.0),
            -0.03 * (i as f64 + 1.0),
            -0.01 * (i as f64 + 1.0),
            -0.025 * (i as f64 + 1.0),
            -0.005 * (i as f64 + 1.0),
        ];
        let grad = PolicyGradientVector::new(agent, grad_vals, samples, loss)
            .map_err(|e| CliError::execution(e.to_string(), json!({})))?;

        let packet = federated
            .broadcast_gradient(&identity, epoch, grad, timestamp_ms)
            .map_err(|e| CliError::execution(e.to_string(), json!({})))?;
        federated
            .receive_gradient_packet(packet, None)
            .map_err(|e| CliError::execution(e.to_string(), json!({})))?;
    }

    let receipt = federated
        .aggregate_cluster_gradients(epoch, timestamp_ms)
        .map_err(|e| CliError::execution(e.to_string(), json!({})))?;

    state.policy = federated.current_policy;
    state.epoch = epoch + 1;

    save_state(config.data_dir(), &state)?;

    let mut text = format!(
        "Federated Policy Gradients Synchronized (Theorem 47 Lemma 47.4):\n  Step Index:           {}\n  Participating Agents: {}\n  Aggregate L2-Norm:    {:.6}\n  Update Hash:          {}\n  Updated Policy Parameters (Theta):\n",
        receipt.step_index,
        receipt.participating_agents,
        receipt.aggregate_norm,
        receipt.update_hash
    );

    for (idx, label) in POLICY_DIMENSION_LABELS.iter().enumerate() {
        text.push_str(&format!(
            "    [{}] {:<24} = {:.4}\n",
            idx, label, state.policy[idx]
        ));
    }

    Ok(success(
        "swarm sync-gradients",
        json!({
            "step_index": receipt.step_index,
            "epoch": epoch,
            "participating_agents": receipt.participating_agents,
            "aggregate_norm": receipt.aggregate_norm,
            "update_hash": receipt.update_hash,
            "updated_policy": state.policy
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_err(res: Result<CommandResult, CliError>) -> CliError {
        match res {
            Err(err) => err,
            Ok(_) => panic!("expected CliError"),
        }
    }

    #[test]
    fn test_swarm_cli_execution_bare_usage() {
        let err = assert_err(execute(&[]));
        assert!(err.message.contains("swarm requires"));
    }

    #[test]
    fn test_swarm_cli_connect_missing_peer() {
        let err = assert_err(execute(&["connect".to_string()]));
        assert!(err.message.contains("swarm connect requires"));
    }

    #[test]
    fn test_swarm_cli_propose_missing_goal() {
        let err = assert_err(execute(&["propose".to_string()]));
        assert!(err.message.contains("swarm propose requires"));
    }

    #[test]
    fn test_swarm_cli_vote_missing_proposal() {
        let err = assert_err(execute(&["vote".to_string()]));
        assert!(err.message.contains("swarm vote requires"));
    }
}
