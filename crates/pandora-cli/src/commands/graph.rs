use super::{LOCAL_TENANT, LOCAL_WORKSPACE, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    GraphInput, GraphIntelligenceEngine, GraphKind, GraphScope, GraphStore, GraphStoreError,
    MAX_GRAPH_INPUT_BYTES,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Read;
use std::path::Path;

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
    let kind = args
        .first()
        .ok_or_else(|| {
            CliError::usage("graph requires 'code', 'knowledge', 'review', or 'architecture'")
        })
        .and_then(|value| parse_kind(value))?;
    let rest = &args[1..];
    // `show` is a read of an already-stored snapshot. Everything else keeps the
    // original build shape, so `graph code --input <path>` is untouched.
    match rest.first().map(String::as_str) {
        Some("show") => show(kind, &rest[1..]),
        Some("remove") => remove(kind, &rest[1..]),
        _ => build(kind, rest),
    }
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
}
