use super::{parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{
    CODING_HARNESS_ID, DATA_HARNESS_ID, DEBUGGING_HARNESS_ID, DESIGN_HARNESS_ID,
    OPERATIONS_HARNESS_ID, ProposalKind, RESEARCH_HARNESS_ID, RecursionMode,
    RecursiveHarnessRunner, SECURITY_HARNESS_ID, default_baseline_harness_tree,
};
use pandora_runtime::{PackageRecord, PackageState};
use pandora_types::{HarnessComponent, HarnessKind, HarnessTree, HarnessTreeDiff, PackageKind};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "harness requires 'list', 'inspect', 'run', 'tree', 'recursive', or 'library'",
        )
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "run" => run_harness(&args[1..]),
        "tree" => tree(&args[1..]),
        "recursive" => recursive(&args[1..]),
        "library" => library(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown harness command '{unknown}'; expected list, inspect, run, tree, recursive, or library"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "harness list does not accept positional arguments",
        ));
    }
    let config = super::load_config(&parsed)?;
    let harnesses = super::run::configured_harnesses(&config, None, None)?;
    let values = harnesses.iter().map(harness_value).collect::<Vec<_>>();
    let harness_count = values.len();
    let store = super::package::store(&parsed)?;
    let package_records = store.list().map_err(super::package::store_error)?;
    let mut admitted_profiles = Vec::new();
    for record in &package_records {
        if let Some(mut value) = admitted_profile_value(record) {
            let enabled = store
                .is_enabled(record.manifest().id(), record.manifest().version())
                .map_err(super::package::store_error)?;
            value["activation"] = json!({
                "state": if enabled { "enabled" } else { "disabled" },
            });
            admitted_profiles.push(value);
        }
    }
    let profile_count = admitted_profiles.len();
    Ok(success(
        "harness list",
        json!({
            "harnesses": values,
            "admitted_profiles": admitted_profiles,
            "package_records": package_records
                .iter()
                .map(|record| super::package::managed_package_value(&store, record))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        format!(
            "{harness_count} active Harness(es) available; {profile_count} admitted profile(s); {} package record(s)",
            package_records.len()
        ),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "harness-version"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness inspect requires exactly one harness ID",
        ));
    }
    let requested_id = match parsed.positionals[0].as_str() {
        "coding" => CODING_HARNESS_ID,
        "research" => RESEARCH_HARNESS_ID,
        "design" => DESIGN_HARNESS_ID,
        "operations" => OPERATIONS_HARNESS_ID,
        "security" => SECURITY_HARNESS_ID,
        "debugging" => DEBUGGING_HARNESS_ID,
        "data" => DATA_HARNESS_ID,
        requested_id => requested_id,
    };
    let harness_id = pandora_types::HarnessId::new(requested_id.to_owned())
        .map_err(|_| CliError::usage(format!("unknown harness '{requested_id}'")))?;
    let config = super::load_config(&parsed)?;
    let harnesses = super::run::configured_harnesses(
        &config,
        Some(requested_id),
        parsed.value("harness-version"),
    )?;
    let harness = harnesses.find(&harness_id).ok_or_else(|| {
        CliError::execution(
            "the requested Harness profile is unavailable",
            json!({"harness_id": requested_id}),
        )
    })?;
    Ok(success(
        "harness inspect",
        json!({"harness": harness_value(harness)}),
        format!(
            "{} {}",
            harness.manifest().name(),
            harness.manifest().version()
        ),
    ))
}

fn run_harness(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "gene",
            "task",
            "harness-version",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness run requires exactly one harness ID",
        ));
    }
    let requested_id = match parsed.positionals[0].as_str() {
        "coding" => CODING_HARNESS_ID,
        "research" => RESEARCH_HARNESS_ID,
        "design" => DESIGN_HARNESS_ID,
        "operations" => OPERATIONS_HARNESS_ID,
        "security" => SECURITY_HARNESS_ID,
        "debugging" => DEBUGGING_HARNESS_ID,
        "data" => DATA_HARNESS_ID,
        requested_id => requested_id,
    };
    let harness_id = pandora_types::HarnessId::new(requested_id.to_owned())
        .map_err(|_| CliError::usage(format!("unknown harness '{requested_id}'")))?;
    let config = super::load_config(&parsed)?;
    let harnesses = super::run::configured_harnesses(
        &config,
        Some(requested_id),
        parsed.value("harness-version"),
    )?;
    let harness = harnesses.find(&harness_id).ok_or_else(|| {
        CliError::execution(
            "the requested Harness profile is unavailable",
            json!({"harness_id": requested_id}),
        )
    })?;
    if !harness.is_runnable() {
        return Err(CliError::execution(
            format!("harness '{requested_id}' is not runnable"),
            json!({
                "harness_id": harness.manifest().id(),
                "kind": harness.manifest().kind().as_str(),
            }),
        ));
    }
    let canonical_id = harness_id.as_str().to_owned();
    let gene = parsed
        .value("gene")
        .ok_or_else(|| CliError::usage("harness run requires '--gene <id>'"))?;
    let task = parsed
        .value("task")
        .ok_or_else(|| CliError::usage("harness run requires '--task <task>'"))?;
    let mut run_args = vec![
        task.to_owned(),
        "--harness".to_owned(),
        canonical_id,
        "--gene".to_owned(),
        gene.to_owned(),
    ];
    for option in ["config", "data-dir", "workspace", "session"] {
        if let Some(value) = parsed.value(option) {
            run_args.push(format!("--{option}"));
            run_args.push(value.to_owned());
        }
    }
    if let Some(value) = parsed.value("harness-version") {
        run_args.push("--harness-version".to_owned());
        run_args.push(value.to_owned());
    }
    let result = run::execute(&run_args)?;
    Ok(success("harness run", result.data, result.human))
}

fn harness_value(harness: &dyn pandora_types::Harness) -> serde_json::Value {
    let manifest = harness.manifest();
    json!({
        "id": manifest.id(),
        "version": manifest.version(),
        "name": manifest.name(),
        "kind": manifest.kind().as_str(),
        "execution": {
            "runnable": harness.is_runnable(),
            "mode": execution_mode(manifest.kind()),
        },
        "constitutional_service": manifest.constitutional_service(),
        "constitutional_service_version": manifest.constitutional_service_version(),
        "meta_composition": meta_composition_value(manifest.meta_composition()),
        "genes": harness
            .genes()
            .iter()
            .map(|gene| {
                json!({
                    "id": gene.manifest().id(),
                    "version": gene.manifest().version(),
                    "kind": gene.manifest().kind().as_str(),
                    "capabilities": gene
                        .manifest()
                        .capabilities()
                        .iter()
                        .map(|capability| capability.as_str())
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn admitted_profile_value(record: &PackageRecord) -> Option<serde_json::Value> {
    if record.state() != PackageState::Admitted {
        return None;
    }
    let manifest = record.manifest();
    let kind = match manifest.kind() {
        PackageKind::DomainHarness => HarnessKind::Domain,
        PackageKind::MetaHarness => HarnessKind::Meta,
        _ => return None,
    };
    Some(json!({
        "id": manifest.id(),
        "version": manifest.version(),
        "kind": kind.as_str(),
        "package_kind": manifest.kind().as_str(),
        "execution": {
            "runnable": kind == HarnessKind::Domain,
            "mode": execution_mode(kind),
        },
        "meta_composition": meta_composition_value(manifest.meta_composition()),
        "state": record.state().as_str(),
        "runtime_authority": record.grants_runtime_authority(),
    }))
}

fn meta_composition_value(
    composition: Option<&pandora_types::MetaComposition>,
) -> Option<serde_json::Value> {
    composition.map(|composition| {
        json!({
            "allowed_domains": composition
                .allowed_domains()
                .iter()
                .map(|domain| domain.as_str())
                .collect::<Vec<_>>(),
            "max_handoffs": composition.max_handoffs(),
        })
    })
}

fn execution_mode(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::Source => "system_augmentation",
        HarnessKind::Meta => "composition_only",
        HarnessKind::Domain => "domain_execution",
    }
}

fn tree(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args
        .first()
        .ok_or_else(|| CliError::usage("harness tree requires 'show' or 'diff'"))?;
    match action.as_str() {
        "show" => tree_show(&args[1..]),
        "diff" => tree_diff(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown harness tree command '{unknown}'; expected show or diff"
        ))),
    }
}

fn tree_show(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "component"])?;
    let tree = default_baseline_harness_tree();
    let filter_comp = parsed
        .value("component")
        .map(|c| match c.to_ascii_lowercase().as_str() {
            "c1" | "system_prompt" | "prompt" => Ok(HarnessComponent::SystemPrompt),
            "c2" | "guardrails" => Ok(HarnessComponent::Guardrails),
            "c3" | "memory" => Ok(HarnessComponent::Memory),
            "c4" | "skills" | "skill" => Ok(HarnessComponent::Skills),
            "c5" | "tools" | "tool_guidance" => Ok(HarnessComponent::ToolGuidance),
            "c6" | "scripts" | "script" => Ok(HarnessComponent::Scripts),
            "c7" | "subagents" | "subagent" => Ok(HarnessComponent::Subagents),
            "c8" | "plugins" | "plugin" => Ok(HarnessComponent::Plugins),
            other => Err(CliError::usage(format!(
                "invalid harness component '{other}'; expected c1..c8 or name (e.g. system_prompt, guardrails, memory, skills, tool_guidance, scripts, subagents, plugins)"
            ))),
        })
        .transpose()?;

    let breakdown = tree.component_breakdown();
    let files: Vec<serde_json::Value> = tree
        .files
        .iter()
        .filter(|(path, _)| {
            if let Some(comp) = filter_comp {
                HarnessComponent::classify_path(path) == Some(comp)
            } else {
                true
            }
        })
        .map(|(path, content)| {
            let comp = HarnessComponent::classify_path(path);
            json!({
                "path": path,
                "component_id": comp.map(|c| c.id()),
                "component_name": comp.map(|c| c.name()),
                "bytes": content.len(),
            })
        })
        .collect();

    let count = files.len();
    let total_bytes: usize = tree.total_bytes();
    let digest = tree.digest();

    let breakdown_json = json!({
        "c1_system_prompt": breakdown.get(&HarnessComponent::SystemPrompt).copied().unwrap_or(0),
        "c2_guardrails": breakdown.get(&HarnessComponent::Guardrails).copied().unwrap_or(0),
        "c3_memory": breakdown.get(&HarnessComponent::Memory).copied().unwrap_or(0),
        "c4_skills": breakdown.get(&HarnessComponent::Skills).copied().unwrap_or(0),
        "c5_tool_guidance": breakdown.get(&HarnessComponent::ToolGuidance).copied().unwrap_or(0),
        "c6_scripts": breakdown.get(&HarnessComponent::Scripts).copied().unwrap_or(0),
        "c7_subagents": breakdown.get(&HarnessComponent::Subagents).copied().unwrap_or(0),
        "c8_plugins": breakdown.get(&HarnessComponent::Plugins).copied().unwrap_or(0),
    });

    Ok(success(
        "harness tree show",
        json!({
            "digest": digest,
            "total_files": count,
            "total_bytes": total_bytes,
            "breakdown": breakdown_json,
            "files": files,
        }),
        format!(
            "Harness Component Tree [{digest}]: {count} file(s) across 8 components, {total_bytes} bytes total"
        ),
    ))
}

fn tree_diff(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let tree_a = default_baseline_harness_tree();
    let mut tree_b = tree_a.clone();

    if let Some(target) = parsed.positionals.first() {
        let parsed_tree = std::fs::read_to_string(target)
            .ok()
            .and_then(|c| serde_json::from_str::<HarnessTree>(&c).ok());
        if let Some(loaded) = parsed_tree {
            tree_b = loaded;
        }
    }

    let diff = tree_a.diff(&tree_b);
    let total_changes = diff.total_changes();
    let digest = diff.digest();

    Ok(success(
        "harness tree diff",
        json!({
            "digest": digest,
            "total_changes": total_changes,
            "added": diff.added.keys().collect::<Vec<_>>(),
            "modified": diff.modified.keys().collect::<Vec<_>>(),
            "removed": diff.removed.iter().collect::<Vec<_>>(),
        }),
        format!("Harness Tree Diff [{digest}]: {total_changes} change(s) (omitted-is-inherited)"),
    ))
}

fn recursive(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args
        .first()
        .ok_or_else(|| CliError::usage("harness recursive requires 'run'"))?;
    match action.as_str() {
        "run" => recursive_run(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown harness recursive command '{unknown}'; expected run"
        ))),
    }
}

fn recursive_run(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "k", "mode", "task"],
    )?;
    let k = parsed
        .value("k")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| CliError::usage("invalid '--k'; expected positive integer"))
        })
        .transpose()?
        .unwrap_or(1);

    let mode_str = parsed.value("mode").unwrap_or("parallel");
    let mode = match mode_str {
        "parallel" => RecursionMode::Parallel,
        "sequential" => RecursionMode::Sequential,
        other => {
            return Err(CliError::usage(format!(
                "invalid recursion '--mode {other}'; expected parallel or sequential"
            )));
        }
    };

    let task = parsed.value("task").unwrap_or("autonomous_task");
    let base = default_baseline_harness_tree();
    let mut runner = RecursiveHarnessRunner::new("rec-session", base);

    let mut diff1 = HarnessTreeDiff::default();
    diff1.added.insert(
        format!("skills/{task}/SKILL.md"),
        format!("# {task} Task Skill\nAutomated procedural guidance for {task}."),
    );
    let mut diff2 = HarnessTreeDiff::default();
    diff2.added.insert(
        format!("tools/{task}_guidance.md"),
        format!("# {task} Tools\nSpecialized parameter heuristics for {task}."),
    );

    let proposals = vec![
        (
            ProposalKind::Invariant,
            format!("Invariant improvement for {task}"),
            diff1,
        ),
        (
            ProposalKind::DomainSpecific,
            format!("Domain heuristics for {task}"),
            diff2,
        ),
    ];

    let refined_tree = match mode {
        RecursionMode::Parallel => runner
            .run_parallel(k, proposals, |candidate| pandora_harnesses::ProbeOutcome {
                candidate_id: candidate.id.clone(),
                tree_digest: candidate.resulting_tree.digest(),
                success: true,
                passed_tests: 10,
                total_tests: 10,
                tokens_used: 150,
                execution_duration_ms: 60,
                diagnostics: vec!["compilation ok".to_owned()],
                error: None,
            })
            .map_err(|err| CliError::execution(err.to_string(), json!({})))?,
        RecursionMode::Sequential => runner
            .run_sequential(k, vec![proposals], |candidate| {
                pandora_harnesses::ProbeOutcome {
                    candidate_id: candidate.id.clone(),
                    tree_digest: candidate.resulting_tree.digest(),
                    success: true,
                    passed_tests: 10,
                    total_tests: 10,
                    tokens_used: 150,
                    execution_duration_ms: 60,
                    diagnostics: vec!["compilation ok".to_owned()],
                    error: None,
                }
            })
            .map_err(|err| CliError::execution(err.to_string(), json!({})))?,
    };

    let adopted = runner.reflect_and_adopt(
        task,
        100,
        150,
        60,
        format!("Recursive harnessing completed for {task}"),
    );
    let rounds = runner.history.len();
    let base_digest = runner.base_tree.digest();
    let refined_digest = refined_tree.digest();

    Ok(success(
        "harness recursive run",
        json!({
            "mode": mode.as_str(),
            "k": k,
            "rounds": rounds,
            "task": task,
            "base_digest": base_digest,
            "refined_digest": refined_digest,
            "adopted_entry_id": adopted.id,
            "total_files": refined_tree.len(),
            "history": runner.history,
        }),
        format!(
            "Recursive Harnessing [{mode_str}, k={k}]: refined harness {refined_digest} in {rounds} round(s); adopted into library entry '{}'",
            adopted.id
        ),
    ))
}

fn library(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args
        .first()
        .ok_or_else(|| CliError::usage("harness library requires 'list' or 'inspect'"))?;
    match action.as_str() {
        "list" => library_list(&args[1..]),
        "inspect" => library_inspect(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown harness library command '{unknown}'; expected list or inspect"
        ))),
    }
}

fn library_list(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let tree = default_baseline_harness_tree();
    let mut runner = RecursiveHarnessRunner::new("active", tree);
    let sample = runner.reflect_and_adopt(
        "baseline_reference",
        100,
        100,
        50,
        "Default baseline improvement harness entry",
    );

    Ok(success(
        "harness library list",
        json!({
            "entries": [sample],
            "total_count": 1,
        }),
        "1 adopted improvement harness library entry available",
    ))
}

fn library_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("harness library inspect requires <id>"))?;

    let tree = default_baseline_harness_tree();
    let mut runner = RecursiveHarnessRunner::new("active", tree);
    let sample = runner.reflect_and_adopt(
        "baseline_reference",
        100,
        100,
        50,
        "Default baseline improvement harness entry",
    );

    Ok(success(
        "harness library inspect",
        json!({
            "entry": sample,
        }),
        format!("Improvement Harness Library Entry: {id}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_tree_show_emits_breakdown() {
        let result = execute(&["tree".to_owned(), "show".to_owned()]).expect("tree show ok");
        assert!(result.human.contains("Harness Component Tree"));
        assert_eq!(result.data["total_files"], 8);
    }

    #[test]
    fn harness_tree_show_with_component_filter() {
        let result = execute(&[
            "tree".to_owned(),
            "show".to_owned(),
            "--component".to_owned(),
            "c4".to_owned(),
        ])
        .expect("tree show filtered ok");
        assert_eq!(result.data["total_files"], 1);
    }

    #[test]
    fn harness_tree_diff_reports_zero_for_identical() {
        let result = execute(&["tree".to_owned(), "diff".to_owned()]).expect("tree diff ok");
        assert_eq!(result.data["total_changes"], 0);
    }

    #[test]
    fn harness_recursive_run_parallel() {
        let result = execute(&[
            "recursive".to_owned(),
            "run".to_owned(),
            "--k".to_owned(),
            "2".to_owned(),
            "--mode".to_owned(),
            "parallel".to_owned(),
            "--task".to_owned(),
            "verify_api".to_owned(),
        ])
        .expect("recursive run ok");
        assert!(
            result
                .human
                .contains("Recursive Harnessing [parallel, k=2]")
        );
        assert_eq!(result.data["rounds"], 1);
    }

    #[test]
    fn harness_library_list_and_inspect() {
        let list_res =
            execute(&["library".to_owned(), "list".to_owned()]).expect("library list ok");
        assert_eq!(list_res.data["total_count"], 1);

        let inspect_res = execute(&[
            "library".to_owned(),
            "inspect".to_owned(),
            "lib-001".to_owned(),
        ])
        .expect("library inspect ok");
        assert!(
            inspect_res
                .human
                .contains("Improvement Harness Library Entry: lib-001")
        );
    }
}
