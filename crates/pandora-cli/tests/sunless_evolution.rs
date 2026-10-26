#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pandora-sunless-test-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER_URL")
            .env_remove("PANDORA_PROVIDER")
            .env_remove("GROQ_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("OLLAMA_API_KEY");
        cmd.output().expect("Pandora command should start")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn parse_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "Command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    serde_json::from_str(&stdout).expect("stdout should be valid JSON")
}

#[test]
fn sunless_dsrs_commands_execute_cleanly() {
    let fixture = Fixture::new();

    // 1. dsrs show
    let show = parse_json(fixture.command(&["dsrs", "show", "--json"]));
    assert_eq!(show["command"], "dsrs show");
    assert!(show["signatures"].as_array().unwrap().len() >= 3);

    // 2. dsrs inspect CodeReviewer
    let inspect = parse_json(fixture.command(&["dsrs", "inspect", "CodeReviewer", "--json"]));
    assert_eq!(inspect["command"], "dsrs inspect");
    assert_eq!(inspect["signature"], "CodeReviewer");
    assert!(inspect["fields"].as_array().unwrap().len() >= 2);

    // 3. dsrs cot SqlSynthesizer
    let cot = parse_json(fixture.command(&["dsrs", "cot", "SqlSynthesizer", "--json"]));
    assert_eq!(cot["command"], "dsrs cot");
    assert_eq!(cot["signature"], "SqlSynthesizer");
    assert_eq!(cot["has_rationale"], true);
}

#[test]
fn sunless_gepa_optimizer_commands_execute_cleanly() {
    let fixture = Fixture::new();

    // 1. gepa frontier
    let frontier = parse_json(fixture.command(&["gepa", "frontier", "--json"]));
    assert_eq!(frontier["command"], "gepa frontier");
    assert_eq!(frontier["count"], 3);
    assert_eq!(frontier["frontier"].as_array().unwrap().len(), 3);

    // 2. gepa optimize
    let optimize = parse_json(fixture.command(&["gepa", "optimize", "--json"]));
    assert_eq!(optimize["command"], "gepa optimize");
    assert!(optimize["admitted_id"].is_string());
    assert!(optimize["instructions"].is_string());
}

#[test]
fn sunless_evolution_ladder_genome_routing_and_depth() {
    let fixture = Fixture::new();

    // 1. evolution ladder
    let ladder = parse_json(fixture.command(&["evolution", "ladder", "--json"]));
    assert_eq!(ladder["command"], "evolution ladder");
    assert_eq!(ladder["levels"].as_array().unwrap().len(), 6);
    assert_eq!(ladder["failure_modes"].as_array().unwrap().len(), 3);
    assert!(
        ladder["safety_spine"]["sandboxed_execution"]
            .as_bool()
            .unwrap()
    );
    assert!(
        ladder["safety_spine"]["monotonic_rollback"]
            .as_bool()
            .unwrap()
    );

    // 2. evolution genome
    let genome = parse_json(fixture.command(&["evolution", "genome", "--json"]));
    assert_eq!(genome["command"], "evolution genome");
    assert!(genome["genome"]["genome_id"].is_string());

    // 3. evolution genome step
    let step = parse_json(fixture.command(&["evolution", "genome", "--step", "--json"]));
    assert_eq!(step["command"], "evolution genome");
    assert!(step["status"] == "promoted" || step["status"] == "rolled_back");

    // 4. evolution routing-card
    let routing = parse_json(fixture.command(&["evolution", "routing-card", "--json"]));
    assert_eq!(routing["command"], "evolution routing-card");
    assert_eq!(routing["entries"].as_array().unwrap().len(), 4);

    // 5. evolution lineage ASCII
    let lineage = parse_json(fixture.command(&["evolution", "lineage", "--json"]));
    assert_eq!(lineage["command"], "evolution lineage");
    assert!(!lineage["nodes"].as_array().unwrap().is_empty());

    // 6. evolution lineage --export-html
    let html_path = fixture.workspace.join("lineage.html");
    let lineage_html = parse_json(fixture.command(&[
        "evolution",
        "lineage",
        "--export-html",
        html_path.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(lineage_html["command"], "evolution lineage");
    assert!(html_path.exists());
    let html_content = fs::read_to_string(&html_path).unwrap();
    assert!(html_content.contains("<!DOCTYPE html>"));
    assert!(html_content.contains("Evolutionary Lineage Visualizer"));

    // 7. evolution depth
    let depth = parse_json(fixture.command(&["evolution", "depth", "--json"]));
    assert_eq!(depth["command"], "evolution depth");
    assert_eq!(depth["current_depth"], 3);
    assert!(depth["fixed_point"]["converged"].as_bool().unwrap());
}

#[test]
fn sunless_prp_pipeline_commands_execute_cleanly() {
    let fixture = Fixture::new();

    // 1. prp inspect
    let inspect = parse_json(fixture.command(&["prp", "inspect", "--json"]));
    assert_eq!(inspect["command"], "prp inspect");
    assert_eq!(inspect["prp"]["id"], "PRP-PHASE16-RSI");
    assert_eq!(
        inspect["prp"]["required_gates"].as_array().unwrap().len(),
        4
    );

    // 2. prp scaffold to file
    let prp_path = fixture.workspace.join("test_prp.json");
    let scaffold = parse_json(fixture.command(&[
        "prp",
        "scaffold",
        "--id",
        "PRP-SUNLESS-001",
        "--title",
        "Sunless Evolution Verification",
        "--out",
        prp_path.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(scaffold["command"], "prp scaffold");
    assert!(prp_path.exists());

    // 3. prp inspect from file
    let inspect_file = parse_json(fixture.command(&[
        "prp",
        "inspect",
        "--input",
        prp_path.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(inspect_file["prp"]["id"], "PRP-SUNLESS-001");

    // 4. prp validate all pass
    let validate_pass = parse_json(fixture.command(&[
        "prp",
        "validate",
        "--input",
        prp_path.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(validate_pass["command"], "prp validate");
    assert!(validate_pass["report"]["all_passed"].as_bool().unwrap());
    assert_eq!(
        validate_pass["report"]["gate_results"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // 5. prp validate with forced gate failure
    let validate_fail =
        parse_json(fixture.command(&["prp", "validate", "--fail-gate", "unit", "--json"]));
    assert_eq!(validate_fail["command"], "prp validate");
    assert!(!validate_fail["report"]["all_passed"].as_bool().unwrap());
    assert_eq!(
        validate_fail["report"]["gate_results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
