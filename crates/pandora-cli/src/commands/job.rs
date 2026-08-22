use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{JobRecord, JobStore, JobStoreError};
use pandora_types::{JobCommand, JobId, JobRequest, JobStatus};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn work(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "job work does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = JobStore::open(config.data_dir().join("jobs.sqlite3")).map_err(job_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let Some(job) = store
        .claim_next(&principal, &tenant, &workspace, timestamp())
        .map_err(job_store_error)?
    else {
        return Ok(success(
            "job work",
            json!({"job": null, "status": "idle"}),
            "No queued jobs",
        ));
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
                    &principal,
                    &tenant,
                    &workspace,
                    JobStatus::Completed,
                    &result,
                    timestamp(),
                )
                .map_err(job_store_error)?;
            Ok(success(
                "job work",
                json!({
                    "job_id": finished.id(),
                    "status": finished.status().as_str(),
                    "result": result,
                }),
                format!("Completed {}", finished.id()),
            ))
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
                    &principal,
                    &tenant,
                    &workspace,
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
        "result": job.result(),
    }))
}

fn job_store_error(error: JobStoreError) -> CliError {
    let message = error.to_string();
    match error {
        JobStoreError::Contract(_) => CliError::usage(message),
        JobStoreError::JobNotFound
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

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_runtime::JobStore;
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
}
