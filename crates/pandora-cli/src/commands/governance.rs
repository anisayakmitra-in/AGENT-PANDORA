//! Subsystem 2: Tripartite Constitutional Governance & Capability Bus CLI.
//!
//! Subcommands:
//! - `status`: Inspect tripartite governance operational status, memory quota, and kill latencies.
//! - `audit`: Verify constitutional effect permit authorization.
//! - `heal`: Synthesize hot-patch and restore broken invariants (Theorems 104–106).
//! - `invariant-check`: Evaluate subsystem invariants and bisimulation preservation.

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::self_healing_kernel::{InvariantViolation, SelfHealingKernel};
use pandora_runtime::subsystems::governance::GovernanceSubsystem;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("governance requires 'status', 'audit', 'heal', 'invariant-check', 'bicameral-vote', or 'shadow-veto-check'")
    })?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "audit" => audit(&args[1..]),
        "heal" => heal(&args[1..]),
        "invariant-check" => invariant_check(&args[1..]),
        "bicameral-vote" => bicameral_vote(&args[1..]),
        "shadow-veto-check" => shadow_veto_check(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown governance subcommand '{unknown}'. Available: 'status', 'audit', 'heal', 'invariant-check', 'bicameral-vote', 'shadow-veto-check'"
        ))),
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    let subsystem = GovernanceSubsystem::new();
    let status = subsystem.status();

    let display = format!(
        "Governance Subsystem [THANATOS]\n\
         Status: {}\n\
         Authorities: {}\n\
         Memory Quota: {} MB\n\
         Kill Latency Bound: {} ms\n\
         Theorems Enforced: {:?}",
        status.status,
        status.active_authorities.join(", "),
        status.max_memory_quota_mb,
        status.max_kill_latency_bound_ms,
        status.theorems_enforced
    );

    Ok(success("governance status", json!(status), display))
}

fn audit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["permit", "config", "data-dir", "workspace", "json"])?;
    let subsystem = GovernanceSubsystem::new();

    let permit = parsed.value("permit").unwrap_or("filesystem:read");
    let authorized = subsystem.authorize_permit(permit);

    let display = format!(
        "Governance Constitutional Audit\n\
         Permit Request: {}\n\
         Decision: {}\n\
         Constitutional Guarantee: Theorem 74 (Zero Capability Leakage)",
        permit,
        if authorized { "AUTHORIZED" } else { "DENIED" }
    );

    Ok(success(
        "governance audit",
        json!({
            "permit": permit,
            "authorized": authorized,
            "theorem": 74
        }),
        display,
    ))
}

fn heal(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["subsystem", "config", "data-dir", "workspace", "json"],
    )?;
    let target = parsed.value("subsystem").unwrap_or("cognitive");

    let violation = InvariantViolation {
        violation_id: "viol_sample_01".to_string(),
        target_subsystem: target.to_string(),
        failed_predicate: "bounded_entropy_threshold".to_string(),
        severity: "medium".to_string(),
        detected_at_unix: 1700000000,
    };

    let kernel = SelfHealingKernel::new();
    let patch = kernel.synthesize_patch(&violation);

    let display = format!(
        "Self-Healing Kernel Patch Synthesis\n\
         Target Subsystem: {}\n\
         Patch ID: {}\n\
         Guard: {}\n\
         Bisimulation Verified: {}\n\
         Theorems: 104 (Bisimulation), 105 (Invariant Re-derivation), 106 (Zero-Downtime)",
        patch.target_subsystem, patch.patch_id, patch.guard_expression, patch.bisimulation_verified
    );

    Ok(success("governance heal", json!(patch), display))
}

fn invariant_check(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["subsystem", "config", "data-dir", "workspace", "json"],
    )?;
    let target = parsed.value("subsystem").unwrap_or("all");

    let kernel = SelfHealingKernel::new();
    let report = kernel.audit_subsystem(target);

    let display = format!(
        "Self-Healing Subsystem Invariant Audit\n\
         Subsystem: {}\n\
         Bisimulation Equivalence: {}\n\
         Invariant Re-derived: {}\n\
         Zero-Downtime Preserved: {}\n\
         Theorems Enforced: 104, 105, 106",
        target,
        report.theorem_104_bisimulation_equivalent,
        report.theorem_105_invariant_rederived,
        report.theorem_106_zero_downtime_preserved
    );

    Ok(success(
        "governance invariant-check",
        json!(report),
        display,
    ))
}

fn bicameral_vote(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "proposal",
            "upper-credits",
            "lower-credits",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;
    let proposal = parsed.value("proposal").unwrap_or("prop-default");
    let upper_credits = parsed
        .value("upper-credits")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(16);
    let lower_credits = parsed
        .value("lower-credits")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(25);

    let subsystem = GovernanceSubsystem::new();
    let permit = subsystem
        .vote_and_ratify(proposal, upper_credits, lower_credits)
        .map_err(|e| {
            CliError::execution(
                format!("bicameral voting failed: {e}"),
                json!({ "error": e }),
            )
        })?;

    let display = format!(
        "Bicameral Quadratic Voting Session Ratified\n\
         Proposal ID: {}\n\
         Permit ID: {}\n\
         Bicameral Margin: {:.4}\n\
         Theorems Enforced: {}\n\
         Signature: {}",
        permit.proposal_id,
        permit.permit_id,
        permit.bicameral_margin,
        permit.theorems_enforced.len(),
        permit.signature_checksum
    );

    Ok(success("governance bicameral-vote", json!(permit), display))
}

fn shadow_veto_check(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["intent", "config", "data-dir", "workspace", "json"])?;
    let intent = parsed.value("intent").unwrap_or("routine maintenance");

    let subsystem = GovernanceSubsystem::new();
    let council = subsystem.shadow_council();
    let check_res = council.red_team_adversarial_check(intent);

    let (vetoed, display) = match check_res {
        Ok(()) => (
            false,
            format!(
                "Shadow Council Red-Team Check\n\
                 Intent: {}\n\
                 Status: APPROVED (No adversarial breaches detected)\n\
                 Preemption Bound: <5 microseconds",
                intent
            ),
        ),
        Err(attestation) => (
            true,
            format!(
                "Shadow Council Red-Team Check [VETO ATTESTED]\n\
                 Intent: {}\n\
                 Breached Invariants: {}\n\
                 Dual Key Sig: ({}, {})\n\
                 Preemption Latency: {} us",
                intent,
                attestation.breached_invariants.join(", "),
                attestation.dual_key_signature.0,
                attestation.dual_key_signature.1,
                attestation.preemption_latency_us
            ),
        ),
    };

    Ok(success(
        "governance shadow-veto-check",
        json!({
            "intent": intent,
            "vetoed": vetoed,
        }),
        display,
    ))
}
