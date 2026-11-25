//! Subsystem 3: Cryptographic Zero-Knowledge & Post-Quantum Substrate CLI.
//!
//! Subcommands:
//! - `status`: Inspect cryptographic engine status, Goldilocks/Mersenne fields, and PQ key.
//! - `pq-sign`: Generate post-quantum hybrid attestation signature.
//! - `mpc-eval`: Evaluate homomorphic Beaver triple multiplication and Shamir secret sharing.

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::subsystems::crypto::CryptoSubsystem;
use pandora_runtime::swarm_mpc::BeaverTriple;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("crypto requires 'status', 'pq-sign', or 'mpc-eval'"))?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "pq-sign" => pq_sign(&args[1..]),
        "mpc-eval" => mpc_eval(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown crypto subcommand '{unknown}'. Available: 'status', 'pq-sign', 'mpc-eval'"
        ))),
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    let subsystem = CryptoSubsystem::new();
    let status = subsystem.status();

    let display = format!(
        "Cryptographic Subsystem [v4.0.4 OLYMPUS]\n\
         Status: {}\n\
         Prime Fields: {}\n\
         PQ Ed25519 PK: {}...\n\
         MPC Multiplication Result: {}\n\
         Theorems Enforced: {:?}",
        status.status,
        status.prime_fields.join(", "),
        &status.hybrid_pq_verifying_key.classical_pk_hex
            [..16.min(status.hybrid_pq_verifying_key.classical_pk_hex.len())],
        status.mpc_report.beaver_multiplication_result,
        status.theorems_enforced
    );

    Ok(success("crypto status", json!(status), display))
}

fn pq_sign(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["message", "config", "data-dir", "workspace", "json"],
    )?;
    let subsystem = CryptoSubsystem::new();

    let message = parsed
        .value("message")
        .unwrap_or("olympus_attestation_payload");
    let receipt = subsystem.sign_hybrid(message.as_bytes());

    let display = format!(
        "Hybrid Post-Quantum Attestation Receipt\n\
         Classical Sig: {} hex chars\n\
         Lattice Sig: {} hex chars\n\
         Sponge Digest: {}\n\
         Classical Verified: {}\n\
         Quantum Verified: {}\n\
         Theorems: 77, 78, 79",
        receipt.classical_sig_hex.len(),
        receipt.lattice_sig_hex.len(),
        receipt.sponge_digest,
        receipt.classical_verified,
        receipt.quantum_verified
    );

    Ok(success(
        "crypto pq-sign",
        json!({
            "classical_sig_len": receipt.classical_sig_hex.len(),
            "lattice_sig_len": receipt.lattice_sig_hex.len(),
            "sponge_digest": receipt.sponge_digest,
            "timestamp_ms": receipt.timestamp_ms,
            "classical_verified": receipt.classical_verified,
            "quantum_verified": receipt.quantum_verified,
        }),
        display,
    ))
}

fn mpc_eval(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["x", "y", "config", "data-dir", "workspace", "json"])?;
    let subsystem = CryptoSubsystem::new();

    let x: u64 = parsed.value("x").and_then(|v| v.parse().ok()).unwrap_or(42);
    let y: u64 = parsed.value("y").and_then(|v| v.parse().ok()).unwrap_or(13);

    let triple = BeaverTriple {
        a: 17,
        b: 23,
        c: (17 * 23) % subsystem.mpc.prime,
    };
    let result = subsystem.homomorphic_multiply(x, y, &triple);
    let verified = subsystem
        .mpc
        .verify_theorem_85_constant_round(x, y, &triple);

    let display = format!(
        "Homomorphic Beaver Triple MPC Evaluation\n\
         Inputs: x={}, y={}\n\
         Product: {} (mod p)\n\
         Evaluation Verified: {}\n\
         Theorems: 83, 84, 85",
        x, y, result, verified
    );

    Ok(success(
        "crypto mpc-eval",
        json!({
            "x": x,
            "y": y,
            "product": result,
            "prime": subsystem.mpc.prime,
            "verified": verified,
        }),
        display,
    ))
}
