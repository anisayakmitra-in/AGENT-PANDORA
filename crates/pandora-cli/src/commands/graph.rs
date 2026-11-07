use super::{LOCAL_TENANT, LOCAL_WORKSPACE, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    AsciiGridRenderer, ForceDirectedLayout, GraphInput, GraphIntelligenceEngine, GraphKind,
    GraphScope, GraphStore, GraphStoreError, Point2D, ViewportCamera, MAX_GRAPH_INPUT_BYTES,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_GRAPH_INPUT_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct GraphInputDocument {
    inputs: Vec<GraphInputRecord>,
}

#[derive(Debug, Deserialize)]
struct GraphInputRecord {
    path: String,
    content: String,
    provenance: String,
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let first = args.first().ok_or_else(|| {
        CliError::usage("graph requires 'code', 'knowledge', 'review', 'architecture', 'map', or 'visual'")
    })?;
    if first == "map" {
        return map_command(&args[1..]);
    }
    if first == "visual" {
        return visual_command(&args[1..]);
    }
    let kind = parse_kind(first)?;
    let rest = &args[1..];
    // `show` is a read of an already-stored snapshot. Everything else keeps the
    // original build shape, so `graph code --input <path>` is untouched.
    match rest.first().map(String::as_str) {
        Some("show") => show(kind, &rest[1..]),
        Some("remove") => remove(kind, &rest[1..]),
        Some("visual") => visual(kind, &rest[1..]),
        _ => build(kind, rest),
    }
}

pub fn map_command(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args.first().map(String::as_str).unwrap_or("validate");
    let rest = if args.is_empty() { args } else { &args[1..] };
    match action {
        "validate" => validate_map(rest),
        "show" => show_map(rest),
        unknown => Err(CliError::usage(format!(
            "unknown graph map action '{unknown}', expected 'validate' or 'show'"
        ))),
    }
}

fn validate_map(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["path"])?;
    let path = parsed
        .value("path")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".pandora/feature_map.json"));

    if !path.is_file() {
        return Err(CliError::execution(
            format!("feature map file not found at '{}'", path.display()),
            json!({"path": path}),
        ));
    }

    let bytes = fs::read(&path).map_err(|e| {
        CliError::execution(
            format!("failed to read '{}': {e}", path.display()),
            json!({"path": path}),
        )
    })?;
    let map: pandora_types::FeatureMap = serde_json::from_slice(&bytes).map_err(|e| {
        CliError::execution(
            format!("invalid feature map JSON in '{}': {e}", path.display()),
            json!({"path": path}),
        )
    })?;

    map.validate().map_err(|e| {
        CliError::execution(
            format!("feature map validation failed: {e}"),
            json!({"path": path}),
        )
    })?;

    let mut resolved_features = Vec::new();
    for feature in &map.features {
        let entry_path = Path::new(&feature.entry_point);
        let exists = entry_path.exists();
        resolved_features.push(json!({
            "name": feature.name,
            "entry_point": feature.entry_point,
            "exists": exists,
            "invariants_count": feature.invariants.len(),
            "dependencies_count": feature.dependencies.len(),
        }));
    }

    let count = map.features.len();
    Ok(success(
        "graph map validate",
        json!({
            "path": path,
            "version": map.version,
            "map_version": map.version,
            "feature_count": count,
            "features": resolved_features,
        }),
        format!(
            "Validated feature map at '{}' ({count} feature(s) verified)",
            path.display()
        ),
    ))
}

fn show_map(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["path"])?;
    let path = parsed
        .value("path")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".pandora/feature_map.json"));

    if !path.is_file() {
        return Err(CliError::execution(
            format!("feature map file not found at '{}'", path.display()),
            json!({"path": path}),
        ));
    }

    let bytes = fs::read(&path).map_err(|e| {
        CliError::execution(
            format!("failed to read '{}': {e}", path.display()),
            json!({"path": path}),
        )
    })?;
    let map: pandora_types::FeatureMap = serde_json::from_slice(&bytes).map_err(|e| {
        CliError::execution(
            format!("invalid feature map JSON in '{}': {e}", path.display()),
            json!({"path": path}),
        )
    })?;

    let count = map.features.len();
    Ok(success(
        "graph map show",
        json!({
            "path": path,
            "version": map.version,
            "map_version": map.version,
            "features": map.features,
        }),
        format!("Loaded feature map ({count} feature(s))"),
    ))
}

fn remove(kind: GraphKind, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["store", "tenant", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "graph remove does not accept positional arguments",
        ));
    }
    let store_path = parsed
        .value("store")
        .ok_or_else(|| CliError::usage("graph remove requires '--store <path>'"))?;
    let scope = GraphScope::new(
        parsed.value("tenant").unwrap_or(LOCAL_TENANT),
        parsed.value("workspace").unwrap_or(LOCAL_WORKSPACE),
    )
    .map_err(|error| CliError::usage(format!("invalid graph scope: {error}")))?;
    let store = GraphStore::open(store_path).map_err(graph_store_error)?;
    let removed = store.remove(kind, &scope).map_err(graph_store_error)?;
    Ok(success(
        "graph remove",
        json!({
            "kind": kind.as_str(),
            "removed": removed,
            "tenant": scope.tenant(),
            "workspace": scope.workspace(),
        }),
        if removed {
            format!("Removed stored {} graph", kind.as_str())
        } else {
            format!("No stored {} graph to remove", kind.as_str())
        },
    ))
}

/// Report the stored snapshot for a scope.
///
/// This is the read counterpart to `graph build --store`. It calls
/// `GraphStore::load`, so it returns the same digest the write recorded rather
/// than re-deriving a graph, and it never calls `replace` or `remove`.
///
/// An absent snapshot is a real state, not an error: the scope simply holds no
/// graph yet. That is reported as `present: false` with a zero exit, because
/// "you have not built this yet" is an answer, while a failure would make
/// scripting a graph's presence awkward for no gain.
fn show(kind: GraphKind, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["store", "tenant", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "graph show does not accept positional arguments",
        ));
    }
    let store_path = parsed
        .value("store")
        .ok_or_else(|| CliError::usage("graph show requires '--store <path>'"))?;
    let scope = GraphScope::new(
        parsed.value("tenant").unwrap_or(LOCAL_TENANT),
        parsed.value("workspace").unwrap_or(LOCAL_WORKSPACE),
    )
    .map_err(|error| CliError::usage(format!("invalid graph scope: {error}")))?;
    let store = GraphStore::open(store_path).map_err(graph_store_error)?;
    let Some(snapshot) = store.load(kind, &scope).map_err(graph_store_error)? else {
        return Ok(success(
            "graph show",
            json!({
                "kind": kind.as_str(),
                "present": false,
                "tenant": scope.tenant(),
                "workspace": scope.workspace(),
            }),
            format!(
                "no stored {} graph for tenant {} workspace {}",
                kind.as_str(),
                scope.tenant(),
                scope.workspace()
            ),
        ));
    };
    let mut data = serde_json::to_value(&snapshot).map_err(|error| {
        CliError::internal(
            "could not serialize graph snapshot",
            json!({"error": error.to_string()}),
        )
    })?;
    data.as_object_mut()
        .expect("graph snapshots serialize as JSON objects")
        .insert("present".to_owned(), json!(true));
    Ok(success(
        "graph show",
        data,
        format!(
            "{} graph: {} source(s), {} node(s), {} edge(s), digest {}",
            kind.as_str(),
            snapshot.source_count(),
            snapshot.nodes().len(),
            snapshot.edges().len(),
            snapshot.digest()
        ),
    ))
}

fn visual_command(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "store", "tenant", "workspace", "width", "height", "iterations", "kind",
            "zoom", "pan-x", "pan-y",
        ],
    )?;
    let kind = if let Some(k) = parsed.value("kind") {
        parse_kind(k)?
    } else {
        GraphKind::Knowledge
    };
    visual(kind, args)
}

fn visual(kind: GraphKind, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "store", "tenant", "workspace", "width", "height", "iterations", "kind",
            "zoom", "pan-x", "pan-y",
        ],
    )?;
    let width: usize = parsed
        .value("width")
        .map(|v| v.parse().map_err(|_| CliError::usage("--width must be an integer")))
        .transpose()?
        .unwrap_or(60);
    let height: usize = parsed
        .value("height")
        .map(|v| v.parse().map_err(|_| CliError::usage("--height must be an integer")))
        .transpose()?
        .unwrap_or(20);
    let iterations: usize = parsed
        .value("iterations")
        .map(|v| v.parse().map_err(|_| CliError::usage("--iterations must be an integer")))
        .transpose()?
        .unwrap_or(100);
    let zoom: f64 = parsed
        .value("zoom")
        .map(|v| v.parse().map_err(|_| CliError::usage("--zoom must be a number")))
        .transpose()?
        .unwrap_or(1.0);
    let pan_x: f64 = parsed
        .value("pan-x")
        .map(|v| v.parse().map_err(|_| CliError::usage("--pan-x must be a number")))
        .transpose()?
        .unwrap_or(0.0);
    let pan_y: f64 = parsed
        .value("pan-y")
        .map(|v| v.parse().map_err(|_| CliError::usage("--pan-y must be a number")))
        .transpose()?
        .unwrap_or(0.0);

    let mut layout = ForceDirectedLayout::new(width as f64, height as f64);

    let mut loaded_from_store = false;
    if let Some(store_path) = parsed.value("store") {
        let scope = GraphScope::new(
            parsed.value("tenant").unwrap_or(LOCAL_TENANT),
            parsed.value("workspace").unwrap_or(LOCAL_WORKSPACE),
        )
        .map_err(|error| CliError::usage(format!("invalid graph scope: {error}")))?;
        if let Ok(Some(snapshot)) = GraphStore::open(store_path).and_then(|store| store.load(kind, &scope)) {
            loaded_from_store = true;
            let center_x = (width as f64) / 2.0;
            let center_y = (height as f64) / 2.0;
            let total_nodes = snapshot.nodes().len().max(1);
            for (i, node) in snapshot.nodes().iter().enumerate() {
                let angle = (i as f64) * 2.0 * std::f64::consts::PI / (total_nodes as f64);
                let radius = (width.min(height) as f64) * 0.35;
                let init_x = (center_x + radius * angle.cos()).clamp(2.0, (width.saturating_sub(2)) as f64);
                let init_y = (center_y + radius * angle.sin()).clamp(2.0, (height.saturating_sub(2)) as f64);
                let label = node.label();
                let short_label = if label.len() > 10 {
                    &label[..10]
                } else {
                    label
                };
                layout.add_node(node.id(), short_label, init_x, init_y);
            }
            for edge in snapshot.edges() {
                layout.add_edge(edge.from(), edge.to());
            }
        }
    }

    if !loaded_from_store {
        match kind {
            GraphKind::Code => {
                layout.add_node("cli", "cli", width as f64 * 0.2, height as f64 * 0.2);
                layout.add_node("svc", "service", width as f64 * 0.5, height as f64 * 0.2);
                layout.add_node("rt", "runtime", width as f64 * 0.5, height as f64 * 0.5);
                layout.add_node("types", "types", width as f64 * 0.8, height as f64 * 0.8);
                layout.add_node("prov", "provider", width as f64 * 0.2, height as f64 * 0.8);
                layout.add_edge("cli", "rt");
                layout.add_edge("cli", "svc");
                layout.add_edge("svc", "rt");
                layout.add_edge("rt", "types");
                layout.add_edge("rt", "prov");
            }
            GraphKind::Architecture => {
                layout.add_node("gw", "Gateway", width as f64 * 0.5, height as f64 * 0.2);
                layout.add_node("core", "CoreEngine", width as f64 * 0.5, height as f64 * 0.5);
                layout.add_node("mesh", "SwarmMesh", width as f64 * 0.2, height as f64 * 0.5);
                layout.add_node("vfs", "ContextVfs", width as f64 * 0.8, height as f64 * 0.5);
                layout.add_node("hw", "HWDispatch", width as f64 * 0.5, height as f64 * 0.8);
                layout.add_edge("gw", "core");
                layout.add_edge("core", "mesh");
                layout.add_edge("core", "vfs");
                layout.add_edge("core", "hw");
            }
            GraphKind::Review => {
                layout.add_node("target", "TargetDiff", width as f64 * 0.5, height as f64 * 0.2);
                layout.add_node("argus", "ArgusReview", width as f64 * 0.3, height as f64 * 0.6);
                layout.add_node("linter", "AstChecker", width as f64 * 0.7, height as f64 * 0.6);
                layout.add_node("verdict", "Verdict", width as f64 * 0.5, height as f64 * 0.8);
                layout.add_edge("target", "argus");
                layout.add_edge("target", "linter");
                layout.add_edge("argus", "verdict");
                layout.add_edge("linter", "verdict");
            }
            _ => {
                layout.add_node("k0", "VFS", width as f64 * 0.3, height as f64 * 0.3);
                layout.add_node("k1", "HNSW", width as f64 * 0.7, height as f64 * 0.3);
                layout.add_node("k2", "BFT", width as f64 * 0.3, height as f64 * 0.7);
                layout.add_node("k3", "SIMD", width as f64 * 0.7, height as f64 * 0.7);
                layout.add_node("root", "Kernel", width as f64 * 0.5, height as f64 * 0.5);
                layout.add_edge("root", "k0");
                layout.add_edge("root", "k1");
                layout.add_edge("root", "k2");
                layout.add_edge("root", "k3");
                layout.add_edge("k0", "k1");
            }
        }
    }

    let iters_converged = layout.simulate_until_equilibrium(iterations, 0.01);
    let mut camera = ViewportCamera::new();
    camera.focus_on(Point2D::new((width as f64) / 2.0, (height as f64) / 2.0));
    camera.pan(pan_x, pan_y);
    camera.zoom = zoom;

    let rendered_lines = AsciiGridRenderer::render(&layout, &camera, width, height);
    let rendered_ascii = rendered_lines.join("\n");
    let summary = format!(
        "Rendered {} topological visual graph ({}x{}, {} nodes, {} edges, converged in {} iter(s), potential energy: {:.2})",
        kind.as_str(),
        width,
        height,
        layout.nodes.len(),
        layout.edges.len(),
        iters_converged,
        layout.potential_energy()
    );

    Ok(success(
        "graph visual",
        json!({
            "kind": kind.as_str(),
            "width": width,
            "height": height,
            "nodes_count": layout.nodes.len(),
            "edges_count": layout.edges.len(),
            "iterations_converged": iters_converged,
            "potential_energy": layout.potential_energy(),
            "rendered": rendered_ascii,
        }),
        format!("{summary}\n\n{rendered_ascii}"),
    ))
}

fn build(kind: GraphKind, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["input", "store", "tenant", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "graph commands do not accept positional arguments after the kind",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("graph requires '--input <path>'"))?;
    let scope = GraphScope::new(
        parsed.value("tenant").unwrap_or(LOCAL_TENANT),
        parsed.value("workspace").unwrap_or(LOCAL_WORKSPACE),
    )
    .map_err(|error| CliError::usage(format!("invalid graph scope: {error}")))?;
    let inputs = parse_inputs(&read_bounded(Path::new(input))?)?;
    let snapshot = GraphIntelligenceEngine::new()
        .build(kind, scope, inputs)
        .map_err(|error| CliError::usage(format!("invalid graph input: {error}")))?;
    let data = serde_json::to_value(&snapshot).map_err(|error| {
        CliError::internal(
            "could not serialize graph snapshot",
            json!({"error": error.to_string()}),
        )
    })?;
    let mut data = data;
    let persisted = if let Some(store_path) = parsed.value("store") {
        let store = GraphStore::open(store_path).map_err(graph_store_error)?;
        store.replace(&snapshot).map_err(graph_store_error)?;
        data.as_object_mut()
            .expect("graph snapshots serialize as JSON objects")
            .insert("persisted".to_owned(), json!(true));
        true
    } else {
        false
    };
    Ok(success(
        "graph build",
        data,
        if persisted {
            format!(
                "{} graph persisted: {} source(s), {} node(s), {} edge(s), digest {}",
                kind.as_str(),
                snapshot.source_count(),
                snapshot.nodes().len(),
                snapshot.edges().len(),
                snapshot.digest()
            )
        } else {
            format!(
                "{} graph: {} source(s), {} node(s), {} edge(s), digest {}",
                kind.as_str(),
                snapshot.source_count(),
                snapshot.nodes().len(),
                snapshot.edges().len(),
                snapshot.digest()
            )
        },
    ))
}

fn graph_store_error(error: GraphStoreError) -> CliError {
    CliError::execution(
        "could not persist graph snapshot",
        json!({"error": error.to_string()}),
    )
}

fn parse_kind(value: &str) -> Result<GraphKind, CliError> {
    match value {
        "code" => Ok(GraphKind::Code),
        "knowledge" => Ok(GraphKind::Knowledge),
        "review" => Ok(GraphKind::Review),
        "architecture" => Ok(GraphKind::Architecture),
        _ => Err(CliError::usage(
            "graph requires 'code', 'knowledge', 'review', or 'architecture'",
        )),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::execution(
            "could not read graph input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.len() > MAX_GRAPH_INPUT_DOCUMENT_BYTES {
        return Err(CliError::usage(format!(
            "graph input exceeds {MAX_GRAPH_INPUT_DOCUMENT_BYTES} bytes"
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::execution(
            "could not open graph input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_GRAPH_INPUT_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read graph input",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.len() as u64 > MAX_GRAPH_INPUT_DOCUMENT_BYTES {
        return Err(CliError::usage(format!(
            "graph input exceeds {MAX_GRAPH_INPUT_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn parse_inputs(bytes: &[u8]) -> Result<Vec<GraphInput>, CliError> {
    let document = serde_json::from_slice::<GraphInputDocument>(bytes)
        .map_err(|error| CliError::usage(format!("invalid graph JSON: {error}")))?;
    document
        .inputs
        .into_iter()
        .map(|input| {
            if input.content.len() > MAX_GRAPH_INPUT_BYTES {
                return Err(CliError::usage(format!(
                    "graph input '{}' exceeds {MAX_GRAPH_INPUT_BYTES} bytes",
                    input.path
                )));
            }
            GraphInput::new(input.path, input.content, input.provenance)
                .map_err(|error| CliError::usage(format!("invalid graph input: {error}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{build, parse_inputs, parse_kind};
    use pandora_runtime::{GraphKind, GraphScope, GraphStore};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn accepts_each_graph_projection() {
        assert_eq!(parse_kind("code").unwrap(), GraphKind::Code);
        assert_eq!(parse_kind("knowledge").unwrap(), GraphKind::Knowledge);
        assert_eq!(parse_kind("review").unwrap(), GraphKind::Review);
        assert_eq!(parse_kind("architecture").unwrap(), GraphKind::Architecture);
    }

    #[test]
    fn parses_evidence_input_without_reading_a_workspace() {
        let inputs = parse_inputs(
            br#"{"inputs":[{"path":"src/main.rs","content":"fn main() {}","provenance":"session:exec"}]}"#,
        )
        .unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].path(), "src/main.rs");
    }

    #[test]
    fn store_option_persists_the_selected_graph_scope() {
        let root = std::env::temp_dir().join(format!(
            "pandora-cli-graph-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let input_path = root.join("graph.json");
        let store_path = root.join("graphs.sqlite3");
        fs::write(
            &input_path,
            br#"{"inputs":[{"path":"src/main.rs","content":"fn main() {}","provenance":"session:exec"}]}"#,
        )
        .unwrap();

        let result = build(
            GraphKind::Code,
            &[
                "--input".to_owned(),
                input_path.display().to_string(),
                "--store".to_owned(),
                store_path.display().to_string(),
            ],
        )
        .unwrap();

        assert_eq!(result.data["persisted"], true);
        let store = GraphStore::open(&store_path).unwrap();
        let scope = GraphScope::new("local-tenant", "local-workspace").unwrap();
        assert!(store.load(GraphKind::Code, &scope).unwrap().is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn feature_map_command_validates_and_shows() {
        let root = std::env::temp_dir().join(format!(
            "pandora-cli-map-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let map_path = root.join("feature_map.json");
        fs::write(
            &map_path,
            br#"{
  "version": 1,
  "features": [
    {
      "name": "TestFeature",
      "entry_point": "src/main.rs",
      "description": "Test entry",
      "invariants": ["Invar 1"],
      "dependencies": []
    }
  ]
}"#,
        )
        .unwrap();

        let val_result = super::map_command(&[
            "validate".to_owned(),
            "--path".to_owned(),
            map_path.display().to_string(),
        ])
        .unwrap();
        assert_eq!(val_result.data["version"], 1);
        assert_eq!(val_result.data["feature_count"], 1);

        let show_result = super::map_command(&[
            "show".to_owned(),
            "--path".to_owned(),
            map_path.display().to_string(),
        ])
        .unwrap();
        assert_eq!(show_result.data["version"], 1);
        assert_eq!(show_result.data["features"][0]["name"], "TestFeature");

        let _ = fs::remove_dir_all(root);
    }
}
