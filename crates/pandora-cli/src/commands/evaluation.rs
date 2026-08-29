use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::commands::run::evaluation_receipt_json;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    EvaluationEngine, EvaluationScheduleStore, EvaluationSuiteStore, EvaluationTarget,
    EvaluationTargetKind, GoldenCase, GoldenSetReport, MAX_CLAIM_BATCH, MAX_GOLDEN_CASES,
};
use pandora_types::{
    EvaluationReceipt, EvaluationRequest, EvaluationStatus, ExecutionId, JobWorkerId, RunLoopId,
    SessionId, hash_artifact,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_EVALUATION_INPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EVALUATION_TASK_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
struct GoldenSetInput {
    #[serde(default)]
    suite_id: Option<String>,
    cases: Vec<GoldenCaseInput>,
}

#[derive(Debug, Deserialize)]
struct EvaluationTargetInput {
    kind: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct GoldenCaseInput {
    id: String,
    #[serde(default)]
    target: Option<EvaluationTargetInput>,
    #[serde(default)]
    task: Option<String>,
    execution_id: String,
    output: String,
    expected_output: String,
    #[serde(default)]
    policy_violations: Vec<String>,
    terminal_failure: Option<String>,
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "evaluation requires 'golden', 'inspect', 'scorecard', 'suite', or 'schedule'",
        )
    })?;
    match subcommand.as_str() {
        "golden" => golden(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "scorecard" => scorecard(&args[1..]),
        "suite" => suite(&args[1..]),
        "schedule" => schedule(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown evaluation command '{subcommand}'"
        ))),
    }
}

fn suite(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("evaluation suite requires 'register', 'list', or 'inspect'")
    })?;
    match subcommand.as_str() {
        "register" => suite_register(&args[1..]),
        "list" => suite_list(&args[1..]),
        "inspect" => suite_inspect(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown evaluation suite command '{subcommand}'"
        ))),
    }
}

fn suite_register(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id", "input"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation suite register does not accept positional arguments",
        ));
    }
    let id = required_option(
        &parsed,
        "id",
        "evaluation suite register requires '--id <id>'",
    )?;
    let input = required_option(
        &parsed,
        "input",
        "evaluation suite register requires '--input <path>'",
    )?;
    let bytes = read_bounded(Path::new(input))?;
    let input_suite_id = scheduled_suite_id(&bytes)?;
    if input_suite_id != id {
        return Err(CliError::usage(format!(
            "suite input ID '{}' does not match '--id {}'",
            input_suite_id, id
        )));
    }
    let cases = parse_cases(&bytes)?;
    if cases.is_empty() {
        return Err(CliError::usage(
            "evaluation suite must contain at least one case",
        ));
    }
    let targeted_case_count = cases.iter().filter(|case| case.target().is_some()).count();
    let target_kinds = cases.iter().filter_map(|case| case.target()).fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, target| {
            *counts.entry(target.kind().as_str().to_owned()).or_default() += 1;
            counts
        },
    );
    let report = EvaluationEngine::new()
        .evaluate_golden_set(cases)
        .map_err(|error| CliError::usage(format!("invalid evaluation suite: {error:?}")))?;
    let config = schedule_config(&parsed)?;
    let store = suite_store(&config)?;
    let suite = store
        .register(id, &bytes, crate::commands::timestamp())
        .map_err(suite_error)?;
    let mut data = suite_value(&suite);
    if let Value::Object(object) = &mut data {
        object.insert("case_count".to_owned(), Value::from(report.total()));
        object.insert(
            "targeted_case_count".to_owned(),
            Value::from(targeted_case_count),
        );
        object.insert("target_kinds".to_owned(), json!(target_kinds));
    }
    Ok(success(
        "evaluation suite register",
        data,
        format!("Registered evaluation suite {}", suite.id()),
    ))
}

fn suite_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation suite list does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let suites = suite_store(&config)?.list().map_err(suite_error)?;
    let count = suites.len();
    Ok(success(
        "evaluation suite list",
        json!({
            "suites": suites.iter().map(suite_value).collect::<Vec<_>>(),
            "count": count,
            "durability": "evaluation-suite-store",
        }),
        format!("Listed {count} evaluation suite(s)"),
    ))
}

fn suite_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation suite inspect does not accept positional arguments",
        ));
    }
    let id = required_option(
        &parsed,
        "id",
        "evaluation suite inspect requires '--id <id>'",
    )?;
    let config = schedule_config(&parsed)?;
    let suite = suite_store(&config)?.inspect(id).map_err(suite_error)?;
    Ok(success(
        "evaluation suite inspect",
        suite_value(&suite),
        format!("Inspected evaluation suite {}", suite.id()),
    ))
}

fn schedule(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "evaluation schedule requires 'create', 'list', 'disable', 'claim', or 'run'",
        )
    })?;
    match subcommand.as_str() {
        "create" => schedule_create(&args[1..]),
        "list" => schedule_list(&args[1..]),
        "disable" => schedule_disable(&args[1..]),
        "claim" => schedule_claim(&args[1..]),
        "run" => schedule_run(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown evaluation schedule command '{subcommand}'"
        ))),
    }
}

fn schedule_create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "id",
            "name",
            "suite",
            "interval-seconds",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation schedule create does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let (principal, tenant, workspace) = session_scope();
    let store = schedule_store(&config)?;
    if store
        .list(&principal, &tenant, &workspace)
        .map_err(schedule_error)?
        .len()
        >= pandora_runtime::MAX_SCHEDULES
    {
        return Err(CliError::usage(format!(
            "at most {} evaluation schedules are allowed",
            pandora_runtime::MAX_SCHEDULES
        )));
    }
    let id = schedule_id(&parsed)?;
    let name = required_option(
        &parsed,
        "name",
        "evaluation schedule create requires '--name <name>'",
    )?;
    let suite = required_option(
        &parsed,
        "suite",
        "evaluation schedule create requires '--suite <id>'",
    )?;
    let interval = parse_u64(
        required_option(
            &parsed,
            "interval-seconds",
            "evaluation schedule create requires '--interval-seconds <seconds>'",
        )?,
        "interval-seconds",
    )?;
    let schedule = store
        .create(
            &id,
            &principal,
            &tenant,
            &workspace,
            name,
            suite,
            interval,
            crate::commands::timestamp(),
        )
        .map_err(schedule_error)?;
    Ok(success(
        "evaluation schedule create",
        schedule_value(&schedule),
        format!("Created evaluation schedule {}", schedule.id()),
    ))
}

fn schedule_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation schedule list does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let (principal, tenant, workspace) = session_scope();
    let schedules = schedule_store(&config)?
        .list(&principal, &tenant, &workspace)
        .map_err(schedule_error)?;
    let count = schedules.len();
    Ok(success(
        "evaluation schedule list",
        json!({"schedules": schedules.iter().map(schedule_value).collect::<Vec<_>>(), "count": count, "durability": "schedule-store"}),
        format!("Listed {count} evaluation schedule(s)"),
    ))
}

fn schedule_disable(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation schedule disable does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let (principal, tenant, workspace) = session_scope();
    let id = schedule_id(&parsed)?;
    schedule_store(&config)?
        .disable(&id, &principal, &tenant, &workspace)
        .map_err(schedule_error)?;
    Ok(success(
        "evaluation schedule disable",
        json!({"id": id, "enabled": false, "durability": "schedule-store"}),
        format!("Disabled evaluation schedule {id}"),
    ))
}

fn schedule_claim(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "worker", "limit"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation schedule claim does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let worker = JobWorkerId::new(
        required_option(
            &parsed,
            "worker",
            "evaluation schedule claim requires '--worker <id>'",
        )?
        .to_owned(),
    )
    .map_err(|_| CliError::usage("worker ID is invalid"))?;
    let limit = parsed
        .value("limit")
        .map(|value| parse_u64(value, "limit"))
        .transpose()?
        .unwrap_or(MAX_CLAIM_BATCH as u64);
    let (principal, tenant, workspace) = session_scope();
    let runs = schedule_store(&config)?
        .claim_due(
            &principal,
            &tenant,
            &workspace,
            &worker,
            crate::commands::timestamp(),
            limit as usize,
        )
        .map_err(schedule_error)?;
    let count = runs.len();
    Ok(success(
        "evaluation schedule claim",
        json!({"runs": runs.iter().map(schedule_run_value).collect::<Vec<_>>(), "count": count, "worker": worker, "durability": "schedule-store"}),
        format!("Claimed {count} due evaluation schedule run(s)"),
    ))
}

fn schedule_run(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "id",
            "worker",
            "input",
            "fail-on-failure",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation schedule run does not accept positional arguments",
        ));
    }
    let config = schedule_config(&parsed)?;
    let id = schedule_id(&parsed)?;
    let worker = JobWorkerId::new(
        required_option(
            &parsed,
            "worker",
            "evaluation schedule run requires '--worker <id>'",
        )?
        .to_owned(),
    )
    .map_err(|_| CliError::usage("worker ID is invalid"))?;
    let (principal, tenant, workspace) = session_scope();
    let store = schedule_store(&config)?;
    let schedule = store
        .list(&principal, &tenant, &workspace)
        .map_err(schedule_error)?
        .into_iter()
        .find(|schedule| schedule.id() == &id)
        .ok_or_else(|| {
            CliError::execution(
                "evaluation schedule was not found",
                json!({
                    "schedule_id": id,
                    "durability": "schedule-store",
                }),
            )
        })?;
    let suite_store = suite_store(&config)?;
    let bytes = if let Some(input) = parsed.value("input") {
        read_bounded(Path::new(input))?
    } else {
        suite_store.load(schedule.suite_id()).map_err(suite_error)?
    };
    let suite_id = scheduled_suite_id(&bytes)?;
    if schedule.suite_id() != suite_id {
        return Err(CliError::usage(format!(
            "scheduled input suite '{}' does not match schedule suite '{}'",
            suite_id,
            schedule.suite_id()
        )));
    }
    let cases = parse_cases(&bytes)?;
    let mut runs = store
        .claim_due_for(
            &principal,
            &tenant,
            &workspace,
            Some(&id),
            &worker,
            crate::commands::timestamp(),
            1,
        )
        .map_err(schedule_error)?;
    let run = runs.pop().ok_or_else(|| {
        CliError::execution(
            "evaluation schedule has no due run",
            json!({
                "schedule_id": id,
                "worker": worker,
                "durability": "schedule-store",
            }),
        )
    })?;
    let report = EvaluationEngine::new()
        .evaluate_golden_set(cases)
        .map_err(|error| {
            let _ = store.complete(
                run.schedule_id(),
                &principal,
                &tenant,
                &workspace,
                run.scheduled_for(),
                &worker,
                false,
                crate::commands::timestamp(),
            );
            CliError::usage(format!("invalid scheduled golden set: {error:?}"))
        })?;
    let passed = report.failed() == 0;
    let finished_at = crate::commands::timestamp();
    store
        .complete(
            run.schedule_id(),
            &principal,
            &tenant,
            &workspace,
            run.scheduled_for(),
            &worker,
            passed,
            finished_at,
        )
        .map_err(schedule_error)?;
    let mut completed_run = schedule_run_value(&run);
    if let Value::Object(object) = &mut completed_run {
        object.insert(
            "status".to_owned(),
            Value::String(if passed { "completed" } else { "failed" }.to_owned()),
        );
        object.insert(
            "finished_at".to_owned(),
            Value::from(finished_at.as_unix_seconds()),
        );
        object.insert("lease_until".to_owned(), Value::Null);
    }
    let data = json!({
        "run": completed_run,
        "report": report_value(&report),
        "completed": true,
        "passed": passed,
        "durability": "schedule-store",
    });
    if parsed.values.contains_key("fail-on-failure") && !passed {
        return Err(CliError::execution(
            "scheduled golden-set evaluation failed",
            data,
        ));
    }
    Ok(success(
        "evaluation schedule run",
        data,
        format!(
            "Scheduled golden set: {}/{} passed (digest {})",
            report.passed(),
            report.total(),
            report.digest()
        ),
    ))
}

fn schedule_config(
    parsed: &super::ParsedArgs,
) -> Result<pandora_runtime::config::RuntimeConfig, CliError> {
    let config = load_config(parsed)?;
    require_config_file(&config)?;
    Ok(config)
}

fn suite_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<EvaluationSuiteStore, CliError> {
    EvaluationSuiteStore::open(config.data_dir().join("evaluation-suites.sqlite3"))
        .map_err(suite_error)
}

fn schedule_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<EvaluationScheduleStore, CliError> {
    EvaluationScheduleStore::open(config.data_dir().join("evaluation-schedules.sqlite3"))
        .map_err(schedule_error)
}

fn schedule_id(parsed: &super::ParsedArgs) -> Result<RunLoopId, CliError> {
    RunLoopId::new(
        required_option(
            parsed,
            "id",
            "evaluation schedule command requires '--id <id>'",
        )?
        .to_owned(),
    )
    .map_err(|_| CliError::usage("schedule ID is invalid"))
}

fn required_option<'a>(
    parsed: &'a super::ParsedArgs,
    name: &str,
    message: &'static str,
) -> Result<&'a str, CliError> {
    parsed.value(name).ok_or_else(|| CliError::usage(message))
}

fn parse_u64(value: &str, name: &str) -> Result<u64, CliError> {
    value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("{name} must be an unsigned integer")))
}

fn suite_value(suite: &pandora_runtime::EvaluationSuite) -> Value {
    json!({
        "id": suite.id(),
        "digest": suite.digest(),
        "definition_bytes": suite.definition_bytes(),
        "created_at": suite.created_at().as_unix_seconds(),
        "durability": "evaluation-suite-store",
    })
}

fn suite_error(error: pandora_runtime::EvaluationSuiteError) -> CliError {
    CliError::execution(
        error.to_string(),
        json!({"durability": "evaluation-suite-store"}),
    )
}

fn schedule_value(schedule: &pandora_runtime::EvaluationSchedule) -> Value {
    json!({"id": schedule.id(), "name": schedule.name(), "suite_id": schedule.suite_id(), "interval_seconds": schedule.interval_seconds(), "next_run_at": schedule.next_run_at(), "enabled": schedule.enabled(), "created_at": schedule.created_at(), "last_claimed_at": schedule.last_claimed_at(), "run_count": schedule.run_count(), "scope": {"principal_id": schedule.principal_id(), "tenant_id": schedule.tenant_id(), "workspace_id": schedule.workspace_id()}})
}

fn schedule_run_value(run: &pandora_runtime::EvaluationScheduleRun) -> Value {
    json!({"schedule_id": run.schedule_id(), "suite_id": run.suite_id(), "scheduled_for": run.scheduled_for(), "status": run.status().as_str(), "worker_id": run.worker_id(), "claimed_at": run.claimed_at(), "lease_until": run.lease_until(), "finished_at": run.finished_at()})
}

fn schedule_error(error: pandora_runtime::EvaluationScheduleError) -> CliError {
    CliError::execution(error.to_string(), json!({"durability": "schedule-store"}))
}

fn scorecard(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "fail-on-non-passed",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation scorecard does not accept positional arguments",
        ));
    }
    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("evaluation scorecard requires '--session <id>'"))
        .and_then(|value| {
            SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
        })?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let snapshot = store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let receipts = snapshot.evaluations();
    let mut by_kind = BTreeMap::<String, ScorecardBucket>::new();
    let mut result_count = 0usize;
    let mut score_sum = 0u64;
    for receipt in receipts {
        for result in receipt.results() {
            result_count += 1;
            score_sum += u64::from(result.score());
            by_kind
                .entry(result.kind().as_str().to_owned())
                .or_default()
                .record(result);
        }
    }
    let receipt_values = receipts
        .iter()
        .map(evaluation_receipt_json)
        .collect::<Vec<_>>();
    let digest = hash_artifact(
        &serde_json::to_vec(&receipt_values)
            .map_err(|_| CliError::internal("could not digest evaluation scorecard", json!({})))?,
    );
    let (passed, failed, review_required) = status_counts(&receipts.iter().collect::<Vec<_>>());
    let data = json!({
        "session_id": session_id,
        "receipt_count": receipts.len(),
        "result_count": result_count,
        "result_counts": {
            "passed": passed,
            "failed": failed,
            "human_review_required": review_required,
        },
        "score_sum": score_sum,
        "average_score": (result_count > 0).then(|| score_sum / result_count as u64),
        "pass_rate_percent": (result_count > 0).then(|| (passed as u64 * 100) / result_count as u64),
        "by_kind": by_kind,
        "digest": digest,
        "durability": "session-store",
    });
    if parsed.values.contains_key("fail-on-non-passed") && (failed > 0 || review_required > 0) {
        return Err(CliError::execution(
            "evaluation scorecard quality gate failed",
            data,
        ));
    }
    Ok(success(
        "evaluation scorecard",
        data,
        format!(
            "Scored {result_count} evaluation result(s) across {} receipt(s) for {session_id}",
            receipts.len()
        ),
    ))
}

#[derive(Default, serde::Serialize)]
struct ScorecardBucket {
    count: usize,
    passed: usize,
    failed: usize,
    human_review_required: usize,
    score_sum: u64,
}

impl ScorecardBucket {
    fn record(&mut self, result: &pandora_types::EvaluationResult) {
        self.count += 1;
        self.score_sum += u64::from(result.score());
        match result.status() {
            EvaluationStatus::Passed => self.passed += 1,
            EvaluationStatus::Failed => self.failed += 1,
            EvaluationStatus::HumanReviewRequired => self.human_review_required += 1,
        }
    }
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "execution"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation inspect does not accept positional arguments",
        ));
    }
    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("evaluation inspect requires '--session <id>'"))
        .and_then(|value| {
            SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
        })?;
    let execution_id = parsed
        .value("execution")
        .map(|value| {
            ExecutionId::new(value.to_owned())
                .map_err(|_| CliError::usage("execution ID is invalid"))
        })
        .transpose()?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let snapshot = store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let receipts = snapshot
        .evaluations()
        .iter()
        .filter(|receipt| {
            execution_id
                .as_ref()
                .is_none_or(|id| receipt.execution_id() == id)
        })
        .collect::<Vec<_>>();
    if execution_id.is_some() && receipts.is_empty() {
        return Err(CliError::execution(
            "evaluation execution was not found in the session",
            json!({"session_id": session_id, "execution_id": execution_id}),
        ));
    }
    let (passed, failed, review_required) = status_counts(&receipts);
    let count = receipts.len();
    Ok(success(
        "evaluation inspect",
        json!({
            "session_id": session_id,
            "execution_id": execution_id,
            "count": count,
            "result_counts": {
                "passed": passed,
                "failed": failed,
                "human_review_required": review_required,
            },
            "receipts": receipts
                .iter()
                .map(|receipt| evaluation_receipt_json(receipt))
                .collect::<Vec<_>>(),
            "durability": "session-store",
        }),
        format!("Inspected {count} evaluation receipt(s) for {}", session_id),
    ))
}

fn status_counts(receipts: &[&EvaluationReceipt]) -> (usize, usize, usize) {
    receipts.iter().fold((0, 0, 0), |mut counts, receipt| {
        for result in receipt.results() {
            match result.status() {
                EvaluationStatus::Passed => counts.0 += 1,
                EvaluationStatus::Failed => counts.1 += 1,
                EvaluationStatus::HumanReviewRequired => counts.2 += 1,
            }
        }
        counts
    })
}

fn golden(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["input", "fail-on-failure"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation golden does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evaluation golden requires '--input <path>'"))?;
    let bytes = read_bounded(Path::new(input))?;
    let cases = parse_cases(&bytes)?;
    let report = EvaluationEngine::new()
        .evaluate_golden_set(cases)
        .map_err(|error| CliError::usage(format!("invalid golden set: {error:?}")))?;
    let data = report_value(&report);
    if parsed.values.contains_key("fail-on-failure") && report.failed() > 0 {
        return Err(CliError::execution("golden-set evaluation failed", data));
    }
    Ok(success(
        "evaluation golden",
        data,
        format!(
            "Golden set: {}/{} passed (digest {})",
            report.passed(),
            report.total(),
            report.digest()
        ),
    ))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::execution(
            "could not read golden-set input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.len() > MAX_EVALUATION_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "golden-set input exceeds {MAX_EVALUATION_INPUT_BYTES} bytes"
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::execution(
            "could not open golden-set input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_EVALUATION_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read golden-set input",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.len() as u64 > MAX_EVALUATION_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "golden-set input exceeds {MAX_EVALUATION_INPUT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn scheduled_suite_id(bytes: &[u8]) -> Result<String, CliError> {
    let suite_id = serde_json::from_slice::<GoldenSetInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid scheduled golden-set JSON: {error}")))?
        .suite_id
        .ok_or_else(|| CliError::usage("scheduled golden-set input requires 'suite_id'"))?;
    if suite_id.trim().is_empty()
        || suite_id.len() > pandora_runtime::MAX_EVALUATION_SUITE_BYTES
        || suite_id.chars().any(char::is_control)
    {
        return Err(CliError::usage("scheduled golden-set suite_id is invalid"));
    }
    Ok(suite_id.trim().to_owned())
}

fn parse_target(case: &GoldenCaseInput) -> Result<Option<EvaluationTarget>, CliError> {
    match (&case.target, &case.task) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(CliError::usage(
            "evaluation target metadata requires a bounded 'task' field",
        )),
        (None, Some(_)) => Err(CliError::usage(
            "evaluation task metadata requires a 'target' object",
        )),
        (Some(target), Some(task)) => {
            if task.trim().is_empty()
                || task.len() > MAX_EVALUATION_TASK_BYTES
                || task.chars().any(char::is_control)
            {
                return Err(CliError::usage(
                    "evaluation task must be 1-16384 bytes without control characters",
                ));
            }
            let kind = match target.kind.as_str() {
                "prompt" => EvaluationTargetKind::Prompt,
                "skill" => EvaluationTargetKind::Skill,
                "workflow" => EvaluationTargetKind::Workflow,
                "wasm_gene" => EvaluationTargetKind::WasmGene,
                _ => {
                    return Err(CliError::usage(
                        "evaluation target kind must be prompt, skill, workflow, or wasm_gene",
                    ));
                }
            };
            EvaluationTarget::new(kind, target.id.clone())
                .map(Some)
                .map_err(|error| CliError::usage(format!("invalid evaluation target: {error:?}")))
        }
    }
}

fn parse_cases(bytes: &[u8]) -> Result<Vec<GoldenCase>, CliError> {
    let input = serde_json::from_slice::<GoldenSetInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid golden-set JSON: {error}")))?;
    if input.cases.len() > MAX_GOLDEN_CASES {
        return Err(CliError::usage(format!(
            "golden set contains more than {MAX_GOLDEN_CASES} cases"
        )));
    }
    input
        .cases
        .into_iter()
        .map(|case| {
            let target = parse_target(&case)?;
            let execution_id = ExecutionId::new(case.execution_id)
                .map_err(|error| CliError::usage(format!("invalid execution_id: {error}")))?;
            let mut evaluation = EvaluationRequest::new(
                execution_id,
                Vec::new(),
                case.output,
                case.policy_violations,
            )
            .map_err(|error| CliError::usage(format!("invalid golden case: {error}")))?;
            if let Some(failure) = case.terminal_failure {
                evaluation = evaluation
                    .with_terminal_failure(failure)
                    .map_err(|error| CliError::usage(format!("invalid golden case: {error}")))?;
            }
            let golden = GoldenCase::new(case.id, evaluation, case.expected_output)
                .map_err(|error| CliError::usage(format!("invalid golden case: {error:?}")))?;
            Ok(if let Some(target) = target {
                golden.with_target(target)
            } else {
                golden
            })
        })
        .collect()
}

fn report_value(report: &GoldenSetReport) -> Value {
    json!({
        "total": report.total(),
        "passed": report.passed(),
        "failed": report.failed(),
        "digest": report.digest(),
        "cases": report.cases().iter().map(|case| {
            let result = case.result();
            json!({
                "id": case.id(),
                "kind": result.kind().as_str(),
                "status": result.status().as_str(),
                "score": result.score(),
                "reason": result.reason(),
                "advisory": result.advisory(),
                "target": case.target().map(|target| json!({"kind": target.kind().as_str(), "id": target.id()})),
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_cases, scheduled_suite_id, status_counts};
    use pandora_types::{
        EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus, ExecutionId,
        SessionId, Timestamp,
    };

    #[test]
    fn parses_bounded_golden_case_shape() {
        let cases = parse_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"done","expected_output":"done"}]}"#,
        )
        .unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id(), "case-a");
    }

    #[test]
    fn parses_supported_task_backed_targets() {
        let cases = parse_cases(
            br#"{"cases":[
                {"id":"prompt-case","target":{"kind":"prompt","id":"prompt-1"},"task":"answer safely","execution_id":"exec-prompt","output":"done","expected_output":"done"},
                {"id":"skill-case","target":{"kind":"skill","id":"skill-1"},"task":"apply skill","execution_id":"exec-skill","output":"done","expected_output":"done"},
                {"id":"workflow-case","target":{"kind":"workflow","id":"workflow-1"},"task":"run workflow","execution_id":"exec-workflow","output":"done","expected_output":"done"},
                {"id":"gene-case","target":{"kind":"wasm_gene","id":"gene-1"},"task":"evaluate gene","execution_id":"exec-gene","output":"done","expected_output":"done"}
            ]}"#,
        )
        .unwrap();

        let kinds = cases
            .iter()
            .map(|case| case.target().unwrap().kind().as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["prompt", "skill", "workflow", "wasm_gene"]);
    }

    #[test]
    fn targeted_cases_require_a_task_label() {
        let error = parse_cases(
            br#"{"cases":[{"id":"case-a","target":{"kind":"prompt","id":"prompt-1"},"execution_id":"exec-a","output":"done","expected_output":"done"}]}"#,
        )
        .unwrap_err();
        assert!(error.message.contains("bounded"));
    }

    #[test]
    fn scheduled_inputs_require_a_bounded_suite_id() {
        assert_eq!(
            scheduled_suite_id(br#"{"suite_id":"nightly","cases":[]}"#).unwrap(),
            "nightly"
        );
        let error = scheduled_suite_id(br#"{"cases":[]}"#).unwrap_err();
        assert!(error.message.contains("requires 'suite_id'"));
    }

    #[test]
    fn rejects_missing_required_fields() {
        let error = parse_cases(br#"{"cases":[{"id":"case-a"}]}"#).unwrap_err();
        assert!(error.message.contains("invalid golden-set JSON"));
    }

    #[test]
    fn preserves_terminal_failure_for_trajectory_evaluation() {
        let cases = parse_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"done","expected_output":"done","terminal_failure":"stopped"}]}"#,
        )
        .unwrap();
        assert!(cases[0].evaluation().terminal_failure().is_some());
    }

    #[test]
    fn counts_all_persisted_evaluation_result_statuses() {
        let receipt = EvaluationReceipt::new(
            SessionId::new("session-a").unwrap(),
            ExecutionId::new("execution-a").unwrap(),
            Timestamp::from_unix_seconds(1),
            vec![
                EvaluationResult::new(
                    EvaluationKind::Trajectory,
                    EvaluationStatus::Passed,
                    100,
                    "ok",
                    false,
                )
                .unwrap(),
                EvaluationResult::new(
                    EvaluationKind::Outcome,
                    EvaluationStatus::Failed,
                    0,
                    "failed",
                    false,
                )
                .unwrap(),
                EvaluationResult::new(
                    EvaluationKind::Policy,
                    EvaluationStatus::HumanReviewRequired,
                    50,
                    "review",
                    false,
                )
                .unwrap(),
            ],
        )
        .unwrap();

        assert_eq!(status_counts(&[&receipt]), (1, 1, 1));
    }
}
