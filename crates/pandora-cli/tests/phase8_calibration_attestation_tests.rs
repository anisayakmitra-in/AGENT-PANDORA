#![forbid(unsafe_code)]

use pandora_runtime::decision_engine::{
    CalibrationAttestationReceipt, TropicalDecisionDriver, compute_calibration_commitment,
    compute_dpo_dataset_digest,
};
use serde_json::Value;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cli_model_calibrate_emits_hash_chained_attestation_receipts_and_detects_tampering() {
    let bin = env!("CARGO_BIN_EXE_pandora");
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase8_cli_attest_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pandora_dir = temp_dir.join(".pandora");
    std::fs::create_dir_all(&pandora_dir).unwrap();

    // Write online-emitted DPO pairs in .pandora/dpo_pairs.jsonl (Phase 5 -> Phase 8 zero-touch pipeline)
    let p1 = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
        "dpo-online-1",
        "run_command rm -rf /",
        "reject",
        "allow",
        0.06,
    );
    let p2 = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
        "dpo-online-2",
        "view_file README.md",
        "allow",
        "require_confirm",
        0.09,
    );
    let pairs = vec![p1.clone(), p2.clone()];
    let expected_dpo_digest = compute_dpo_dataset_digest(&pairs);
    let jsonl_content = format!(
        "{}\n{}\n",
        serde_json::to_string(&p1).unwrap(),
        serde_json::to_string(&p2).unwrap()
    );
    std::fs::write(pandora_dir.join("dpo_pairs.jsonl"), jsonl_content).unwrap();

    let pnn_path = pandora_dir.join("model_decision.pnn");

    // 1. First calibration run (genesis receipt sequence = 0)
    let out1 = Command::new(bin)
        .args([
            "model",
            "calibrate",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--pnn",
            pnn_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run pandora model calibrate #1");
    assert!(
        out1.status.success(),
        "calibrate #1 failed: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let env1: Value = serde_json::from_slice(&out1.stdout).unwrap();
    assert_eq!(env1["canary_passed"], true);
    assert_eq!(env1["pairs_count"], 2);
    assert_eq!(env1["dpo_dataset_digest"], expected_dpo_digest);

    let rec1 = &env1["attestation_receipt"];
    assert_eq!(rec1["sequence"], 0);
    assert_eq!(
        rec1["prev_receipt_hash"],
        CalibrationAttestationReceipt::GENESIS_PREV_HASH
    );
    assert_eq!(rec1["verified"], true);

    let base_dig = rec1["baseline_digest"].as_str().unwrap();
    let cand_dig = rec1["candidate_digest"].as_str().unwrap();
    let lb95 = rec1["lower_bound_95"].as_f64().unwrap();
    let expected_c1 =
        compute_calibration_commitment(base_dig, cand_dig, &expected_dpo_digest, lb95);
    assert_eq!(rec1["commitment"], expected_c1);

    // 2. Second calibration run (sequence = 1, chained to r0.chain_hash)
    let out2 = Command::new(bin)
        .args([
            "model",
            "calibrate",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--pnn",
            pnn_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run pandora model calibrate #2");
    assert!(
        out2.status.success(),
        "calibrate #2 failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let env2: Value = serde_json::from_slice(&out2.stdout).unwrap();
    let rec2 = &env2["attestation_receipt"];
    assert_eq!(rec2["sequence"], 1);
    assert_eq!(rec2["prev_receipt_hash"], rec1["chain_hash"]);
    assert_eq!(rec2["verified"], true);

    // Verify the on-disk .attestation.jsonl ledger chain
    let receipt_path =
        std::path::PathBuf::from(format!("{}.attestation.jsonl", pnn_path.display()));
    assert!(receipt_path.is_file());
    let raw_ledger = std::fs::read_to_string(&receipt_path).unwrap();
    let ledger_receipts: Vec<CalibrationAttestationReceipt> = raw_ledger
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(ledger_receipts.len(), 2);
    assert!(CalibrationAttestationReceipt::verify_chain(
        &ledger_receipts
    ));

    // 3. Tamper with the on-disk ledger and confirm next calibrate fails closed
    let mut tampered = ledger_receipts.clone();
    tampered[0].lower_bound_95 = 0.12345;
    let tampered_content = tampered
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&receipt_path, tampered_content).unwrap();

    let out3 = Command::new(bin)
        .args([
            "model",
            "calibrate",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--pnn",
            pnn_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run pandora model calibrate #3 after tamper");
    assert!(
        !out3.status.success(),
        "calibrate must fail closed when attestation ledger is tampered"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn cli_model_verify_attestation_audits_ledger_and_rejects_tampering() {
    let bin = env!("CARGO_BIN_EXE_pandora");
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase12_verify_attest_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pandora_dir = temp_dir.join(".pandora");
    std::fs::create_dir_all(&pandora_dir).unwrap();
    let pnn_path = pandora_dir.join("model_decision.pnn");

    // Calibrate twice to build a 2-receipt hash chain
    for _ in 0..2 {
        let out = Command::new(bin)
            .args([
                "model",
                "calibrate",
                "--workspace",
                temp_dir.to_str().unwrap(),
                "--pnn",
                pnn_path.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("run pandora model calibrate");
        assert!(out.status.success());
    }

    // Verify attestation ledger offline via CLI
    let verify_ok = Command::new(bin)
        .args([
            "model",
            "verify-attestation",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--pnn",
            pnn_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run pandora model verify-attestation");
    assert!(
        verify_ok.status.success(),
        "verify-attestation failed: {}",
        String::from_utf8_lossy(&verify_ok.stderr)
    );
    let env: Value = serde_json::from_slice(&verify_ok.stdout).unwrap();
    assert_eq!(env["valid"], true);
    assert_eq!(env["chain_length"], 2);
    assert_eq!(env["latest_sequence"], 1);
    assert_eq!(env["pnn_digest_matches"], true);

    // Tamper with live .pnn file and confirm verify-attestation fails closed
    let mut pnn_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&pnn_path).unwrap()).unwrap();
    pnn_json["sha256_digest"] = serde_json::json!("f".repeat(64));
    std::fs::write(&pnn_path, serde_json::to_string_pretty(&pnn_json).unwrap()).unwrap();

    let verify_tampered = Command::new(bin)
        .args([
            "model",
            "verify-attestation",
            "--workspace",
            temp_dir.to_str().unwrap(),
            "--pnn",
            pnn_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run pandora model verify-attestation on tampered pnn");
    assert!(
        !verify_tampered.status.success(),
        "verify-attestation must fail closed when .pnn digest mismatches attested ledger"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
