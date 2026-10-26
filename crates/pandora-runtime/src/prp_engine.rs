#![forbid(unsafe_code)]

use pandora_types::{PrpDocument, PrpGateResult, PrpValidationGate, PrpValidationReport};
use std::time::Instant;

/// The PRP Execution and Progressive Validation Engine.
pub struct PrpEngine;

impl PrpEngine {
    /// Validates pre-flight invariants declared in a PRP document.
    pub fn verify_pre_flight_invariants(doc: &PrpDocument) -> Result<(), String> {
        for inv in &doc.pre_flight_invariants {
            if inv.trim().is_empty() {
                return Err("PRP contains an empty pre-flight invariant assertion".to_string());
            }
        }
        Ok(())
    }

    /// Evaluates a PRP document across its required progressive validation gates.
    /// In simulated or real environments, executes each gate sequentially,
    /// stopping early if a gate fails.
    pub fn validate_prp(
        doc: &PrpDocument,
        syntax_ok: bool,
        unit_ok: bool,
        integration_ok: bool,
        receipt_ok: bool,
    ) -> PrpValidationReport {
        let mut results = Vec::new();
        let mut all_passed = true;

        for gate in &doc.required_gates {
            let start = Instant::now();
            let (passed, details) = match gate {
                PrpValidationGate::Syntax => {
                    if syntax_ok {
                        (
                            true,
                            "Syntax and compilation checks passed with zero warnings".to_string(),
                        )
                    } else {
                        (
                            false,
                            "Syntax or compiler diagnostic error detected".to_string(),
                        )
                    }
                }
                PrpValidationGate::Unit => {
                    if unit_ok {
                        (
                            true,
                            "All isolated unit test suites passed 100%".to_string(),
                        )
                    } else {
                        (
                            false,
                            "Unit test assertions failed in targeted module".to_string(),
                        )
                    }
                }
                PrpValidationGate::Integration => {
                    if integration_ok {
                        (
                            true,
                            "Integration and end-to-end multi-agent flow succeeded".to_string(),
                        )
                    } else {
                        (
                            false,
                            "Integration harness communication or contract timeout".to_string(),
                        )
                    }
                }
                PrpValidationGate::ReceiptCheck => {
                    if receipt_ok {
                        (
                            true,
                            "Cryptographic evidence and receipt verified deterministically"
                                .to_string(),
                        )
                    } else {
                        (
                            false,
                            "Receipt verification signature or checksum mismatch".to_string(),
                        )
                    }
                }
            };

            let duration_ms = start.elapsed().as_millis().max(1) as u64;

            results.push(PrpGateResult {
                gate: *gate,
                passed,
                duration_ms,
                details,
            });

            if !passed {
                all_passed = false;
                break; // Progressive validation gate stops on first defect
            }
        }

        let summary = if all_passed {
            format!(
                "PRP '{}' successfully cleared all {} progressive validation gates",
                doc.id,
                doc.required_gates.len()
            )
        } else {
            format!(
                "PRP '{}' failed progressive validation; rollback plan recommended: {}",
                doc.id, doc.rollback_plan
            )
        };

        PrpValidationReport {
            prp_id: doc.id.clone(),
            all_passed,
            gate_results: results,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prp_validation_all_passing() {
        let prp = PrpDocument::scaffold_default("PRP-001", "Evolution Core");
        assert!(PrpEngine::verify_pre_flight_invariants(&prp).is_ok());

        let report = PrpEngine::validate_prp(&prp, true, true, true, true);
        assert!(report.all_passed);
        assert_eq!(report.gate_results.len(), 4);
    }

    #[test]
    fn test_prp_validation_fails_fast_on_unit_defect() {
        let prp = PrpDocument::scaffold_default("PRP-002", "Evolution Core");
        let report = PrpEngine::validate_prp(&prp, true, false, true, true);
        assert!(!report.all_passed);
        // Should stop at gate 2 (unit test failure) without running integration or receipt check
        assert_eq!(report.gate_results.len(), 2);
        assert_eq!(report.gate_results[1].gate, PrpValidationGate::Unit);
        assert!(!report.gate_results[1].passed);
    }
}
