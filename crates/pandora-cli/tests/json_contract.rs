use pandora_runtime::SessionStore;
use pandora_types::{
    EventContext, EventId, EventPayload, EventType, PackageCompatibility, PackageDependency,
    PackageKind, PackageManifest, PrincipalId, RuntimeEvent, Session, SessionId, TenantId,
    Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CREDENTIAL_ENV: &str = "PANDORA_JSON_CONTRACT_KEY";
const CREDENTIAL_VALUE: &str = "json-contract-secret-value";
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
            "pandora-json-contract-{}-{timestamp}-{sequence}",
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

    /// Write a real config so commands that require one are reachable.
    ///
    /// The default fixture stays unconfigured on purpose: `doctor` is expected
    /// to fail closed when no config exists, and that assertion should not be
    /// reachable through a constructor that quietly configures it.
    fn setup(&self) {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(["setup", "--config"])
            .arg(&self.config)
            .args(["--data-dir"])
            .arg(&self.data)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .output()
            .expect("setup command should start");
        assert!(
            output.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            self.config.exists(),
            "setup should have written the config file"
        );
    }

    fn run(&self, args: &[&str]) -> JsonResponse {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env(CREDENTIAL_ENV, CREDENTIAL_VALUE)
            .env_remove("PANDORA_PROVIDER_URL")
            .output()
            .expect("Pandora command should start");
        JsonResponse::from_output(output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct JsonResponse {
    status: ExitStatus,
    text: String,
    value: Value,
}

impl JsonResponse {
    fn from_output(output: Output) -> Self {
        let text = String::from_utf8(output.stdout).expect("JSON output must be valid UTF-8");
        assert!(
            !text.contains(CREDENTIAL_VALUE),
            "JSON output must not contain credential values"
        );
        let value = serde_json::from_str(&text).expect("command output must be one JSON value");
        Self {
            status: output.status,
            text,
            value,
        }
    }

    fn success(self, command: &str) -> Value {
        assert!(self.status.success(), "command failed: {}", self.text);
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["command"], command);
        self.value
    }

    fn error(self, code: &str, exit_code: i32) -> Value {
        assert_eq!(
            self.status.code(),
            Some(exit_code),
            "response: {}",
            self.text
        );
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["code"], code);
        assert!(self.value["message"].is_string());
        assert!(self.value["details"].is_object());
        assert!(self.value.get("command").is_none());
        self.value
    }
}

#[test]
fn release_critical_success_envelopes_are_stable() {
    let fixture = Fixture::new();

    let version = fixture.run(&["--version", "--json"]).success("version");
    assert!(version["pandora_version"].is_string());

    let setup = fixture
        .run(&[
            "setup",
            "--provider-url",
            "https://provider.example/v1",
            "--model",
            "contract-model",
            "--api-key-env",
            CREDENTIAL_ENV,
            "--json",
        ])
        .success("setup");
    assert_eq!(setup["config_path"], path_value(&fixture.config));
    assert_eq!(setup["data_dir"], path_value(&fixture.data));
    assert_eq!(setup["workspace"], path_value(&fixture.workspace));
    assert_eq!(setup["provider_configured"], true);
    assert_eq!(setup["provider_model"], "contract-model");
    assert_eq!(setup["api_key_env"], CREDENTIAL_ENV);
    assert_eq!(setup["interactive"], false);

    let doctor = fixture.run(&["doctor", "--json"]).success("doctor");
    assert_eq!(doctor["healthy"], true);
    assert!(doctor["platform"].is_object());
    assert_eq!(doctor["config_path"], path_value(&fixture.config));
    assert_eq!(doctor["storage_path"], path_value(&fixture.data));
    assert_eq!(doctor["workspace_path"], path_value(&fixture.workspace));
    assert_eq!(doctor["provider"]["configured"], true);
    assert_eq!(doctor["provider"]["credential"], "available");
    assert_eq!(doctor["policy"]["effect_boundary"], "reference_monitor");
    assert!(doctor["containment"].is_object());
    assert!(doctor["checks"].is_array());

    let target = fixture.root.join("installed-pandora");
    let artifact = fixture.root.join("candidate.bin");
    let previous = b"previous release bytes";
    let candidate = b"verified candidate bytes";
    fs::write(&target, previous).expect("previous target should be written");
    fs::write(&artifact, candidate).expect("candidate should be written");
    let checksum = hash_artifact(candidate);
    let update = fixture
        .run(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            &checksum,
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .success("update");
    assert_eq!(update["verified"], true);
    assert_eq!(update["artifact"], path_value(&artifact));
    assert_eq!(update["target"], path_value(&target));
    assert_eq!(update["signature_verified"], false);
    assert_eq!(update["dry_run"], false);

    let rollback = fixture
        .run(&[
            "update",
            "--rollback",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .success("update rollback");
    assert_eq!(rollback["target"], path_value(&target));
    assert_eq!(rollback["restored"], true);
    assert_eq!(rollback["dry_run"], false);
    assert_eq!(fs::read(&target).unwrap(), previous);

    let preview = fixture
        .run(&["uninstall", "--dry-run", "--json"])
        .success("uninstall");
    assert_eq!(preview["dry_run"], true);
    assert!(preview["would_remove"].is_array());
    assert_eq!(preview["preserved"][0], path_value(&fixture.workspace));

    let uninstall = fixture
        .run(&["uninstall", "--yes", "--json"])
        .success("uninstall");
    assert_eq!(uninstall["dry_run"], false);
    assert!(uninstall["removed"].is_array());
    assert_eq!(uninstall["preserved"][0], path_value(&fixture.workspace));
}

#[test]
fn release_critical_error_envelopes_match_process_exit_codes() {
    let fixture = Fixture::new();

    let usage = fixture.run(&["update", "--json"]).error("usage_error", 2);
    assert_eq!(usage["details"], serde_json::json!({}));

    let configuration = fixture
        .run(&["doctor", "--json"])
        .error("configuration_error", 10);
    assert_eq!(configuration["details"]["healthy"], false);

    let artifact = fixture.root.join("candidate.bin");
    fs::write(&artifact, b"candidate").expect("candidate should be written");
    let target = fixture.root.join("target");
    let update = fixture
        .run(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .error("update_error", 70);
    assert_eq!(update["details"]["reason"], "checksum_mismatch");
    assert_eq!(update["details"]["path"], path_value(&artifact));
}

#[test]
fn runtime_engines_expose_the_core_inventory_without_a_service() {
    let fixture = Fixture::new();

    // No configuration, no provider, no store: the inventory is compiled-in.
    let listed = fixture.run(&["runtime", "engines", "list", "--json"]);
    let listed = listed.success("runtime engines list");
    let engines = listed["engines"]
        .as_array()
        .expect("engines should be an array");
    assert_eq!(engines.len(), 22);
    assert_eq!(listed["count"], 22);
    assert_eq!(
        listed["categories"]["Core authority"], 2,
        "execution controller and reference monitor are the constitutional pair"
    );

    let by_id = |id: &str| -> Value {
        engines
            .iter()
            .find(|engine| engine["id"] == id)
            .unwrap_or_else(|| panic!("engine {id} should be listed"))
            .clone()
    };

    let controller = by_id("execution-controller");
    assert_eq!(controller["authority"], "Runtime authority");
    assert_eq!(controller["category"], "Core authority");
    let related = controller["related_components"]
        .as_array()
        .expect("related components should be an array");
    for component in ["Parliament", "Shadow Council", "ReferenceMonitor"] {
        assert!(
            related.iter().any(|entry| entry == component),
            "execution controller should reference {component}"
        );
    }

    // The monitor is the only permit issuer in the inventory.
    assert_eq!(
        by_id("reference-monitor")["authority"],
        "Sole permit issuer"
    );

    // Parliament and the Shadow Council stay components. A top-level entry
    // would imply they can be selected on their own.
    for engine in engines {
        let id = engine["id"].as_str().expect("engine id should be a string");
        assert!(
            !["parliament", "shadow-council"].contains(&id),
            "{id} must stay a component of execution-controller"
        );
    }

    let inspected = fixture
        .run(&[
            "runtime",
            "engines",
            "inspect",
            "execution-controller",
            "--json",
        ])
        .success("runtime engines inspect");
    assert_eq!(inspected["engine"]["id"], "execution-controller");
    assert_eq!(inspected["engine"]["name"], "ExecutionController");
    assert!(
        inspected["engine"]["invariants"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "inspect should expose the deep contract, not just the identity"
    );

    // Fail closed rather than reporting an empty result for a bad id.
    let unknown = fixture
        .run(&[
            "runtime",
            "engines",
            "inspect",
            "definitely-not-a-real-engine",
            "--json",
        ])
        .error("usage_error", 2);
    assert!(
        unknown["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown engine")),
        "unknown engine should name the problem: {unknown}"
    );
}

#[test]
fn session_events_page_in_bounded_slices() {
    let fixture = Fixture::new();
    fixture.setup();

    let session_id = SessionId::new("paged-session-1").unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let sessions = SessionStore::open(fixture.data.join("sessions.sqlite3")).unwrap();
    sessions
        .create(&Session::new(
            session_id.clone(),
            principal.clone(),
            tenant.clone(),
            workspace.clone(),
            Timestamp::from_unix_seconds(10),
        ))
        .expect("session should be created");
    for index in 1_u64..=5 {
        sessions
            .append_event_at(
                &session_id,
                &principal,
                &tenant,
                &workspace,
                &RuntimeEvent::new(
                    EventId::new(format!("event-{index}")).unwrap(),
                    EventType::SessionStarted,
                    EventContext::new(tenant.clone(), workspace.clone())
                        .with_session(session_id.clone()),
                    EventPayload::Empty,
                ),
                Timestamp::from_unix_seconds(10 + index),
            )
            .expect("event should be appended");
    }
    drop(sessions);

    // First page: bounded, and the cursor points at the next unread event.
    let first = fixture
        .run(&[
            "session",
            "events",
            "paged-session-1",
            "--limit",
            "2",
            "--json",
        ])
        .success("session events");
    assert_eq!(first["count"], 2);
    assert_eq!(
        first["events"].as_array().map(Vec::len),
        Some(2),
        "a page must never exceed --limit"
    );
    assert_eq!(first["has_more"], true);
    let cursor = first["next_sequence"]
        .as_u64()
        .expect("a partial page must report a cursor");

    // Second page resumes exactly where the first stopped, with no gap and no
    // repeat. Together the two pages cover all five events.
    let second = fixture
        .run(&[
            "session",
            "events",
            "paged-session-1",
            "--after-sequence",
            &cursor.to_string(),
            "--limit",
            "2",
            "--json",
        ])
        .success("session events");
    assert_eq!(second["after_sequence"], cursor as u64);
    let first_ids = event_ids(&first);
    let second_ids = event_ids(&second);
    for id in &first_ids {
        assert!(
            !second_ids.contains(id),
            "event {id} appeared in both pages, so the cursor repeated a record"
        );
    }
    assert_eq!(second["has_more"], true);

    // Final page reports exhaustion, so a client can stop without guessing.
    let last = fixture
        .run(&[
            "session",
            "events",
            "paged-session-1",
            "--after-sequence",
            &second["next_sequence"].as_u64().unwrap().to_string(),
            "--json",
        ])
        .success("session events");
    assert_eq!(last["has_more"], false);
    assert_eq!(last["next_sequence"], Value::Null);
    let mut seen = first_ids;
    seen.extend(event_ids(&second));
    seen.extend(event_ids(&last));
    assert_eq!(seen.len(), 5, "paging dropped an event: {seen:?}");
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        5,
        "paging repeated an event: {seen:?}"
    );

    // Bad bounds fail closed as usage errors before the store is consulted, so
    // an invalid limit can never be mistaken for a session with no events.
    for bad in ["0", "257", "not-a-number"] {
        let rejected = fixture
            .run(&[
                "session",
                "events",
                "paged-session-1",
                "--limit",
                bad,
                "--json",
            ])
            .error("usage_error", 2);
        assert!(
            rejected["message"]
                .as_str()
                .is_some_and(|message| message.contains("limit")),
            "limit {bad} should be named in the error: {rejected}"
        );
    }

    // An unknown session fails closed rather than returning an empty page that
    // would read as "this session has no events".
    let missing = fixture
        .run(&["session", "events", "no-such-session", "--json"])
        .error("internal_error", 60);
    assert!(
        missing["message"]
            .as_str()
            .is_some_and(|message| message.contains("not found")),
        "a missing session should say so: {missing}"
    );
}

fn event_ids(page: &Value) -> Vec<String> {
    page["events"]
        .as_array()
        .expect("events should be an array")
        .iter()
        .map(|event| event["event_id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn tool_catalog_active_adds_packaged_genes_without_moving_the_default() {
    let fixture = Fixture::new();
    fixture.setup();

    // The built-in set, captured before anything is packaged. This is the
    // baseline the default invocation must keep producing.
    let baseline = fixture.run(&["tool", "list", "--json"]);
    let baseline_text = baseline.text.clone();
    let baseline = baseline.success("tool list");
    let builtin_count = baseline["tools"]
        .as_array()
        .expect("tools should be an array")
        .len();
    assert!(
        baseline.get("catalog").is_none(),
        "the default output must not gain a catalog field: {baseline}"
    );

    let wasm = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
            (func (export "pandora_run") (param i32 i32) (result i64)
                local.get 0
                i64.extend_i32_u
                i64.const 32
                i64.shl
                local.get 1
                i64.extend_i32_u
                i64.or))"#,
    )
    .unwrap();
    let gene = PackageManifest::new(
        "example/catalog-echo",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(&wasm),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let domain_artifact = b"wasm domain\n";
    let domain = PackageManifest::new(
        "example/catalog-domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(domain_artifact),
        vec![PackageDependency::new("example/catalog-echo", "1.0.0", false).unwrap()],
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    for (name, manifest, artifact) in [
        ("catalog-echo", &gene, wasm.as_slice()),
        ("catalog-domain", &domain, domain_artifact.as_slice()),
    ] {
        let manifest_path = fixture.root.join(format!("{name}.json"));
        let artifact_path = fixture.root.join(format!("{name}.artifact"));
        fs::write(&manifest_path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
        fs::write(&artifact_path, artifact).unwrap();
        fixture
            .run(&[
                "package",
                "admit",
                "--manifest",
                manifest_path.to_str().unwrap(),
                "--artifact",
                artifact_path.to_str().unwrap(),
                "--json",
            ])
            .success("package admit");
    }
    for id in ["example/catalog-echo", "example/catalog-domain"] {
        fixture
            .run(&["package", "enable", id, "1.0.0", "--yes", "--json"])
            .success("package enable");
    }

    // Naming the built-in catalog must be indistinguishable from not naming it.
    let explicit = fixture.run(&["tool", "list", "--catalog", "builtin", "--json"]);
    let explicit_text = explicit.text.clone();
    let explicit = explicit.success("tool list");
    assert_eq!(
        explicit_text, baseline_text,
        "--catalog builtin changed the default output"
    );
    assert_eq!(explicit["tools"].as_array().unwrap().len(), builtin_count);

    // The active catalog adds the packaged gene and nothing else.
    let active = fixture
        .run(&["tool", "list", "--catalog", "active", "--json"])
        .success("tool list");
    assert_eq!(active["catalog"], "active");
    let active_tools = active["tools"]
        .as_array()
        .expect("tools should be an array");
    let builtin_ids = baseline["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["id"].as_str().unwrap().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let packaged = active_tools
        .iter()
        .filter(|tool| !builtin_ids.contains(tool["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        packaged.len(),
        1,
        "expected exactly the one packaged gene to be added: {active}"
    );
    let packaged_id = packaged[0]["id"].as_str().unwrap().to_owned();
    assert!(
        packaged_id.starts_with("package."),
        "a packaged gene should keep its aliased id: {packaged_id}"
    );

    // Inspect resolves the gene from the active catalog, and the same id is
    // honestly absent from the built-in one.
    let inspected = fixture
        .run(&[
            "tool",
            "inspect",
            &packaged_id,
            "--catalog",
            "active",
            "--json",
        ])
        .success("tool inspect");
    assert_eq!(inspected["tool"]["id"], packaged_id);
    assert_eq!(inspected["tool"]["capability"], "wasm.execute");
    assert_eq!(inspected["catalog"], "active");

    let missing = fixture
        .run(&["tool", "inspect", &packaged_id, "--json"])
        .error("usage_error", 2);
    assert!(
        missing["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown tool")),
        "the default catalog should not claim to hold a packaged gene: {missing}"
    );

    // An unknown selector fails closed instead of quietly reading built-ins,
    // which would make a typo look like a successful answer.
    let rejected = fixture
        .run(&["tool", "list", "--catalog", "not-a-catalog", "--json"])
        .error("usage_error", 2);
    assert!(
        rejected["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown tool catalog")),
        "an unknown catalog should be named: {rejected}"
    );
}

fn path_value(path: &Path) -> Value {
    Value::String(path.to_string_lossy().into_owned())
}
