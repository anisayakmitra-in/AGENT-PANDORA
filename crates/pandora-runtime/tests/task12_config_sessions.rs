use pandora_runtime::config::{ConfigOverrides, ProviderPricing, RuntimeConfig};
use pandora_runtime::sessions::SessionStore;
use pandora_types::{
    EventContext, EventId, EventPayload, EventType, PrincipalId, RuntimeEvent, Session, SessionId,
    TenantId, Timestamp, WorkspaceId,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[test]
fn configuration_uses_first_run_defaults() {
    let fixture = Fixture::new("config-defaults");
    let config = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &fixture.path.join("missing.json"),
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(config.provider_url(), None);
    assert_eq!(config.provider_model(), None);
    assert_eq!(config.data_dir(), fixture.path.join("default-data"));
    assert_eq!(
        config.workspace_dir(),
        fixture.path.join("default-workspace")
    );
}

#[test]
fn configuration_flags_override_environment_and_file() {
    let fixture = Fixture::new("config-precedence");
    let config_path = fixture.path.join("config.json");
    fs::write(
        &config_path,
        r#"{
            "provider_url": "https://file.example/v1",
            "provider_model": "file-model",
            "data_dir": "file-data",
            "workspace_dir": "file-workspace"
        }"#,
    )
    .unwrap();

    let environment = BTreeMap::from([
        (
            "PANDORA_PROVIDER_URL".to_owned(),
            "https://environment.example/v1".to_owned(),
        ),
        (
            "PANDORA_PROVIDER_MODEL".to_owned(),
            "environment-model".to_owned(),
        ),
        (
            "PANDORA_DATA_DIR".to_owned(),
            fixture.path.join("environment-data").display().to_string(),
        ),
    ]);
    let overrides = ConfigOverrides::default()
        .with_provider_url("https://flag.example/v1")
        .with_provider_model("flag-model")
        .with_workspace_dir(fixture.path.join("flag-workspace"));

    let config = RuntimeConfig::from_sources(
        &overrides,
        &environment,
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(config.provider_url(), Some("https://flag.example/v1"));
    assert_eq!(config.provider_model(), Some("flag-model"));
    assert_eq!(config.data_dir(), fixture.path.join("environment-data"));
    assert_eq!(config.workspace_dir(), fixture.path.join("flag-workspace"));
}

#[test]
fn configuration_persists_provider_model() {
    let fixture = Fixture::new("config-provider-model");
    let config_path = fixture.path.join("config.json");
    let config = RuntimeConfig::from_sources(
        &ConfigOverrides::default()
            .with_config_path(config_path.clone())
            .with_provider_model("gpt-5"),
        &BTreeMap::new(),
        &fixture.path.join("missing.json"),
        fixture.path.join("data"),
        fixture.path.join("workspace"),
    )
    .unwrap();

    config.write().unwrap();

    let loaded = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(loaded.provider_model(), Some("gpt-5"));
}

#[test]
fn named_provider_profiles_round_trip_and_select() {
    let fixture = Fixture::new("config-provider-profiles");
    let config_path = fixture.path.join("config.json");
    fs::write(
        &config_path,
        r#"{
            "providers": {
                "design": {
                    "base_url": "https://design.example/v1",
                    "model": "vision-model",
                    "api_key_env": "PANDORA_DESIGN_API_KEY"
                },
                "coding": {
                    "base_url": "https://coding.example/v1",
                    "model": "coding-model",
                    "api_key_env": "PANDORA_CODING_API_KEY",
                    "fallback_provider": "design"
                }
            },
            "active_provider": "design"
        }"#,
    )
    .unwrap();

    let config = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(config.provider_names(), &["coding", "design"]);
    assert_eq!(config.active_provider(), Some("design"));
    assert_eq!(config.provider_url(), Some("https://design.example/v1"));
    assert_eq!(config.provider_model(), Some("vision-model"));
    assert_eq!(
        config.provider_api_key_env(),
        Some("PANDORA_DESIGN_API_KEY")
    );

    let one_run = RuntimeConfig::from_sources(
        &ConfigOverrides::default().with_provider_name("coding"),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();
    assert_eq!(one_run.active_provider(), Some("coding"));
    assert_eq!(one_run.provider_model(), Some("coding-model"));

    config.write().unwrap();
    let reloaded = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(reloaded.active_provider(), Some("design"));
    assert_eq!(reloaded.provider_model(), Some("vision-model"));
    assert_eq!(
        reloaded
            .provider_profile("coding")
            .expect("coding profile should remain configured")
            .api_key_env(),
        "PANDORA_CODING_API_KEY"
    );
    assert_eq!(
        reloaded
            .provider_profile("coding")
            .expect("coding profile should remain configured")
            .fallback_provider(),
        Some("design")
    );
}

#[test]
fn provider_pricing_round_trips_and_calculates_known_cost() {
    let fixture = Fixture::new("config-provider-pricing");
    let config_path = fixture.path.join("config.json");
    fs::write(
        &config_path,
        r#"{
            "providers": {
                "direct": {
                    "base_url": "https://provider.example/v1",
                    "model": "model",
                    "api_key_env": "PANDORA_PROVIDER_API_KEY",
                    "pricing": {
                        "input_micros_per_million_tokens": 2000000,
                        "output_micros_per_million_tokens": 4000000
                    }
                },
                "fallback": {
                    "base_url": "https://fallback.example/v1",
                    "model": "fallback-model",
                    "api_key_env": "PANDORA_FALLBACK_API_KEY"
                }
            },
            "active_provider": "direct"
        }"#,
    )
    .unwrap();

    let config = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .unwrap();

    assert_eq!(
        config.provider_profile("direct").unwrap().pricing(),
        Some(ProviderPricing::new(2_000_000, 4_000_000))
    );
    assert_eq!(
        config.provider_cost_micros("direct", 1_000, 500),
        Some(4_000)
    );
    assert_eq!(config.provider_cost_micros("fallback", 1_000, 500), None);

    config.write().unwrap();
    let reloaded = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &config_path,
        fixture.path.join("other-data"),
        fixture.path.join("other-workspace"),
    )
    .unwrap();
    assert_eq!(
        reloaded.provider_profile("direct").unwrap().pricing(),
        Some(ProviderPricing::new(2_000_000, 4_000_000))
    );
}

#[test]
fn configuration_rejects_non_http_provider_urls() {
    let fixture = Fixture::new("config-invalid-url");
    let environment = BTreeMap::from([(
        "PANDORA_PROVIDER_URL".to_owned(),
        "ftp://provider.example/v1".to_owned(),
    )]);

    let result = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &environment,
        &fixture.path.join("missing.json"),
        fixture.path.join("data"),
        fixture.path.join("workspace"),
    );

    assert!(result.is_err());
}

#[test]
fn configuration_rejects_unknown_provider_selection() {
    let fixture = Fixture::new("config-unknown-provider");
    fs::write(
        fixture.path.join("config.json"),
        r#"{
            "providers": {
                "coding": {
                    "base_url": "https://coding.example/v1",
                    "model": "coding-model",
                    "api_key_env": "PANDORA_CODING_API_KEY"
                }
            }
        }"#,
    )
    .unwrap();

    let error = RuntimeConfig::from_sources(
        &ConfigOverrides::default().with_provider_name("missing"),
        &BTreeMap::new(),
        &fixture.path.join("config.json"),
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .expect_err("unknown provider selection should fail closed");

    assert!(matches!(
        error,
        pandora_runtime::config::ConfigError::UnknownProvider
    ));
}

#[test]
fn configuration_rejects_unknown_provider_fallback() {
    let fixture = Fixture::new("config-unknown-fallback");
    fs::write(
        fixture.path.join("config.json"),
        r#"{
            "providers": {
                "coding": {
                    "base_url": "https://coding.example/v1",
                    "model": "coding-model",
                    "api_key_env": "PANDORA_CODING_API_KEY",
                    "fallback_provider": "missing"
                }
            }
        }"#,
    )
    .unwrap();

    let error = RuntimeConfig::from_sources(
        &ConfigOverrides::default(),
        &BTreeMap::new(),
        &fixture.path.join("config.json"),
        fixture.path.join("default-data"),
        fixture.path.join("default-workspace"),
    )
    .expect_err("unknown fallback provider should fail closed");

    assert!(matches!(
        error,
        pandora_runtime::config::ConfigError::UnknownProvider
    ));
}

#[test]
fn configuration_write_uses_a_private_file_mode_when_supported() {
    let fixture = Fixture::new("config-permissions");
    let config_path = fixture.path.join("nested").join("config.json");
    let config = RuntimeConfig::from_sources(
        &ConfigOverrides::default().with_config_path(config_path.clone()),
        &BTreeMap::new(),
        &fixture.path.join("missing.json"),
        fixture.path.join("data"),
        fixture.path.join("workspace"),
    )
    .unwrap();

    config.write().unwrap();

    assert!(config_path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(config_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}

#[test]
fn sessions_survive_store_reopen_with_events() {
    let fixture = Fixture::new("session-reopen");
    let session = session("session-reopen", "principal-a", "tenant-a", "workspace-a");
    let event = event(&session);

    {
        let store = SessionStore::open(fixture.path.join("sessions.db")).unwrap();
        store.create(&session).unwrap();
        store
            .append_event(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                &event,
            )
            .unwrap();
    }

    let store = SessionStore::open(fixture.path.join("sessions.db")).unwrap();
    let snapshot = store
        .resume(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
        )
        .unwrap();

    assert_eq!(snapshot.session(), &session);
    assert_eq!(snapshot.events(), &[event]);
}

#[test]
fn session_list_isolated_by_principal_and_workspace() {
    let fixture = Fixture::new("session-isolation");
    let first = session("session-first", "principal-a", "tenant-a", "workspace-a");
    let second = session("session-second", "principal-b", "tenant-a", "workspace-a");
    let third = session("session-third", "principal-a", "tenant-a", "workspace-b");
    let store = SessionStore::open(fixture.path.join("sessions.db")).unwrap();

    store.create(&first).unwrap();
    store.create(&second).unwrap();
    store.create(&third).unwrap();

    let listed = store
        .list(
            first.principal_id(),
            first.tenant_id(),
            first.workspace_id(),
        )
        .unwrap();

    assert_eq!(listed, vec![first]);
}

#[test]
fn malformed_database_fails_closed() {
    let fixture = Fixture::new("session-corrupt");
    let path = fixture.path.join("sessions.db");
    fs::write(&path, b"not a sqlite database").unwrap();

    assert!(SessionStore::open(path).is_err());
}

#[test]
fn session_migrations_are_idempotent() {
    let fixture = Fixture::new("session-migrations");
    let path = fixture.path.join("sessions.db");

    drop(SessionStore::open(&path).unwrap());
    drop(SessionStore::open(path).unwrap());
}

fn session(id: &str, principal: &str, tenant: &str, workspace: &str) -> Session {
    Session::new(
        SessionId::new(id).unwrap(),
        PrincipalId::new(principal).unwrap(),
        TenantId::new(tenant).unwrap(),
        WorkspaceId::new(workspace).unwrap(),
        Timestamp::from_unix_seconds(1),
    )
}

fn event(session: &Session) -> RuntimeEvent {
    RuntimeEvent::new(
        EventId::new("event-1").unwrap(),
        EventType::SessionStarted,
        EventContext::new(session.tenant_id().clone(), session.workspace_id().clone())
            .with_session(session.id().clone()),
        EventPayload::Empty,
    )
}

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pandora-task12-{name}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(Path::new(&self.path));
    }
}
