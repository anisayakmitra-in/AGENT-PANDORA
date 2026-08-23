use super::efficiency::parse_objective;
use super::provider::configured_provider_for;
use super::{
    LOCAL_WORKSPACE, create_session, load_config, parse_options, require_config_file,
    session_store, timestamp,
};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{CODING_HARNESS_ID, HarnessCatalog};
use pandora_provider::{
    ChatMessage, FallbackPolicy, ModelRequest, TraceMetadata, parse_and_validate,
};
use pandora_runtime::config::RuntimeConfig;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{
    AgentApprovalContext, AgentLoop, AgentLoopError, AgentRunRequest, ApprovalRequest,
    ApprovalStore, EvaluationEngine, MAX_AGENT_TOOL_CALLS, MAX_AGENT_TURNS, RunStatus,
    RuntimeError,
};
use pandora_runtime::{
    DEFAULT_MAX_SAMPLES_PER_TARGET, EfficiencyEngine, EfficiencyStore, ExecutionController,
    PackageState, PackageStore, SkillEngine, SkillError,
};
use pandora_types::{
    Capability, ContextClassification, EfficiencyObjective, EfficiencySample, EvaluationReceipt,
    EvaluationRequest, EvaluationResult, EventPayload, EventType, ExecutionId, HarnessId,
    MemoryKind, MemoryRecord, MemoryScope, Operation, PackageId, PackageKind, PolicyContext,
    Session, SessionId, TaskIntent, WorkspaceId,
};
use serde_json::{Value, json};
use std::fs;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_PLANNED_TASK_BYTES: usize = 8 * 1024;
const DEFAULT_AGENT_MAX_TURNS: u32 = 8;
const DEFAULT_AGENT_MAX_TOOL_CALLS: u32 = 16;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider",
            "session",
            "approval",
            "harness",
            "harness-version",
            "gene",
            "model",
            "task-class",
            "plan",
            "agent",
            "max-turns",
            "max-tools",
            "optimize",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("run requires exactly one task"));
    }
    if parsed.value("approval").is_some()
        && (parsed.value("plan").is_some() || parsed.value("model").is_some())
    {
        return Err(CliError::usage(
            "--approval cannot be combined with --plan or --model",
        ));
    }
    if parsed.value("agent").is_some()
        && (parsed.value("plan").is_some()
            || parsed.value("harness").is_some()
            || parsed.value("gene").is_some()
            || parsed.value("harness-version").is_some())
    {
        return Err(CliError::usage(
            "--agent cannot be combined with --plan, --harness, or --gene",
        ));
    }
    if parsed.value("agent").is_none()
        && (parsed.value("max-turns").is_some() || parsed.value("max-tools").is_some())
    {
        return Err(CliError::usage(
            "--max-turns and --max-tools require --agent",
        ));
    }
    if parsed.value("optimize").is_some() && parsed.value("provider").is_some() {
        return Err(CliError::usage(
            "--optimize cannot be combined with --provider",
        ));
    }
    if parsed.value("optimize").is_some() && parsed.value("model").is_some() {
        return Err(CliError::usage(
            "--optimize cannot be combined with --model",
        ));
    }
    if parsed.value("optimize").is_some()
        && parsed.value("agent").is_none()
        && parsed.value("plan").is_none()
    {
        return Err(CliError::usage("--optimize requires --agent or --plan"));
    }
    if parsed.value("optimize").is_some() && parsed.value("approval").is_some() {
        return Err(CliError::usage(
            "--optimize cannot be combined with --approval",
        ));
    }
    if parsed.value("harness-version").is_some() && parsed.value("harness").is_none() {
        return Err(CliError::usage("--harness-version requires --harness <id>"));
    }
    let max_turns = parse_agent_budget(
        parsed.value("max-turns"),
        "max-turns",
        DEFAULT_AGENT_MAX_TURNS,
        MAX_AGENT_TURNS,
    )?;
    let max_tool_calls = parse_agent_budget(
        parsed.value("max-tools"),
        "max-tools",
        DEFAULT_AGENT_MAX_TOOL_CALLS,
        MAX_AGENT_TOOL_CALLS,
    )?;
    let task_class = parsed.value("task-class").unwrap_or("general");
    validate_task_class(task_class)?;
    let optimization = parsed.value("optimize").map(parse_objective).transpose()?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let harnesses = configured_harnesses(
        &config,
        parsed.value("harness"),
        parsed.value("harness-version"),
    )?;
    require_runnable_harness(&harnesses, parsed.value("harness"))?;
    let optimized_provider = optimization
        .map(|objective| select_provider(&config, task_class, objective))
        .transpose()?
        .flatten();
    let store = session_store(&config)?;
    let approval_store = ApprovalStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let workspace_id = WorkspaceId::new(LOCAL_WORKSPACE).expect("built-in workspace ID is valid");
    let principal = super::session_scope().0;
    let approval = parsed
        .value("approval")
        .map(|id| {
            approval_store
                .inspect(id, &principal)
                .map_err(approval_error)
        })
        .transpose()?;
    let (session, agent_history) = match approval.as_ref() {
        Some(approval) => {
            if let Some(value) = parsed.value("session")
                && value != approval.session_id().as_str()
            {
                return Err(CliError::policy(
                    "approval is bound to a different session",
                    json!({}),
                ));
            }
            let snapshot = resume_session(&store, approval.session_id().as_str())?;
            (
                snapshot.session().clone(),
                snapshot.agent_messages().to_vec(),
            )
        }
        None => match parsed.value("session") {
            Some(value) => {
                let snapshot = resume_session(&store, value)?;
                (
                    snapshot.session().clone(),
                    snapshot.agent_messages().to_vec(),
                )
            }
            None => (create_session(&store, &workspace_id)?, Vec::new()),
        },
    };
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|error| {
        let _ = error;
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let policy = PolicyContext::new(
        1,
        [
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::ProcessExecute,
            Capability::ProviderInvoke,
        ],
        [Operation::Write, Operation::Execute],
    );
    let controller = ExecutionController::with_policy_and_harnesses(workspace, policy, harnesses);
    let (task, planning_model) = if parsed.value("plan").is_some() {
        plan_task(
            &config,
            &controller,
            &session,
            &parsed.positionals[0],
            parsed.value("model"),
            optimized_provider.as_deref(),
        )?
    } else {
        (parsed.positionals[0].clone(), None)
    };
    if parsed.value("agent").is_some() {
        return execute_agent_core(
            &config,
            &controller,
            &session,
            &store,
            &approval_store,
            AgentOptions {
                task: &task,
                task_class,
                history: agent_history,
                approval_id: parsed.value("approval"),
                model_override: parsed.value("model"),
                provider_name: optimized_provider.as_deref(),
                optimization,
                max_turns,
                max_tool_calls,
                control: None,
            },
        );
    }
    let mut intent =
        TaskIntent::new(task.clone()).map_err(|error| CliError::usage(error.to_string()))?;
    if let Some(harness) = parsed.value("harness") {
        let harness = canonical_harness_id(harness);
        let harness = HarnessId::new(harness.to_owned())
            .map_err(|_| CliError::usage("Harness ID is invalid"))?;
        intent = intent.with_harness(harness);
    }
    if let Some(gene) = parsed.value("gene") {
        let gene = pandora_types::GeneId::new(gene.to_owned())
            .map_err(|_| CliError::usage("Gene ID is invalid"))?;
        intent = intent.with_gene(gene);
    }
    let started = Instant::now();
    let summary = match parsed.value("approval") {
        Some(approval_id) => controller
            .run_with_approval(
                intent,
                session.clone(),
                &approval_store,
                approval_id,
                timestamp(),
            )
            .map_err(runtime_error)?,
        None => controller
            .run_at(intent, session.clone(), timestamp())
            .map_err(runtime_error)?,
    };
    let evaluation = evaluate_and_append_execution(&store, &session, &summary)?;
    let evaluation_value = evaluation_json(&evaluation);
    let memory_evidence_recorded = record_execution_evidence(&store, &session, "local", &summary);
    let efficiency_recorded =
        record_execution_efficiency(&config, task_class, &summary, elapsed_millis(started));
    let details = add_optimization(
        run_details(
            &summary,
            session.id(),
            planning_model.as_deref(),
            evaluation_value.clone(),
        ),
        optimization,
        optimized_provider.as_deref(),
    );
    match summary.status() {
        RunStatus::Completed => {
            let data = add_planning(
                json!({
                "session_id": session.id(),
                "execution_id": summary.execution_id(),
                "harness_id": summary.selected_harness(),
                "gene_id": summary.selected_gene(),
                "status": "completed",
                    "output": summary.output().map(output_text),
                    "efficiency_recorded": efficiency_recorded,
                    "memory_evidence_recorded": memory_evidence_recorded,
                    "evaluation": evaluation_value,
                }),
                planning_model.as_deref(),
            );
            Ok(success(
                "run",
                add_optimization(data, optimization, optimized_provider.as_deref()),
                format!(
                    "Completed {} with {}",
                    summary.selected_gene(),
                    session.id()
                ),
            ))
        }
        RunStatus::Denied { reason } => Err(CliError::policy(reason.clone(), details)),
        RunStatus::ApprovalRequired { reason } => {
            let approval = create_approval(
                &approval_store,
                &summary,
                session.id(),
                session.principal_id(),
                &task,
            )?;
            let details = add_detail(details, "approval_id", approval.id());
            Err(CliError::approval(reason.clone(), details))
        }
        RunStatus::Failed { code } => Err(CliError::execution(code.clone(), details)),
    }
}

pub(super) fn configured_harnesses(
    config: &RuntimeConfig,
    requested: Option<&str>,
    version: Option<&str>,
) -> Result<HarnessCatalog, CliError> {
    let harnesses = HarnessCatalog::builtins();
    let Some(requested) = requested else {
        return Ok(harnesses);
    };
    let requested = canonical_harness_id(requested);
    let harness_id = HarnessId::new(requested.to_owned())
        .map_err(|_| CliError::usage("Harness ID is invalid"))?;
    if let Some(harness) = harnesses.find(&harness_id) {
        if let Some(version) = version
            && version != harness.manifest().version()
        {
            return Err(CliError::usage(format!(
                "built-in Harness '{}' is version {}, not {}",
                requested,
                harness.manifest().version(),
                version
            )));
        }
        return Ok(harnesses);
    }

    let version = version.ok_or_else(|| {
        CliError::usage("custom Domain Harnesses require '--harness-version <version>'")
    })?;
    let package_id = PackageId::new(requested.to_owned())
        .map_err(|_| CliError::usage("package Harness ID is invalid"))?;
    let store = PackageStore::open(config.data_dir().join("packages.sqlite3"))
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
    let record = store
        .get(&package_id, version)
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?
        .ok_or_else(|| {
            CliError::execution(
                "the requested Domain Harness profile is not admitted",
                json!({"id": package_id, "version": version}),
            )
        })?;
    if record.state() != PackageState::Admitted
        || !matches!(
            record.manifest().kind(),
            PackageKind::DomainHarness | PackageKind::MetaHarness
        )
    {
        return Err(CliError::execution(
            "the requested package is not an admitted Harness profile",
            json!({
                "id": record.manifest().id(),
                "version": record.manifest().version(),
                "kind": record.manifest().kind().as_str(),
                "state": record.state().as_str(),
            }),
        ));
    }
    match record.manifest().kind() {
        PackageKind::DomainHarness => harnesses
            .with_declarative_domain(record.manifest())
            .map_err(|error| CliError::execution(error.to_string(), json!({}))),
        PackageKind::MetaHarness => harnesses
            .with_declarative_meta(record.manifest())
            .map_err(|error| CliError::execution(error.to_string(), json!({}))),
        _ => unreachable!("admitted Harness profile kind was validated"),
    }
}

fn canonical_harness_id(value: &str) -> &str {
    match value {
        "coding" => CODING_HARNESS_ID,
        value => value,
    }
}

pub(super) fn require_runnable_harness(
    harnesses: &HarnessCatalog,
    requested: Option<&str>,
) -> Result<(), CliError> {
    let Some(requested) = requested else {
        return Ok(());
    };
    let harness_id = HarnessId::new(canonical_harness_id(requested).to_owned())
        .map_err(|_| CliError::usage("Harness ID is invalid"))?;
    let harness = harnesses
        .find(&harness_id)
        .ok_or_else(|| CliError::execution("requested harness is not supported", json!({})))?;
    if harness.is_runnable() {
        return Ok(());
    }
    Err(CliError::execution(
        "requested harness is not runnable",
        json!({
            "harness_id": harness.manifest().id(),
            "kind": harness.manifest().kind().as_str(),
        }),
    ))
}

pub(super) fn execute_agent_core(
    config: &RuntimeConfig,
    controller: &ExecutionController,
    session: &Session,
    store: &SessionStore,
    approval_store: &ApprovalStore,
    options: AgentOptions<'_>,
) -> Result<CommandResult, CliError> {
    let skill_context = active_skill_context(config)?;
    let model = options
        .model_override
        .or_else(|| {
            options
                .provider_name
                .and_then(|name| config.provider_profile(name).map(|profile| profile.model()))
        })
        .or(config.provider_model())
        .unwrap_or("default");
    let provider = configured_provider_for(config, model, "agent mode", options.provider_name)?;
    let l1_evidence = store
        .l1_evidence_context(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            provider.manifest().id().as_str(),
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let loop_engine = AgentLoop::new(options.max_turns, options.max_tool_calls)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let started = Instant::now();
    let result = match options.approval_id {
        Some(approval_id) => loop_engine.run_with_history_and_approval_and_skill_context(
            provider.as_ref(),
            controller,
            options.history,
            AgentApprovalContext::new(session.clone(), approval_store, approval_id, timestamp())
                .with_l1_evidence(Some(&l1_evidence)),
            skill_context.as_deref(),
            options.task,
        ),
        None => {
            let mut request =
                AgentRunRequest::new(session.clone(), options.history, options.task, timestamp())
                    .with_skill_context(skill_context.as_deref())
                    .with_l1_evidence(Some(&l1_evidence));
            if let Some(control) = options.control {
                request = request.with_control(control);
            }
            loop_engine.run_with_request(provider.as_ref(), controller, request)
        }
    };
    match result {
        Ok(summary) => {
            let evaluations = summary
                .runs()
                .iter()
                .map(|run| evaluate_and_append_execution(store, session, run))
                .collect::<Result<Vec<_>, _>>()?;
            let efficiency_recorded = record_agent_efficiency(
                config,
                session,
                options.task_class,
                provider.as_ref(),
                &summary,
                elapsed_millis(started),
                true,
            );
            store
                .save_agent_transcript(
                    session.id(),
                    session.principal_id(),
                    session.tenant_id(),
                    session.workspace_id(),
                    summary.messages(),
                )
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
            let run_count = summary.runs().len();
            let mut memory_evidence_recorded = 0;
            for run in summary.runs() {
                if record_execution_evidence(store, session, provider.manifest().id().as_str(), run)
                {
                    memory_evidence_recorded += 1;
                }
            }
            Ok(success(
                "run",
                json!({
                    "agent": true,
                    "session_id": session.id(),
                    "status": "completed",
                    "turns": summary.turns(),
                    "tool_calls": summary.tool_calls(),
                    "turn_budget": options.max_turns,
                    "tool_budget": options.max_tool_calls,
                    "provider_calls": summary.provider_receipts().len(),
                    "context": {
                        "included": summary.context_receipt().included_ids(),
                        "dropped": summary.context_receipt().dropped_ids(),
                        "token_cost": summary.context_receipt().token_cost(),
                        "cacheable": summary.context_receipt().cacheable(),
                    },
                    "output": summary.final_text(),
                    "usage": usage_json(summary.usage()),
                    "runs": run_count,
                    "efficiency_recorded": efficiency_recorded,
                    "memory_evidence_recorded": memory_evidence_recorded,
                    "evaluations": evaluations_json(&evaluations),
                    "optimization": optimization_value(
                        options.optimization,
                        options.provider_name,
                    ),
                }),
                format!("Completed agent task in {}", session.id()),
            ))
        }
        Err(AgentLoopError::ApprovalRequired { reason, summary }) => {
            let evaluations = summary
                .runs()
                .iter()
                .map(|run| evaluate_and_append_execution(store, session, run))
                .collect::<Result<Vec<_>, _>>()?;
            let _ = record_agent_efficiency(
                config,
                session,
                options.task_class,
                provider.as_ref(),
                &summary,
                elapsed_millis(started),
                false,
            );
            store
                .save_agent_transcript(
                    session.id(),
                    session.principal_id(),
                    session.tenant_id(),
                    session.workspace_id(),
                    summary.messages(),
                )
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
            let run = summary.runs().last().ok_or_else(|| {
                CliError::internal("approval request has no execution summary", json!({}))
            })?;
            for run in summary.runs() {
                let _ = record_execution_evidence(
                    store,
                    session,
                    provider.manifest().id().as_str(),
                    run,
                );
            }
            let approval = create_approval(
                approval_store,
                run,
                session.id(),
                session.principal_id(),
                options.task,
            )?;
            Err(CliError::approval(
                reason,
                json!({
                    "agent": true,
                    "session_id": session.id(),
                    "turns": summary.turns(),
                    "tool_calls": summary.tool_calls(),
                    "turn_budget": options.max_turns,
                    "tool_budget": options.max_tool_calls,
                    "approval_id": approval.id(),
                    "evaluations": evaluations_json(&evaluations),
                    "context": {
                        "included": summary.context_receipt().included_ids(),
                        "dropped": summary.context_receipt().dropped_ids(),
                        "token_cost": summary.context_receipt().token_cost(),
                        "cacheable": summary.context_receipt().cacheable(),
                    },
                    "optimization": optimization_value(
                        options.optimization,
                        options.provider_name,
                    ),
                }),
            ))
        }
        Err(error) => {
            let _ = record_agent_failure(
                config,
                session,
                options.task_class,
                provider.as_ref(),
                elapsed_millis(started),
            );
            Err(agent_error(error))
        }
    }
}

fn active_skill_context(config: &RuntimeConfig) -> Result<Option<String>, CliError> {
    let root = config.data_dir().join("skills");
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(CliError::configuration(
                "could not inspect the Skill directory",
                json!({"root": root}),
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::configuration(
            "Skill directory is not a regular directory",
            json!({"root": root}),
        ));
    }
    SkillEngine::discover(root)
        .and_then(|engine| engine.active_context())
        .map_err(|error: SkillError| {
            CliError::execution(
                "enabled Skill context is unavailable",
                json!({"error": error.to_string()}),
            )
        })
}

pub(super) struct AgentOptions<'a> {
    pub task: &'a str,
    pub task_class: &'a str,
    pub history: Vec<ChatMessage>,
    pub approval_id: Option<&'a str>,
    pub model_override: Option<&'a str>,
    pub provider_name: Option<&'a str>,
    pub optimization: Option<EfficiencyObjective>,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub control: Option<&'a dyn pandora_runtime::AgentRunControl>,
}

fn parse_agent_budget(
    value: Option<&str>,
    name: &str,
    default: u32,
    maximum: u32,
) -> Result<u32, CliError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let budget = value.parse::<u32>().map_err(|_| {
        CliError::usage(format!(
            "--{name} must be an integer between 1 and {maximum}"
        ))
    })?;
    if budget == 0 || budget > maximum {
        return Err(CliError::usage(format!(
            "--{name} must be an integer between 1 and {maximum}"
        )));
    }
    Ok(budget)
}

fn evaluate_and_append_execution(
    store: &SessionStore,
    session: &Session,
    summary: &pandora_runtime::RunSummary,
) -> Result<EvaluationReceipt, CliError> {
    let status = match summary.status() {
        RunStatus::Completed => "completed",
        RunStatus::Denied { .. } => "denied",
        RunStatus::ApprovalRequired { .. } => "approval_required",
        RunStatus::Failed { .. } => "failed",
    };
    let policy_violations = summary
        .events()
        .iter()
        .filter(|event| event.event_type() == EventType::PolicyDenied)
        .map(|_| "policy_denied".to_owned())
        .collect();
    let mut request = EvaluationRequest::new(
        summary.execution_id().clone(),
        summary.receipts().to_vec(),
        status,
        policy_violations,
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    if let RunStatus::Failed { code } = summary.status() {
        request = request
            .with_terminal_failure(code)
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    }
    let engine = EvaluationEngine::new();
    let mut results = vec![
        engine.evaluate_trajectory(&request, 0),
        engine.evaluate_policy(&request),
    ];
    if matches!(summary.status(), RunStatus::ApprovalRequired { .. }) {
        results.push(engine.require_human_review(&request, "explicit approval is required"));
    }
    let evaluated_at = timestamp();
    let receipt = EvaluationReceipt::new(
        session.id().clone(),
        summary.execution_id().clone(),
        evaluated_at,
        results,
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    store
        .append_execution(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            summary.events(),
            &receipt,
            evaluated_at,
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    Ok(receipt)
}

fn evaluation_json(receipt: &EvaluationReceipt) -> Value {
    json!({
        "recorded": true,
        "outcome_available": false,
        "receipt": evaluation_receipt_json(receipt),
    })
}

fn evaluations_json(receipts: &[EvaluationReceipt]) -> Value {
    json!({
        "count": receipts.len(),
        "outcome_available": false,
        "receipts": receipts
            .iter()
            .map(evaluation_receipt_json)
            .collect::<Vec<_>>(),
    })
}

pub(super) fn evaluation_receipt_json(receipt: &EvaluationReceipt) -> Value {
    json!({
        "session_id": receipt.session_id(),
        "execution_id": receipt.execution_id(),
        "evaluated_at": receipt.evaluated_at().as_unix_seconds(),
        "results": receipt
            .results()
            .iter()
            .map(evaluation_result_json)
            .collect::<Vec<_>>(),
    })
}

fn evaluation_result_json(result: &EvaluationResult) -> Value {
    json!({
        "kind": result.kind().as_str(),
        "status": result.status().as_str(),
        "score": result.score(),
        "reason": result.reason(),
        "advisory": result.advisory(),
    })
}

fn record_execution_evidence(
    store: &SessionStore,
    session: &Session,
    provider: &str,
    summary: &pandora_runtime::RunSummary,
) -> bool {
    let scope = match MemoryScope::new(
        session.tenant_id().clone(),
        session.workspace_id().clone(),
        session.id().clone(),
        provider,
    ) {
        Ok(scope) => scope,
        Err(_) => return false,
    };
    let status = match summary.status() {
        RunStatus::Completed => "completed",
        RunStatus::Denied { .. } => "denied",
        RunStatus::ApprovalRequired { .. } => "approval_required",
        RunStatus::Failed { .. } => "failed",
    };
    let record = MemoryRecord::new_l1(
        summary.execution_id().as_str(),
        MemoryKind::ExecutionEvidence,
        scope,
        format!(
            "{status} execution through {}/{}",
            summary.selected_harness(),
            summary.selected_gene(),
        ),
        ContextClassification::Internal,
        timestamp(),
        format!("execution:{}", summary.execution_id()),
    );
    match record {
        Ok(record) => store
            .record_l1_evidence(session.principal_id(), &record)
            .is_ok(),
        Err(_) => false,
    }
}

fn usage_json(usage: &pandora_provider::TokenUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens(),
        "completion_tokens": usage.completion_tokens(),
        "total_tokens": usage.total_tokens(),
    })
}

fn validate_task_class(value: &str) -> Result<(), CliError> {
    if value.trim().is_empty() {
        return Err(CliError::usage("task class cannot be empty"));
    }
    if value.len() > 128 || value.chars().any(char::is_control) {
        return Err(CliError::usage("task class is invalid or too long"));
    }
    Ok(())
}

fn select_provider(
    config: &RuntimeConfig,
    task_class: &str,
    objective: EfficiencyObjective,
) -> Result<Option<String>, CliError> {
    let store = EfficiencyStore::open(config.data_dir().join("efficiency.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let samples = store
        .load_task_class(task_class)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let engine = EfficiencyEngine::from_samples(DEFAULT_MAX_SAMPLES_PER_TARGET, samples)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let rankings = engine
        .rank(task_class, objective)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;

    for summary in rankings {
        if summary.completed_samples() == 0 {
            continue;
        }
        if objective == EfficiencyObjective::LowestCost && !summary.has_cost_evidence() {
            continue;
        }
        for name in config.provider_names() {
            let Some(profile) = config.provider_profile(&name) else {
                continue;
            };
            let target = format!("{}/{}", profile.name(), profile.model());
            if target == summary.target() {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

fn optimization_value(objective: Option<EfficiencyObjective>, provider: Option<&str>) -> Value {
    match objective {
        Some(objective) => json!({
            "objective": objective.as_str(),
            "provider": provider,
            "evidence_used": provider.is_some(),
        }),
        None => Value::Null,
    }
}

fn add_optimization(
    mut details: Value,
    objective: Option<EfficiencyObjective>,
    provider: Option<&str>,
) -> Value {
    if objective.is_some()
        && let Value::Object(object) = &mut details
    {
        object.insert(
            "optimization".to_owned(),
            optimization_value(objective, provider),
        );
    }
    details
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn record_execution_efficiency(
    config: &RuntimeConfig,
    task_class: &str,
    summary: &pandora_runtime::RunSummary,
    latency_ms: u64,
) -> bool {
    let target = format!("{}/{}", summary.selected_harness(), summary.selected_gene());
    let sample = EfficiencySample::new_without_cost(
        summary.execution_id().clone(),
        task_class,
        target,
        0,
        0,
        latency_ms,
        matches!(summary.status(), RunStatus::Completed),
        timestamp(),
    );
    let Ok(sample) = sample else {
        return false;
    };
    record_efficiency_sample(config, sample)
}

fn record_agent_efficiency(
    config: &RuntimeConfig,
    session: &Session,
    task_class: &str,
    provider: &dyn pandora_provider::Provider,
    summary: &pandora_runtime::AgentRunSummary,
    latency_ms: u64,
    completed: bool,
) -> bool {
    let execution_id = summary
        .runs()
        .last()
        .map(|run| run.execution_id().clone())
        .or_else(|| {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos();
            ExecutionId::new(format!("agent-efficiency-{}-{nonce}", session.id())).ok()
        });
    let Some(execution_id) = execution_id else {
        return false;
    };
    let target = format!(
        "{}/{}",
        provider.manifest().id(),
        provider.manifest().default_model()
    );
    let prompt_tokens = summary.usage().prompt_tokens();
    let completion_tokens = summary.usage().completion_tokens();
    let sample = match config.provider_cost_micros(
        provider.manifest().id().as_str(),
        prompt_tokens,
        completion_tokens,
    ) {
        Some(cost_micros) => EfficiencySample::new(
            execution_id,
            task_class,
            target,
            u64::from(prompt_tokens),
            u64::from(completion_tokens),
            cost_micros,
            latency_ms,
            completed,
            timestamp(),
        ),
        None => EfficiencySample::new_without_cost(
            execution_id,
            task_class,
            target,
            u64::from(prompt_tokens),
            u64::from(completion_tokens),
            latency_ms,
            completed,
            timestamp(),
        ),
    };
    let Ok(sample) = sample else {
        return false;
    };
    record_efficiency_sample(config, sample)
}

fn record_agent_failure(
    config: &RuntimeConfig,
    session: &Session,
    task_class: &str,
    provider: &dyn pandora_provider::Provider,
    latency_ms: u64,
) -> bool {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let Ok(execution_id) = ExecutionId::new(format!("agent-failure-{}-{nonce}", session.id()))
    else {
        return false;
    };
    let sample = EfficiencySample::new_without_cost(
        execution_id,
        task_class,
        format!(
            "{}/{}",
            provider.manifest().id(),
            provider.manifest().default_model()
        ),
        0,
        0,
        latency_ms,
        false,
        timestamp(),
    );
    let Ok(sample) = sample else {
        return false;
    };
    record_efficiency_sample(config, sample)
}

fn record_efficiency_sample(config: &RuntimeConfig, sample: EfficiencySample) -> bool {
    let Ok(store) = EfficiencyStore::open(config.data_dir().join("efficiency.sqlite3")) else {
        return false;
    };
    store
        .record(&sample, DEFAULT_MAX_SAMPLES_PER_TARGET)
        .is_ok()
}

fn agent_error(error: AgentLoopError) -> CliError {
    match error {
        AgentLoopError::InvalidBudget | AgentLoopError::InvalidTask => {
            CliError::usage(error.to_string())
        }
        AgentLoopError::InvalidSkillContext
        | AgentLoopError::InvalidL1Evidence
        | AgentLoopError::Context(_) => CliError::execution(error.to_string(), json!({})),
        AgentLoopError::Provider(error) => CliError::provider(error.to_string(), json!({})),
        AgentLoopError::ProviderExecution { error, receipts } => CliError::provider(
            error.to_string(),
            json!({
                "provider_calls": receipts.len(),
                "receipts": receipts.iter().map(|receipt| {
                    let outcome = match receipt.outcome() {
                        pandora_types::EffectOutcome::Succeeded => json!({"status": "succeeded"}),
                        pandora_types::EffectOutcome::Failed { code } => {
                            json!({"status": "failed", "code": code})
                        }
                        pandora_types::EffectOutcome::Denied { reason } => {
                            json!({"status": "denied", "reason": reason})
                        }
                    };
                    json!({
                        "receipt_id": receipt.receipt_id().as_str(),
                        "outcome": outcome,
                    })
                }).collect::<Vec<_>>(),
            }),
        ),
        AgentLoopError::EmptyResponse
        | AgentLoopError::ToolBudgetExceeded
        | AgentLoopError::TurnBudgetExceeded
        | AgentLoopError::ControlledStop { .. } => {
            CliError::execution(error.to_string(), json!({}))
        }
        AgentLoopError::Execution(error) => runtime_error(error),
        AgentLoopError::ApprovalRequired { reason, .. } => CliError::approval(reason, json!({})),
    }
}

fn resume_session(
    store: &SessionStore,
    value: &str,
) -> Result<pandora_runtime::sessions::SessionSnapshot, CliError> {
    let session_id =
        SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))?;
    let (principal, tenant, workspace) = super::session_scope();
    store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn run_details(
    summary: &pandora_runtime::RunSummary,
    session_id: &SessionId,
    planning_model: Option<&str>,
    evaluation: Value,
) -> Value {
    add_planning(
        json!({
        "session_id": session_id,
        "execution_id": summary.execution_id(),
        "harness_id": summary.selected_harness(),
        "gene_id": summary.selected_gene(),
        "status": match summary.status() {
            RunStatus::Completed => "completed",
            RunStatus::Denied { .. } => "denied",
            RunStatus::ApprovalRequired { .. } => "approval_required",
            RunStatus::Failed { .. } => "failed",
        },
        "evaluation": evaluation,
        }),
        planning_model,
    )
}

fn plan_task(
    config: &RuntimeConfig,
    controller: &ExecutionController,
    session: &Session,
    request: &str,
    model_override: Option<&str>,
    provider_name: Option<&str>,
) -> Result<(String, Option<String>), CliError> {
    let model = model_override
        .or_else(|| {
            provider_name
                .and_then(|name| config.provider_profile(name).map(|profile| profile.model()))
        })
        .or(config.provider_model())
        .unwrap_or("default");
    let provider = configured_provider_for(config, model, "planning", provider_name)?;
    let manifest = provider.manifest().clone();
    let messages = vec![
        ChatMessage::system(
            "You are Pandora's planning layer. Return only JSON with one string field named task. "
                .to_owned()
                + "The task must use exactly one bounded format: read:<path>, search:<query>, "
                + "review:<path>, patch:<path>:<content>, or verify. Do not return shell commands, "
                + "credentials, additional fields, or paths outside the workspace.",
        )
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ChatMessage::user(request.to_owned())
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
    ];
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        messages,
    )
    .and_then(|request| request.with_max_output_tokens(256))
    .and_then(|request| request.with_timeout(Duration::from_secs(60)))
    .map(|request| {
        request.with_trace_metadata(TraceMetadata::new().with_session_id(session.id().clone()))
    })
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let response = controller
        .invoke_provider(provider.as_ref(), request, session, timestamp())
        .map_err(runtime_error)?
        .into_result()
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    if !response.tool_calls().is_empty() {
        return Err(CliError::provider(
            "planner returned tool calls; planning cannot execute tools",
            json!({}),
        ));
    }
    let validated = parse_and_validate(
        response.text(),
        &json!({
            "type": "object",
            "required": ["task"],
            "properties": {"task": {"type": "string"}},
            "additionalProperties": false,
        }),
        None,
        FallbackPolicy::Reject,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let planned = validated
        .value()
        .get("task")
        .and_then(Value::as_str)
        .filter(|task| !task.trim().is_empty())
        .ok_or_else(|| CliError::provider("planner returned an empty task", json!({})))?;
    if planned.len() > MAX_PLANNED_TASK_BYTES {
        return Err(CliError::provider(
            "planner task exceeds the size limit",
            json!({}),
        ));
    }
    Ok((
        planned.to_owned(),
        Some(manifest.default_model().as_str().to_owned()),
    ))
}

fn add_planning(mut details: Value, model: Option<&str>) -> Value {
    if let Some(model) = model
        && let Value::Object(object) = &mut details
    {
        object.insert(
            "planning".to_owned(),
            json!({"enabled": true, "model": model}),
        );
    }
    details
}

fn create_approval(
    store: &ApprovalStore,
    summary: &pandora_runtime::RunSummary,
    session_id: &SessionId,
    principal_id: &pandora_types::PrincipalId,
    task: &str,
) -> Result<pandora_runtime::PendingApproval, CliError> {
    let request_digest = summary
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            EventPayload::Effect { request_digest, .. } => Some(request_digest.clone()),
            _ => None,
        })
        .ok_or_else(|| CliError::internal("approval request has no effect digest", json!({})))?;
    let expires_at = timestamp().as_unix_seconds().saturating_add(900);
    let request = ApprovalRequest::new(
        format!("approval-{}", summary.execution_id()),
        session_id.clone(),
        summary.execution_id().clone(),
        principal_id.clone(),
        summary.selected_gene().clone(),
        request_digest,
        approval_summary(task),
        1,
        pandora_types::Timestamp::from_unix_seconds(expires_at),
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    store
        .create(request)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn approval_summary(task: &str) -> String {
    let mut parts = task.splitn(3, ':');
    let action = parts.next().unwrap_or("task");
    let path = parts.next().unwrap_or("workspace");
    format!("coding {action} operation for {path}")
}

fn add_detail(mut details: Value, key: &str, value: impl Into<Value>) -> Value {
    if let Value::Object(object) = &mut details {
        object.insert(key.to_owned(), value.into());
    }
    details
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn runtime_error(error: RuntimeError) -> CliError {
    match error {
        RuntimeError::InvalidIntent(message) => CliError::usage(message),
        RuntimeError::NoDefaultHarness => CliError::execution(
            "no default harness is available",
            json!({"hint": "use --agent for natural-language tasks or a coding action prefix"}),
        ),
        RuntimeError::Denied(reason) => CliError::policy(reason, json!({})),
        RuntimeError::ApprovalRequired(reason) => CliError::approval(reason, json!({})),
        RuntimeError::Approval(error) => approval_error(error),
        RuntimeError::ApprovalNotRequired => CliError::policy(
            "the supplied approval is not required by the active policy",
            json!({}),
        ),
        RuntimeError::UnsupportedHarness(_) => {
            CliError::execution("requested harness is not supported", json!({}))
        }
        RuntimeError::NonExecutableHarness { id, kind } => CliError::execution(
            "requested harness is not runnable",
            json!({"harness_id": id, "kind": kind.as_str()}),
        ),
        RuntimeError::UnknownGene => {
            CliError::execution("requested Gene is not available", json!({}))
        }
        RuntimeError::Planning(_) => CliError::execution("Gene planning failed", json!({})),
        RuntimeError::Authorization(_) => {
            CliError::execution("effect authorization failed", json!({}))
        }
        RuntimeError::Permit(_) => CliError::execution("effect permit failed", json!({})),
        RuntimeError::Provider(error) => CliError::provider(error.to_string(), json!({})),
        RuntimeError::Request(_) => CliError::execution("effect request was invalid", json!({})),
        RuntimeError::Filesystem(_) => {
            CliError::execution("filesystem execution failed", json!({}))
        }
        RuntimeError::Process(_) => CliError::execution("process execution failed", json!({})),
        RuntimeError::UnsupportedOperation(_) => {
            CliError::execution("requested operation is not supported", json!({}))
        }
    }
}

fn approval_error(error: pandora_runtime::ApprovalError) -> CliError {
    match error {
        pandora_runtime::ApprovalError::Expired | pandora_runtime::ApprovalError::Terminal => {
            CliError::approval(error.to_string(), json!({}))
        }
        pandora_runtime::ApprovalError::ScopeMismatch
        | pandora_runtime::ApprovalError::DigestMismatch => {
            CliError::policy(error.to_string(), json!({}))
        }
        other => CliError::internal(other.to_string(), json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_runtime::config::{ConfigOverrides, RuntimeConfig};
    use pandora_types::{ExecutionId, Timestamp};
    use std::collections::BTreeMap;

    #[test]
    fn optimization_selects_a_configured_provider_with_completed_evidence() {
        let root =
            std::env::temp_dir().join(format!("pandora-provider-selection-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture root should be created");
        let config_path = root.join("config.json");
        fs::write(
            &config_path,
            r#"{
                "providers": {
                    "slow": {
                        "base_url": "https://slow.example/v1",
                        "model": "slow-model",
                        "api_key_env": "PANDORA_SLOW_API_KEY"
                    },
                    "fast": {
                        "base_url": "https://fast.example/v1",
                        "model": "fast-model",
                        "api_key_env": "PANDORA_FAST_API_KEY"
                    }
                },
                "active_provider": "slow"
            }"#,
        )
        .expect("fixture config should be written");
        let config = RuntimeConfig::from_sources(
            &ConfigOverrides::default(),
            &BTreeMap::new(),
            &config_path,
            root.join("data"),
            root.join("workspace"),
        )
        .expect("fixture config should load");
        let store = EfficiencyStore::open(config.data_dir().join("efficiency.sqlite3"))
            .expect("efficiency store should open");
        let sample = EfficiencySample::new(
            ExecutionId::new("provider-selection-1").unwrap(),
            "coding",
            "fast/fast-model",
            100,
            40,
            20,
            12,
            true,
            Timestamp::from_unix_seconds(1),
        )
        .unwrap();
        store
            .record(&sample, DEFAULT_MAX_SAMPLES_PER_TARGET)
            .expect("efficiency sample should persist");

        let selected = select_provider(&config, "coding", EfficiencyObjective::LowestLatency)
            .expect("provider selection should succeed");
        assert_eq!(selected.as_deref(), Some("fast"));

        let _ = fs::remove_dir_all(root);
    }
}
