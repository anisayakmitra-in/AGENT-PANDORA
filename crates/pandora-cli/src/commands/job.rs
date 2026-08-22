use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{JobRecord, JobStore, JobStoreError};
use pandora_types::{JobCommand, JobId, JobRequest, JobStatus, JobWorkerId};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_DRAIN_JOBS: usize = 64;

struct CompletedJob {
    id: JobId,
    status: JobStatus,
    result: Value,
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("job requires a subcommand"))?;
    match subcommand.as_str() {
        "submit" => submit(&args[1..]),
        "work" => work(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "cancel" => cancel(&args[1..]),
        "mark-interrupted" => mark_interrupted(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown job command '{unknown}'"))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "job list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let jobs = store
        .list(&principal, &tenant, &workspace)
        .map_err(job_store_error)?
        .into_iter()
        .map(|job| job_json(&job))
        .collect::<Result<Vec<_>, _>>()?;
    let count = jobs.len();
    Ok(success(
        "job list",
        json!({"count": count, "jobs": jobs}),
        format!("{count} job(s)"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("job inspect requires exactly one job ID"));
    }
    let id = parse_job_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let job = store
        .inspect(&id, &principal, &tenant, &workspace)
        .map_err(job_store_error)?;
    Ok(success(
        "job inspect",
        job_json(&job)?,
        format!("{} is {}", job.id(), job.status().as_str()),
    ))
}

fn cancel(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("job cancel requires exactly one job ID"));
    }
    let id = parse_job_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let job = store
        .cancel(&id, &principal, &tenant, &workspace, timestamp())
        .map_err(job_store_error)?;
    Ok(success(
        "job cancel",
        job_json(&job)?,
        format!("Cancelled {}", job.id()),
    ))
}

fn mark_interrupted(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "reason", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "job mark-interrupted requires exactly one job ID",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "job mark-interrupted requires '--yes'; review external effects before resubmitting",
        ));
    }
    let reason = parsed
        .value("reason")
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| CliError::usage("job mark-interrupted requires a non-empty '--reason'"))?;
    let id = parse_job_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let job = store
        .mark_interrupted(&id, &principal, &tenant, &workspace, reason, timestamp())
        .map_err(job_store_error)?;
    Ok(success(
        "job mark-interrupted",
        job_json(&job)?,
        format!(
            "Marked {} interrupted; review external effects before resubmitting",
            job.id()
        ),
    ))
}

fn work(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "max-jobs"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "job work does not accept positional arguments",
        ));
    }
    let max_jobs = parse_max_jobs(parsed.value("max-jobs"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let worker_id = allocate_worker_id()?;
    if let Some(max_jobs) = max_jobs {
        return drain_jobs(
            &store, &principal, &tenant, &workspace, &worker_id, max_jobs,
        );
    }
    let Some(completed) = execute_one_job(&store, &principal, &tenant, &workspace, &worker_id)?
    else {
        return Ok(success(
            "job work",
            json!({"job": null, "status": "idle"}),
            "No queued jobs",
        ));
    };
    Ok(success(
        "job work",
        json!({
            "job_id": completed.id,
            "status": completed.status.as_str(),
            "result": completed.result,
        }),
        format!("Completed {}", completed.id),
    ))
}

fn parse_max_jobs(value: Option<&str>) -> Result<Option<usize>, CliError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let count = value.parse::<usize>().ok();
    if count.is_none_or(|count| !(1..=MAX_DRAIN_JOBS).contains(&count)) {
        return Err(CliError::usage(format!(
            "job work --max-jobs must be an integer from 1 to {MAX_DRAIN_JOBS}"
        )));
    }
    Ok(count)
}

fn drain_jobs(
    store: &JobStore,
    principal: &pandora_types::PrincipalId,
    tenant: &pandora_types::TenantId,
    workspace: &pandora_types::WorkspaceId,
    worker_id: &JobWorkerId,
    max_jobs: usize,
) -> Result<CommandResult, CliError> {
    let mut jobs = Vec::with_capacity(max_jobs);
    let mut stop_reason = "limit_reached";
    for _ in 0..max_jobs {
        match execute_one_job(store, principal, tenant, workspace, worker_id) {
            Ok(Some(completed)) => jobs.push(job_summary(&completed)),
            Ok(None) => {
                stop_reason = "queue_empty";
                break;
            }
            Err(mut error) => {
                add_drain_error_details(&mut error, jobs);
                return Err(error);
            }
        }
    }
    let processed_count = jobs.len();
    Ok(success(
        "job work",
        json!({
            "processed_count": processed_count,
            "stop_reason": stop_reason,
            "jobs": jobs,
        }),
        format!("Processed {processed_count} job(s)"),
    ))
}

fn job_summary(job: &CompletedJob) -> Value {
    json!({
        "job_id": job.id,
        "status": job.status.as_str(),
    })
}

fn execute_one_job(
    store: &JobStore,
    principal: &pandora_types::PrincipalId,
    tenant: &pandora_types::TenantId,
    workspace: &pandora_types::WorkspaceId,
    worker_id: &JobWorkerId,
) -> Result<Option<CompletedJob>, CliError> {
    let Some(job) = store
        .claim_next(principal, tenant, workspace, worker_id, timestamp())
        .map_err(job_store_error)?
    else {
        return Ok(None);
    };
    let result = match job.request().command() {
        JobCommand::Run => super::run::execute(job.request().arguments()),
    };
    match result {
        Ok(result) => {
            let result = crate::output::envelope(result);
            let finished = store
                .finish(
                    job.id(),
                    principal,
                    tenant,
                    workspace,
                    worker_id,
                    JobStatus::Completed,
                    &result,
                    timestamp(),
                )
                .map_err(job_store_error)?;
            Ok(Some(CompletedJob {
                id: finished.id().clone(),
                status: finished.status(),
                result,
            }))
        }
        Err(mut error) => {
            let status = if error.code == "approval_required" {
                JobStatus::ApprovalRequired
            } else {
                JobStatus::Failed
            };
            let result = error.envelope();
            store
                .finish(
                    job.id(),
                    principal,
                    tenant,
                    workspace,
                    worker_id,
                    status,
                    &result,
                    timestamp(),
                )
                .map_err(job_store_error)?;
            add_job_error_details(&mut error, job.id(), status);
            Err(error)
        }
    }
}

fn submit(args: &[String]) -> Result<CommandResult, CliError> {
    let separator = args
        .iter()
        .position(|argument| argument == "--")
        .ok_or_else(|| CliError::usage("job submit requires '--' before run arguments"))?;
    let parsed = parse_options(&args[..separator], &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "job submit accepts queue options only before '--'",
        ));
    }
    let requested_run_args = &args[separator + 1..];
    if requested_run_args.is_empty() {
        return Err(CliError::usage(
            "job submit requires run arguments after '--'",
        ));
    }
    if requested_run_args.iter().any(|argument| {
        ["config", "data-dir", "workspace"].iter().any(|name| {
            argument == &format!("--{name}") || argument.starts_with(&format!("--{name}="))
        })
    }) {
        return Err(CliError::usage(
            "run path options belong before the job submit '--' separator",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let run_args = normalized_run_args(&config, requested_run_args);
    let request = JobRequest::new(JobCommand::Run, run_args)
        .map_err(|error| CliError::usage(error.to_string()))?;
    let id = allocate_job_id()?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .submit(&id, &principal, &tenant, &workspace, &request, timestamp())
        .map_err(job_store_error)?;
    Ok(success(
        "job submit",
        json!({
            "job_id": record.id(),
            "status": record.status().as_str(),
            "created_at": record.created_at().as_unix_seconds(),
        }),
        format!("Queued {}", record.id()),
    ))
}

fn normalized_run_args(
    config: &pandora_runtime::config::RuntimeConfig,
    requested: &[String],
) -> Vec<String> {
    let mut arguments = vec![
        "--config".to_owned(),
        config.config_path().to_string_lossy().into_owned(),
        "--data-dir".to_owned(),
        config.data_dir().to_string_lossy().into_owned(),
        "--workspace".to_owned(),
        config.workspace_dir().to_string_lossy().into_owned(),
    ];
    arguments.extend_from_slice(requested);
    arguments
}

fn allocate_job_id() -> Result<JobId, CliError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    JobId::new(format!("job-{}-{nonce}", std::process::id()))
        .map_err(|_| CliError::internal("could not allocate a job ID", json!({})))
}

fn allocate_worker_id() -> Result<JobWorkerId, CliError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    JobWorkerId::new(format!("worker-{}-{nonce}", std::process::id()))
        .map_err(|_| CliError::internal("could not allocate a worker ID", json!({})))
}

fn parse_job_id(value: &str) -> Result<JobId, CliError> {
    JobId::new(value.to_owned()).map_err(|_| CliError::usage("job ID is invalid"))
}

fn job_json(job: &JobRecord) -> Result<Value, CliError> {
    let request = serde_json::to_value(job.request())
        .map_err(|_| CliError::internal("could not serialize job request", json!({})))?;
    Ok(json!({
        "job_id": job.id(),
        "principal_id": job.principal_id(),
        "tenant_id": job.tenant_id(),
        "workspace_id": job.workspace_id(),
        "request": request,
        "status": job.status().as_str(),
        "created_at": job.created_at().as_unix_seconds(),
        "started_at": job.started_at().map(|value| value.as_unix_seconds()),
        "finished_at": job.finished_at().map(|value| value.as_unix_seconds()),
        "worker_id": job.worker_id().map(JobWorkerId::as_str),
        "result": job.result(),
    }))
}

fn job_store_error(error: JobStoreError) -> CliError {
    let message = error.to_string();
    match error {
        JobStoreError::Contract(_) => CliError::usage(message),
        JobStoreError::JobNotFound
        | JobStoreError::JobOwnedByAnotherWorker
        | JobStoreError::InvalidTransition { .. }
        | JobStoreError::ResultTooLarge => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}

fn add_job_error_details(error: &mut CliError, id: &JobId, status: JobStatus) {
    let mut details = match std::mem::replace(&mut error.details, Value::Null) {
        Value::Object(details) => details,
        value => {
            let mut details = Map::new();
            details.insert("run_details".to_owned(), value);
            details
        }
    };
    details.insert("job_id".to_owned(), Value::String(id.as_str().to_owned()));
    details.insert(
        "job_status".to_owned(),
        Value::String(status.as_str().to_owned()),
    );
    error.details = Value::Object(details);
}

fn add_drain_error_details(error: &mut CliError, mut jobs: Vec<Value>) {
    let current_job = error
        .details
        .get("job_id")
        .and_then(Value::as_str)
        .zip(error.details.get("job_status").and_then(Value::as_str))
        .map(|(job_id, status)| {
            json!({
                "job_id": job_id,
                "status": status,
            })
        });
    if let Some(current_job) = current_job {
        jobs.push(current_job);
    }
    let processed_count = jobs.len();
    let mut details = match std::mem::replace(&mut error.details, Value::Null) {
        Value::Object(details) => details,
        value => {
            let mut details = Map::new();
            details.insert("run_details".to_owned(), value);
            details
        }
    };
    details.insert("processed_count".to_owned(), json!(processed_count));
    details.insert(
        "stop_reason".to_owned(),
        Value::String(error.code.to_owned()),
    );
    details.insert("processed_jobs".to_owned(), Value::Array(jobs));
    error.details = Value::Object(details);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_runtime::JobStore;
    use pandora_types::{JobCommand, JobRequest, JobWorkerId};
    use std::fs;

    #[test]
    fn submit_requires_a_separator_before_run_arguments() {
        let error = match execute(&["submit".to_owned(), "guide".to_owned()]) {
            Ok(_) => panic!("submission without a separator should fail"),
            Err(error) => error,
        };

        assert_eq!(
            error.message,
            "job submit requires '--' before run arguments"
        );
    }

    #[test]
    fn submit_persists_a_scoped_run_request_with_explicit_paths() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-submit-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();

        let result = execute(&[
            "submit".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--".to_owned(),
            "guide".to_owned(),
        ])
        .unwrap();

        assert_eq!(result.command, "job submit");
        assert_eq!(result.data["status"], "queued");
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let jobs = store.list(&principal, &tenant, &workspace_id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].request().arguments(),
            &[
                "--config".to_owned(),
                config_path.to_string_lossy().into_owned(),
                "--data-dir".to_owned(),
                data_dir.to_string_lossy().into_owned(),
                "--workspace".to_owned(),
                workspace.to_string_lossy().into_owned(),
                "guide".to_owned(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn work_executes_one_queued_request_through_the_run_path() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-work-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        execute(&[
            "submit".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--".to_owned(),
            "guide".to_owned(),
        ])
        .unwrap();

        let result = execute(&[
            "work".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert_eq!(result.command, "job work");
        assert_eq!(result.data["status"], "completed");
        assert_eq!(result.data["result"]["command"], "run");
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let jobs = store.list(&principal, &tenant, &workspace_id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status(), pandora_types::JobStatus::Completed);
        assert_eq!(jobs[0].result().unwrap()["command"], "run");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn work_drains_the_requested_number_of_jobs_in_fifo_order() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-drain-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        let submit_args = [
            "submit".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--".to_owned(),
            "guide".to_owned(),
        ];
        let first = execute(&submit_args).unwrap().data["job_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second = execute(&submit_args).unwrap().data["job_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let result = execute(&[
            "work".to_owned(),
            "--max-jobs".to_owned(),
            "2".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert_eq!(result.command, "job work");
        assert_eq!(result.data["processed_count"], 2);
        assert_eq!(result.data["stop_reason"], "limit_reached");
        assert_eq!(result.data["jobs"][0]["job_id"], first);
        assert_eq!(result.data["jobs"][0]["status"], "completed");
        assert_eq!(result.data["jobs"][1]["job_id"], second);
        assert_eq!(result.data["jobs"][1]["status"], "completed");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn work_rejects_job_counts_outside_the_bounded_range() {
        for value in ["0", "65"] {
            let result = execute(&["work".to_owned(), "--max-jobs".to_owned(), value.to_owned()]);
            let error = match result {
                Ok(_) => panic!("out-of-range job counts should fail"),
                Err(error) => error,
            };

            assert_eq!(
                error.message,
                "job work --max-jobs must be an integer from 1 to 64"
            );
        }
    }

    #[test]
    fn drain_reports_an_empty_queue_without_processing() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-drain-empty-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();

        let result = execute(&[
            "work".to_owned(),
            "--max-jobs".to_owned(),
            "3".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert_eq!(result.data["processed_count"], 0);
        assert_eq!(result.data["stop_reason"], "queue_empty");
        assert_eq!(result.data["jobs"], json!([]));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_inspect_and_cancel_expose_the_scoped_queue() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-control-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        let submit_args = [
            "submit".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--".to_owned(),
            "guide".to_owned(),
        ];
        let submitted = execute(&submit_args).unwrap();
        let job_id = submitted.data["job_id"].as_str().unwrap().to_owned();
        let queue_args = [
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ];

        let mut list_args = vec!["list".to_owned()];
        list_args.extend_from_slice(&queue_args);
        let listed = execute(&list_args).unwrap();
        assert_eq!(listed.data["count"], 1);
        assert_eq!(listed.data["jobs"][0]["job_id"], job_id);

        let mut inspect_args = vec!["inspect".to_owned(), job_id.clone()];
        inspect_args.extend_from_slice(&queue_args);
        let inspected = execute(&inspect_args).unwrap();
        assert_eq!(inspected.data["status"], "queued");
        assert_eq!(inspected.data["request"]["command"], "run");

        let mut cancel_args = vec!["cancel".to_owned(), job_id.clone()];
        cancel_args.extend_from_slice(&queue_args);
        let cancelled = execute(&cancel_args).unwrap();
        assert_eq!(cancelled.data["status"], "cancelled");
        let inspected = execute(&inspect_args).unwrap();
        assert_eq!(inspected.data["status"], "cancelled");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mark_interrupted_records_an_unknown_outcome_without_requeueing() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-interrupt-command-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let id = JobId::new("job-interrupt-command").unwrap();
        let request = JobRequest::new(JobCommand::Run, vec!["guide".to_owned()]).unwrap();
        let worker = JobWorkerId::new("worker-command").unwrap();
        store
            .submit(
                &id,
                &principal,
                &tenant,
                &workspace_id,
                &request,
                super::super::timestamp(),
            )
            .unwrap();
        store
            .claim_next(
                &principal,
                &tenant,
                &workspace_id,
                &worker,
                super::super::timestamp(),
            )
            .unwrap();

        let result = execute(&[
            "mark-interrupted".to_owned(),
            id.as_str().to_owned(),
            "--reason".to_owned(),
            "worker exited before reporting an outcome".to_owned(),
            "--yes".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert_eq!(result.data["status"], "interrupted");
        assert_eq!(result.data["worker_id"], "worker-command");
        assert_eq!(result.data["result"]["code"], "worker_interrupted");
        assert_eq!(result.data["result"]["outcome_known"], false);
        assert_eq!(
            store
                .inspect(&id, &principal, &tenant, &workspace_id)
                .unwrap()
                .status(),
            JobStatus::Interrupted
        );
        assert!(
            store
                .claim_next(
                    &principal,
                    &tenant,
                    &workspace_id,
                    &worker,
                    super::super::timestamp(),
                )
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mark_interrupted_requires_explicit_confirmation() {
        let error = match execute(&[
            "mark-interrupted".to_owned(),
            "job-confirmation".to_owned(),
            "--reason".to_owned(),
            "operator reviewed the job".to_owned(),
        ]) {
            Ok(_) => panic!("marking a job interrupted requires confirmation"),
            Err(error) => error,
        };

        assert_eq!(error.code, "usage_error");
        assert_eq!(
            error.message,
            "job mark-interrupted requires '--yes'; review external effects before resubmitting"
        );
    }

    #[test]
    fn work_preserves_the_existing_approval_boundary() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-approval-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        fs::write(workspace.join("README.md"), b"unchanged").unwrap();
        execute(&[
            "submit".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--".to_owned(),
            "patch:README.md:changed".to_owned(),
        ])
        .unwrap();

        let result = execute(&[
            "work".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ]);
        let error = match result {
            Ok(_) => panic!("write job should require approval"),
            Err(error) => error,
        };

        assert_eq!(error.code, "approval_required");
        assert_eq!(error.details["job_status"], "approval_required");
        assert_eq!(fs::read(workspace.join("README.md")).unwrap(), b"unchanged");
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let jobs = store.list(&principal, &tenant, &workspace_id).unwrap();
        assert_eq!(jobs[0].status(), pandora_types::JobStatus::ApprovalRequired);
        assert_eq!(jobs[0].result().unwrap()["code"], "approval_required");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn drain_stops_at_approval_and_leaves_later_jobs_queued() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-drain-approval-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        fs::write(workspace.join("README.md"), b"unchanged").unwrap();
        let queue_options = [
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ];
        for task in ["guide", "patch:README.md:changed", "guide"] {
            let mut args = vec!["submit".to_owned()];
            args.extend_from_slice(&queue_options);
            args.extend(["--".to_owned(), task.to_owned()]);
            execute(&args).unwrap();
        }
        let mut work_args = vec!["work".to_owned(), "--max-jobs".to_owned(), "3".to_owned()];
        work_args.extend_from_slice(&queue_options);

        let error = match execute(&work_args) {
            Ok(_) => panic!("drain should stop when a job requires approval"),
            Err(error) => error,
        };

        assert_eq!(error.code, "approval_required");
        assert_eq!(error.details["processed_count"], 2);
        assert_eq!(error.details["stop_reason"], "approval_required");
        assert_eq!(error.details["processed_jobs"][0]["status"], "completed");
        assert_eq!(
            error.details["processed_jobs"][1]["status"],
            "approval_required"
        );
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let jobs = store.list(&principal, &tenant, &workspace_id).unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].status(), JobStatus::Queued);
        assert_eq!(jobs[1].status(), JobStatus::ApprovalRequired);
        assert_eq!(jobs[2].status(), JobStatus::Completed);
        assert_eq!(fs::read(workspace.join("README.md")).unwrap(), b"unchanged");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn drain_stops_at_execution_failure() {
        let root = std::env::temp_dir().join(format!(
            "pandora-job-drain-failure-{}-{}",
            std::process::id(),
            super::super::timestamp().as_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        let config_path = root.join("config.json");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(&config_path, b"{}").unwrap();
        let queue_options = [
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ];
        for task in ["guide", "summarize the workspace", "guide"] {
            let mut args = vec!["submit".to_owned()];
            args.extend_from_slice(&queue_options);
            args.extend(["--".to_owned(), task.to_owned()]);
            execute(&args).unwrap();
        }
        let mut work_args = vec!["work".to_owned(), "--max-jobs".to_owned(), "3".to_owned()];
        work_args.extend_from_slice(&queue_options);

        let error = match execute(&work_args) {
            Ok(_) => panic!("drain should stop when a job fails"),
            Err(error) => error,
        };

        assert_eq!(error.code, "execution_failed");
        assert_eq!(error.details["processed_count"], 2);
        assert_eq!(error.details["stop_reason"], "execution_failed");
        assert_eq!(error.details["processed_jobs"][0]["status"], "completed");
        assert_eq!(error.details["processed_jobs"][1]["status"], "failed");
        let store = JobStore::open(data_dir.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace_id) = super::super::session_scope();
        let jobs = store.list(&principal, &tenant, &workspace_id).unwrap();
        assert_eq!(jobs[0].status(), JobStatus::Queued);
        assert_eq!(jobs[1].status(), JobStatus::Failed);
        assert_eq!(jobs[2].status(), JobStatus::Completed);
        let _ = fs::remove_dir_all(root);
    }
}
