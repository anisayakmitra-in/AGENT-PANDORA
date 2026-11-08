//! Phase 9: Distributed Differential Privacy Budget Tracker with Rényi DP CLI.
//!
//! Subcommands:
//! - `status`: Inspect global and per-agent Rényi DP budgets, active reservations, and optimal (epsilon, delta)-DP conversion.
//! - `reserve`: Reserve an RDP budget for an agent query before execution.
//! - `commit`: Commit an active reservation to the immutable expenditure ledger.
//! - `release`: Release/rollback an uncommitted reservation.
//! - `record`: Directly record a Gaussian or Laplace mechanism query expenditure.
//! - `simulate`: Multi-agent Rényi DP composition and Theorem 18 tail-bound benchmark.

use super::{load_config_storage, parse_options, path_option};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::privacy_budget::{
    DistributedPrivacyTracker, PrivacyAccountantError, PrivacyExpenditure, RdpBudget,
    STANDARD_RDP_ORDERS, key_to_order,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PRIVACY_STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivacyStoreFile {
    pub version: u32,
    pub tracker: DistributedPrivacyTracker,
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn map_privacy_error(err: PrivacyAccountantError) -> CliError {
    match err {
        PrivacyAccountantError::BudgetExceeded { projected, limit } => CliError::execution(
            format!("privacy budget exceeded: projected={projected:.4}, limit={limit:.4}"),
            json!({"code": "budget_exceeded", "projected_epsilon": projected, "limit_epsilon": limit}),
        ),
        PrivacyAccountantError::ReservationNotFound(id) => CliError::execution(
            format!("query reservation not found: {id}"),
            json!({"code": "reservation_not_found", "reservation_id": id}),
        ),
        PrivacyAccountantError::ReservationAlreadyCommitted(id) => CliError::execution(
            format!("reservation already committed: {id}"),
            json!({"code": "reservation_already_committed", "reservation_id": id}),
        ),
        other => CliError::usage(other.to_string()),
    }
}

fn resolve_store_path(parsed: &super::ParsedArgs) -> Result<PathBuf, CliError> {
    if let Some(path) = path_option(parsed, "store") {
        return Ok(path);
    }
    let config = load_config_storage(parsed)?;
    Ok(config.data_dir().join("privacy").join("rdp_tracker.json"))
}

fn load_privacy_tracker(
    path: &Path,
    default_max_eps: f64,
    default_delta: f64,
) -> Result<DistributedPrivacyTracker, CliError> {
    if !path.exists() {
        return DistributedPrivacyTracker::new(default_max_eps, default_delta)
            .map_err(map_privacy_error);
    }

    let bytes = fs::read(path).map_err(|e| {
        CliError::execution(
            format!(
                "failed to read privacy store file '{}': {e}",
                path.display()
            ),
            json!({}),
        )
    })?;

    let store: PrivacyStoreFile = serde_json::from_slice(&bytes).map_err(|e| {
        CliError::execution(
            format!(
                "failed to parse privacy store file '{}': {e}",
                path.display()
            ),
            json!({}),
        )
    })?;

    Ok(store.tracker)
}

fn save_privacy_tracker(path: &Path, tracker: &DistributedPrivacyTracker) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CliError::execution(
                format!("failed to create directory '{}': {e}", parent.display()),
                json!({}),
            )
        })?;
    }
    let store = PrivacyStoreFile {
        version: PRIVACY_STORE_VERSION,
        tracker: tracker.clone(),
    };
    let serialized = serde_json::to_string_pretty(&store).map_err(|e| {
        CliError::internal(format!("failed to serialize privacy store: {e}"), json!({}))
    })?;
    fs::write(path, serialized).map_err(|e| {
        CliError::execution(
            format!(
                "failed to write privacy store file '{}': {e}",
                path.display()
            ),
            json!({}),
        )
    })?;
    Ok(())
}

fn parse_mechanism_budget(
    parsed: &super::ParsedArgs,
) -> Result<(String, f64, f64, RdpBudget), CliError> {
    let mechanism = parsed
        .value("mechanism")
        .unwrap_or("gaussian")
        .to_ascii_lowercase();
    let sensitivity: f64 = if let Some(s) = parsed.value("sensitivity") {
        s.parse()
            .map_err(|_| CliError::usage("--sensitivity must be a positive number"))?
    } else {
        1.0
    };
    let noise: f64 = if let Some(n) = parsed
        .value("sigma")
        .or_else(|| parsed.value("scale"))
        .or_else(|| parsed.value("noise"))
    {
        n.parse().map_err(|_| {
            CliError::usage("noise parameter (--sigma / --scale) must be a positive number")
        })?
    } else {
        2.0
    };

    match mechanism.as_str() {
        "gaussian" => {
            let budget = RdpBudget::from_gaussian(sensitivity, noise).map_err(map_privacy_error)?;
            Ok(("gaussian".to_string(), sensitivity, noise, budget))
        }
        "laplace" => {
            let budget = RdpBudget::from_laplace(sensitivity, noise).map_err(map_privacy_error)?;
            Ok(("laplace".to_string(), sensitivity, noise, budget))
        }
        other => Err(CliError::usage(format!(
            "unknown mechanism '{other}'; expected 'gaussian' or 'laplace'"
        ))),
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "privacy requires 'status', 'reserve', 'commit', 'release', 'record', or 'simulate'",
        )
    })?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "reserve" => reserve(&args[1..]),
        "commit" => commit(&args[1..]),
        "release" | "rollback" => release(&args[1..]),
        "record" => record(&args[1..]),
        "simulate" => simulate(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown privacy subcommand '{unknown}'. Available: 'status', 'reserve', 'commit', 'release', 'record', 'simulate'"
        ))),
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "max-epsilon",
            "delta",
            "store",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "privacy status does not accept positional arguments",
        ));
    }

    let max_eps: f64 = if let Some(m) = parsed.value("max-epsilon") {
        m.parse()
            .map_err(|_| CliError::usage("--max-epsilon must be a positive number"))?
    } else {
        10.0
    };
    let delta: f64 = if let Some(d) = parsed.value("delta") {
        d.parse()
            .map_err(|_| CliError::usage("--delta must be in (0, 1)"))?
    } else {
        1e-5
    };

    let store_path = resolve_store_path(&parsed)?;
    let tracker = load_privacy_tracker(&store_path, max_eps, delta)?;

    let (current_eps, target_delta) = tracker.current_approx_dp().map_err(map_privacy_error)?;
    let (optimal_alpha, _) = tracker
        .cumulative_budget
        .optimal_order_for_delta(target_delta)
        .map_err(map_privacy_error)?;
    let remaining_eps = tracker.remaining_epsilon().map_err(map_privacy_error)?;
    let utilization = tracker.budget_utilization().map_err(map_privacy_error)?;

    let mut rdp_spectrum = BTreeMap::new();
    for (&k, &v) in &tracker.cumulative_budget.values {
        rdp_spectrum.insert(format!("{:.2}", key_to_order(k)), v);
    }

    let human = format!(
        "Distributed Rényi Differential Privacy Accountant:\n  Global Max Budget (epsilon_max): {:.4}\n  Target Failure Probability (delta): {:.2e}\n  Current Committed (epsilon, delta): ({:.4}, {:.2e})\n  Optimal Rényi Order (alpha*): {:.2}\n  Remaining Budget (epsilon_rem): {:.4}\n  Budget Utilization: {:.2}%\n  Active Reservations: {}\n  Committed Transactions: {}\n  Tracked Agents: {}",
        tracker.global_max_epsilon,
        target_delta,
        current_eps,
        target_delta,
        optimal_alpha,
        remaining_eps,
        utilization * 100.0,
        tracker.active_reservations.len(),
        tracker.ledger.len(),
        tracker.agent_allocations.len(),
    );

    Ok(success(
        "privacy status",
        json!({
            "global_max_epsilon": tracker.global_max_epsilon,
            "global_target_delta": target_delta,
            "current_epsilon": current_eps,
            "optimal_renyi_order_alpha": optimal_alpha,
            "remaining_epsilon": remaining_eps,
            "budget_utilization": utilization,
            "active_reservations_count": tracker.active_reservations.len(),
            "committed_transactions_count": tracker.ledger.len(),
            "tracked_agents_count": tracker.agent_allocations.len(),
            "rdp_spectrum": rdp_spectrum,
            "store_path": store_path.display().to_string(),
        }),
        human,
    ))
}

fn reserve(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "agent-id",
            "query",
            "mechanism",
            "sensitivity",
            "sigma",
            "scale",
            "noise",
            "max-epsilon",
            "delta",
            "store",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let agent_id = parsed
        .value("agent-id")
        .ok_or_else(|| CliError::usage("privacy reserve requires '--agent-id <id>'"))?;
    let query_id = parsed
        .value("query")
        .ok_or_else(|| CliError::usage("privacy reserve requires '--query <id>'"))?;

    let max_eps: f64 = if let Some(m) = parsed.value("max-epsilon") {
        m.parse()
            .map_err(|_| CliError::usage("--max-epsilon must be a positive number"))?
    } else {
        10.0
    };
    let delta: f64 = if let Some(d) = parsed.value("delta") {
        d.parse()
            .map_err(|_| CliError::usage("--delta must be in (0, 1)"))?
    } else {
        1e-5
    };

    let (mechanism, sensitivity, noise, budget) = parse_mechanism_budget(&parsed)?;
    let store_path = resolve_store_path(&parsed)?;
    let mut tracker = load_privacy_tracker(&store_path, max_eps, delta)?;

    let reservation_id = tracker
        .reserve(agent_id, query_id, &budget)
        .map_err(map_privacy_error)?;
    save_privacy_tracker(&store_path, &tracker)?;

    let projected_eps = tracker
        .total_allocated_rdp()
        .epsilon_at_delta(tracker.global_target_delta)
        .map_err(map_privacy_error)?;

    let human = format!(
        "Privacy Budget Reserved:\n  Reservation ID: {reservation_id}\n  Agent ID: {agent_id}\n  Query ID: {query_id}\n  Mechanism: {mechanism} (sensitivity={sensitivity:.4}, noise={noise:.4})\n  Projected Allocated Epsilon: {projected_eps:.4} / {:.4}",
        tracker.global_max_epsilon
    );

    Ok(success(
        "privacy reserve",
        json!({
            "reservation_id": reservation_id,
            "agent_id": agent_id,
            "query_id": query_id,
            "mechanism": mechanism,
            "sensitivity": sensitivity,
            "noise_parameter": noise,
            "projected_allocated_epsilon": projected_eps,
            "global_max_epsilon": tracker.global_max_epsilon,
        }),
        human,
    ))
}

fn commit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "reservation",
            "id",
            "store",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let reservation_id = parsed
        .value("reservation")
        .or_else(|| parsed.value("id"))
        .ok_or_else(|| CliError::usage("privacy commit requires '--reservation <id>'"))?;

    let store_path = resolve_store_path(&parsed)?;
    let mut tracker = load_privacy_tracker(&store_path, 10.0, 1e-5)?;
    let updated_eps = tracker
        .commit_reservation(reservation_id)
        .map_err(map_privacy_error)?;
    let remaining_eps = tracker.remaining_epsilon().map_err(map_privacy_error)?;
    save_privacy_tracker(&store_path, &tracker)?;

    let human = format!(
        "Privacy Reservation Committed:\n  Reservation ID: {reservation_id}\n  Committed Global Epsilon: {updated_eps:.4} / {:.4}\n  Remaining Epsilon: {remaining_eps:.4}\n  Ledger Entries: {}",
        tracker.global_max_epsilon,
        tracker.ledger.len()
    );

    Ok(success(
        "privacy commit",
        json!({
            "reservation_id": reservation_id,
            "committed": true,
            "current_epsilon": updated_eps,
            "remaining_epsilon": remaining_eps,
            "global_max_epsilon": tracker.global_max_epsilon,
            "ledger_entries": tracker.ledger.len(),
        }),
        human,
    ))
}

fn release(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "reservation",
            "id",
            "store",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let reservation_id = parsed
        .value("reservation")
        .or_else(|| parsed.value("id"))
        .ok_or_else(|| CliError::usage("privacy release requires '--reservation <id>'"))?;

    let store_path = resolve_store_path(&parsed)?;
    let mut tracker = load_privacy_tracker(&store_path, 10.0, 1e-5)?;
    tracker
        .release_reservation(reservation_id)
        .map_err(map_privacy_error)?;
    let (current_eps, _) = tracker.current_approx_dp().map_err(map_privacy_error)?;
    save_privacy_tracker(&store_path, &tracker)?;

    let human = format!(
        "Privacy Reservation Released:\n  Reservation ID: {reservation_id}\n  Current Committed Epsilon: {current_eps:.4}\n  Active Reservations Remaining: {}",
        tracker.active_reservations.len()
    );

    Ok(success(
        "privacy release",
        json!({
            "reservation_id": reservation_id,
            "released": true,
            "current_epsilon": current_eps,
            "active_reservations_count": tracker.active_reservations.len(),
        }),
        human,
    ))
}

fn record(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "agent-id",
            "query",
            "mechanism",
            "sensitivity",
            "sigma",
            "scale",
            "noise",
            "max-epsilon",
            "delta",
            "store",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let agent_id = parsed.value("agent-id").unwrap_or("agent-default");
    let query_id = parsed.value("query").unwrap_or("query-direct");
    let max_eps: f64 = if let Some(m) = parsed.value("max-epsilon") {
        m.parse()
            .map_err(|_| CliError::usage("--max-epsilon must be a positive number"))?
    } else {
        10.0
    };
    let delta: f64 = if let Some(d) = parsed.value("delta") {
        d.parse()
            .map_err(|_| CliError::usage("--delta must be in (0, 1)"))?
    } else {
        1e-5
    };

    let (mechanism, sensitivity, noise, budget) = parse_mechanism_budget(&parsed)?;
    let store_path = resolve_store_path(&parsed)?;
    let mut tracker = load_privacy_tracker(&store_path, max_eps, delta)?;

    let tx_id = format!("tx_direct_{}", tracker.ledger.len() + 1);
    let exp = PrivacyExpenditure {
        transaction_id: tx_id.clone(),
        agent_id: agent_id.to_string(),
        query_id: query_id.to_string(),
        mechanism_name: mechanism.clone(),
        sensitivity,
        noise_parameter: noise,
        rdp_expenditure: budget,
        timestamp_ms: current_timestamp_ms(),
    };

    let updated_eps = tracker.record_expenditure(exp).map_err(map_privacy_error)?;
    let remaining_eps = tracker.remaining_epsilon().map_err(map_privacy_error)?;
    save_privacy_tracker(&store_path, &tracker)?;

    let human = format!(
        "Privacy Expenditure Recorded:\n  Transaction ID: {tx_id}\n  Agent ID: {agent_id}\n  Mechanism: {mechanism} (sensitivity={sensitivity:.4}, noise={noise:.4})\n  Updated Global Epsilon: {updated_eps:.4} / {:.4}\n  Remaining Epsilon: {remaining_eps:.4}",
        tracker.global_max_epsilon
    );

    Ok(success(
        "privacy record",
        json!({
            "transaction_id": tx_id,
            "agent_id": agent_id,
            "query_id": query_id,
            "mechanism": mechanism,
            "sensitivity": sensitivity,
            "noise_parameter": noise,
            "current_epsilon": updated_eps,
            "remaining_epsilon": remaining_eps,
            "global_max_epsilon": tracker.global_max_epsilon,
        }),
        human,
    ))
}

fn simulate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "queries",
            "agents",
            "sensitivity",
            "sigma",
            "delta",
            "max-epsilon",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "privacy simulate does not accept positional arguments",
        ));
    }

    let queries: usize = if let Some(q) = parsed.value("queries") {
        q.parse()
            .map_err(|_| CliError::usage("--queries must be a positive integer"))?
    } else {
        16
    };
    if queries == 0 {
        return Err(CliError::usage("--queries must be greater than 0"));
    }

    let agents: usize = if let Some(a) = parsed.value("agents") {
        a.parse()
            .map_err(|_| CliError::usage("--agents must be a positive integer"))?
    } else {
        4
    };
    if agents == 0 {
        return Err(CliError::usage("--agents must be greater than 0"));
    }

    let sensitivity: f64 = if let Some(s) = parsed.value("sensitivity") {
        s.parse()
            .map_err(|_| CliError::usage("--sensitivity must be a positive number"))?
    } else {
        1.0
    };
    let sigma: f64 = if let Some(s) = parsed.value("sigma") {
        s.parse()
            .map_err(|_| CliError::usage("--sigma must be a positive number"))?
    } else {
        4.0
    };
    let delta: f64 = if let Some(d) = parsed.value("delta") {
        d.parse()
            .map_err(|_| CliError::usage("--delta must be in (0, 1)"))?
    } else {
        1e-5
    };
    let max_eps: f64 = if let Some(m) = parsed.value("max-epsilon") {
        m.parse()
            .map_err(|_| CliError::usage("--max-epsilon must be a positive number"))?
    } else {
        25.0
    };

    let mut tracker = DistributedPrivacyTracker::new(max_eps, delta).map_err(map_privacy_error)?;
    let step_budget = RdpBudget::from_gaussian(sensitivity, sigma).map_err(map_privacy_error)?;

    let mut admitted_queries = 0usize;
    let mut rejected_queries = 0usize;
    for q in 0..queries {
        let agent_id = format!("agent_{}", q % agents);
        let query_id = format!("sim_q_{q}");
        match tracker.reserve(&agent_id, &query_id, &step_budget) {
            Ok(res_id) => {
                tracker
                    .commit_reservation(&res_id)
                    .map_err(map_privacy_error)?;
                admitted_queries += 1;
            }
            Err(PrivacyAccountantError::BudgetExceeded { .. }) => {
                rejected_queries += 1;
            }
            Err(other) => return Err(map_privacy_error(other)),
        }
    }

    let (grid_alpha, grid_eps) = tracker
        .cumulative_budget
        .optimal_order_for_delta(delta)
        .map_err(map_privacy_error)?;
    let (cont_alpha, cont_eps) =
        RdpBudget::continuous_gaussian_optimum(admitted_queries, sensitivity, sigma, delta)
            .map_err(map_privacy_error)?;
    let naive_eps_per_step = RdpBudget::continuous_gaussian_optimum(1, sensitivity, sigma, delta)
        .map_err(map_privacy_error)?
        .1;
    let naive_linear_eps = (admitted_queries as f64) * naive_eps_per_step;
    let composition_savings_ratio = if grid_eps > 0.0 {
        naive_linear_eps / grid_eps
    } else {
        1.0
    };

    let human = format!(
        "Rényi Differential Privacy Composition Benchmark (Theorems 17 & 18):\n  Queries Requested: {queries} across {agents} agents\n  Queries Admitted: {admitted_queries} (Rejected by Budget Gate: {rejected_queries})\n  Gaussian Mechanism: Delta_2={sensitivity:.4}, sigma={sigma:.4}, delta={delta:.2e}\n  Grid-Optimal Order alpha* ({} orders): {grid_alpha:.2} -> epsilon(delta) = {grid_eps:.6}\n  Continuous Analytic Optimum: alpha* = {cont_alpha:.4} -> epsilon*(delta) = {cont_eps:.6}\n  Naive Basic Composition Bound: {naive_linear_eps:.6}\n  Sublinear O(sqrt(k)) Tightening Factor: {composition_savings_ratio:.2}x tighter than naive composition",
        STANDARD_RDP_ORDERS.len()
    );

    Ok(success(
        "privacy simulate",
        json!({
            "queries_requested": queries,
            "agents_count": agents,
            "queries_admitted": admitted_queries,
            "queries_rejected": rejected_queries,
            "sensitivity": sensitivity,
            "sigma": sigma,
            "delta": delta,
            "max_epsilon": max_eps,
            "grid_optimal_alpha": grid_alpha,
            "grid_converted_epsilon": grid_eps,
            "continuous_optimal_alpha": cont_alpha,
            "continuous_optimal_epsilon": cont_eps,
            "naive_linear_composition_epsilon": naive_linear_eps,
            "composition_savings_ratio": composition_savings_ratio,
        }),
        human,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_privacy_simulate_sublinear_tightening() {
        let res = execute(&[
            "simulate".to_string(),
            "--queries".to_string(),
            "16".to_string(),
            "--sigma".to_string(),
            "4.0".to_string(),
        ])
        .expect("simulate should succeed");
        assert_eq!(res.command, "privacy simulate");
        assert_eq!(res.data["queries_admitted"], 16);
        let grid_eps = res.data["grid_converted_epsilon"].as_f64().unwrap();
        let cont_eps = res.data["continuous_optimal_epsilon"].as_f64().unwrap();
        let naive_eps = res.data["naive_linear_composition_epsilon"]
            .as_f64()
            .unwrap();
        assert!(grid_eps >= cont_eps - 1e-9);
        assert!(grid_eps < naive_eps);
    }
}
