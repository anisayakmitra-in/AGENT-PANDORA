use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

pub const MAX_GRAPH_INPUTS: usize = 2_048;
pub const MAX_GRAPH_INPUT_BYTES: usize = 1_048_576;
pub const MAX_GRAPH_NODES: usize = 8_192;
pub const MAX_GRAPH_EDGES: usize = 16_384;
pub const MAX_GRAPH_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphKind {
    Code,
    Knowledge,
    Review,
    Architecture,
}

impl GraphKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Knowledge => "knowledge",
            Self::Review => "review",
            Self::Architecture => "architecture",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    InvalidPath,
    TooManyInputs,
    InputTooLarge,
    DuplicateInput,
    NodeLimit,
    EdgeLimit,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::InvalidPath => formatter.write_str("graph input path must be relative and safe"),
            Self::TooManyInputs => formatter.write_str("graph input count exceeds the limit"),
            Self::InputTooLarge => formatter.write_str("graph input exceeds the size limit"),
            Self::DuplicateInput => formatter.write_str("graph input path was provided twice"),
            Self::NodeLimit => formatter.write_str("graph node limit was exceeded"),
            Self::EdgeLimit => formatter.write_str("graph edge limit was exceeded"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphScope {
    tenant: String,
    workspace: String,
}

impl GraphScope {
    pub fn new(
        tenant: impl Into<String>,
        workspace: impl Into<String>,
    ) -> Result<Self, GraphError> {
        Ok(Self {
            tenant: validate_text("tenant", tenant.into(), 256)?,
            workspace: validate_text("workspace", workspace.into(), 256)?,
        })
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    pub fn workspace(&self) -> &str {
        &self.workspace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphInput {
    path: String,
    content: String,
    provenance: String,
}

impl GraphInput {
    pub fn new(
        path: impl Into<String>,
        content: impl Into<String>,
        provenance: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let path = validate_path(path.into())?;
        let content = content.into();
        if content.len() > MAX_GRAPH_INPUT_BYTES {
            return Err(GraphError::InputTooLarge);
        }
        if content.chars().any(char::is_control) && !content.contains(['\n', '\r', '\t']) {
            return Err(GraphError::InvalidPath);
        }
        Ok(Self {
            path,
            content,
            provenance: validate_text("provenance", provenance.into(), 512)?,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    File,
    Module,
    Document,
    Heading,
    Finding,
    Layer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphNode {
    id: String,
    kind: GraphNodeKind,
    label: String,
    source: String,
    provenance_digest: String,
}

impl GraphNode {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn kind(&self) -> GraphNodeKind {
        self.kind
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphEdge {
    from: String,
    to: String,
    relation: String,
}

impl GraphEdge {
    pub fn from(&self) -> &str {
        &self.from
    }

    pub fn to(&self) -> &str {
        &self.to
    }

    pub fn relation(&self) -> &str {
        &self.relation
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphSnapshot {
    kind: GraphKind,
    scope: GraphScope,
    source_count: usize,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    digest: String,
}

impl GraphSnapshot {
    pub const fn kind(&self) -> GraphKind {
        self.kind
    }

    pub fn scope(&self) -> &GraphScope {
        &self.scope
    }

    pub const fn source_count(&self) -> usize {
        self.source_count
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GraphIntelligenceEngine {
    max_inputs: usize,
    max_nodes: usize,
    max_edges: usize,
}

impl Default for GraphIntelligenceEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphIntelligenceEngine {
    pub const fn new() -> Self {
        Self {
            max_inputs: MAX_GRAPH_INPUTS,
            max_nodes: MAX_GRAPH_NODES,
            max_edges: MAX_GRAPH_EDGES,
        }
    }

    pub const fn with_limits(max_inputs: usize, max_nodes: usize, max_edges: usize) -> Self {
        Self {
            max_inputs,
            max_nodes,
            max_edges,
        }
    }

    pub fn build(
        &self,
        kind: GraphKind,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphError> {
        let mut inputs = inputs.into_iter().collect::<Vec<_>>();
        if inputs.len() > self.max_inputs {
            return Err(GraphError::TooManyInputs);
        }
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        let mut paths = BTreeSet::new();
        for input in &inputs {
            if !paths.insert(input.path.clone()) {
                return Err(GraphError::DuplicateInput);
            }
        }

        let mut builder = GraphBuilder::new(kind, scope, self.max_nodes, self.max_edges);
        for input in &inputs {
            match kind {
                GraphKind::Code => builder.add_code(input)?,
                GraphKind::Knowledge => builder.add_knowledge(input)?,
                GraphKind::Review => builder.add_review(input)?,
                GraphKind::Architecture => builder.add_architecture(input)?,
            }
        }
        builder.finish(inputs.len())
    }

    pub fn code(
        &self,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphError> {
        self.build(GraphKind::Code, scope, inputs)
    }

    pub fn knowledge(
        &self,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphError> {
        self.build(GraphKind::Knowledge, scope, inputs)
    }

    pub fn review(
        &self,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphError> {
        self.build(GraphKind::Review, scope, inputs)
    }

    pub fn architecture(
        &self,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphError> {
        self.build(GraphKind::Architecture, scope, inputs)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphStoreError {
    Database,
    Io,
    Serialization,
    CorruptSnapshot,
    SnapshotTooLarge,
    LockPoisoned,
    Graph(GraphError),
}

impl fmt::Display for GraphStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database => formatter.write_str("graph store database is unavailable"),
            Self::Io => formatter.write_str("graph store filesystem operation failed"),
            Self::Serialization => formatter.write_str("graph snapshot serialization failed"),
            Self::CorruptSnapshot => formatter.write_str("graph snapshot is corrupt"),
            Self::SnapshotTooLarge => formatter.write_str("graph snapshot exceeds the size limit"),
            Self::LockPoisoned => formatter.write_str("graph store lock is unavailable"),
            Self::Graph(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for GraphStoreError {}

impl From<rusqlite::Error> for GraphStoreError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

impl From<std::io::Error> for GraphStoreError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

impl From<serde_json::Error> for GraphStoreError {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

impl From<GraphError> for GraphStoreError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

pub struct GraphStore {
    connection: Mutex<Connection>,
}

impl GraphStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GraphStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS graph_snapshots (
                 tenant TEXT NOT NULL,
                 workspace TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 digest TEXT NOT NULL,
                 snapshot_json TEXT NOT NULL,
                 PRIMARY KEY (tenant, workspace, kind)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn replace(&self, snapshot: &GraphSnapshot) -> Result<(), GraphStoreError> {
        validate_snapshot(snapshot)?;
        let snapshot_json = serde_json::to_string(snapshot)?;
        if snapshot_json.len() > MAX_GRAPH_SNAPSHOT_BYTES {
            return Err(GraphStoreError::SnapshotTooLarge);
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| GraphStoreError::LockPoisoned)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO graph_snapshots (tenant, workspace, kind, digest, snapshot_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (tenant, workspace, kind) DO UPDATE SET
                 digest = excluded.digest,
                 snapshot_json = excluded.snapshot_json",
            params![
                snapshot.scope.tenant,
                snapshot.scope.workspace,
                snapshot.kind.as_str(),
                snapshot.digest,
                snapshot_json,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn refresh(
        &self,
        engine: &GraphIntelligenceEngine,
        kind: GraphKind,
        scope: GraphScope,
        inputs: impl IntoIterator<Item = GraphInput>,
    ) -> Result<GraphSnapshot, GraphStoreError> {
        let snapshot = engine.build(kind, scope, inputs)?;
        self.replace(&snapshot)?;
        Ok(snapshot)
    }

    pub fn load(
        &self,
        kind: GraphKind,
        scope: &GraphScope,
    ) -> Result<Option<GraphSnapshot>, GraphStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| GraphStoreError::LockPoisoned)?;
        let mut statement = connection.prepare(
            "SELECT digest, snapshot_json FROM graph_snapshots
             WHERE tenant = ?1 AND workspace = ?2 AND kind = ?3",
        )?;
        let mut rows = statement.query(params![scope.tenant, scope.workspace, kind.as_str()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let digest: String = row.get(0)?;
        let snapshot_json: String = row.get(1)?;
        if snapshot_json.len() > MAX_GRAPH_SNAPSHOT_BYTES {
            return Err(GraphStoreError::SnapshotTooLarge);
        }
        let snapshot: GraphSnapshot = serde_json::from_str(&snapshot_json)?;
        if snapshot.kind != kind || snapshot.scope != *scope || snapshot.digest != digest {
            return Err(GraphStoreError::CorruptSnapshot);
        }
        validate_snapshot(&snapshot)?;
        Ok(Some(snapshot))
    }

    pub fn remove(&self, kind: GraphKind, scope: &GraphScope) -> Result<bool, GraphStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| GraphStoreError::LockPoisoned)?;
        let removed = connection.execute(
            "DELETE FROM graph_snapshots
             WHERE tenant = ?1 AND workspace = ?2 AND kind = ?3",
            params![scope.tenant, scope.workspace, kind.as_str()],
        )?;
        Ok(removed != 0)
    }
}

fn validate_snapshot(snapshot: &GraphSnapshot) -> Result<(), GraphStoreError> {
    if snapshot.scope.tenant.is_empty()
        || snapshot.scope.workspace.is_empty()
        || snapshot.scope.tenant.len() > 256
        || snapshot.scope.workspace.len() > 256
        || snapshot.scope.tenant.chars().any(char::is_control)
        || snapshot.scope.workspace.chars().any(char::is_control)
        || snapshot.source_count > MAX_GRAPH_INPUTS
        || snapshot.nodes.len() > MAX_GRAPH_NODES
        || snapshot.edges.len() > MAX_GRAPH_EDGES
    {
        return Err(GraphStoreError::CorruptSnapshot);
    }
    let mut node_ids = BTreeSet::new();
    for node in &snapshot.nodes {
        if !node_ids.insert(node.id.clone()) {
            return Err(GraphStoreError::CorruptSnapshot);
        }
    }
    let mut edges = BTreeSet::new();
    for edge in &snapshot.edges {
        if !edges.insert((edge.from.clone(), edge.to.clone(), edge.relation.clone())) {
            return Err(GraphStoreError::CorruptSnapshot);
        }
    }
    if snapshot.digest
        != snapshot_digest(
            snapshot.kind,
            &snapshot.scope,
            &snapshot.nodes,
            &snapshot.edges,
        )
    {
        return Err(GraphStoreError::CorruptSnapshot);
    }
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<(), GraphStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

struct GraphBuilder {
    kind: GraphKind,
    scope: GraphScope,
    max_nodes: usize,
    max_edges: usize,
    nodes: BTreeMap<String, GraphNode>,
    edges: BTreeSet<(String, String, String)>,
}

impl GraphBuilder {
    fn new(kind: GraphKind, scope: GraphScope, max_nodes: usize, max_edges: usize) -> Self {
        Self {
            kind,
            scope,
            max_nodes,
            max_edges,
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
        }
    }

    fn add_node(
        &mut self,
        id: String,
        kind: GraphNodeKind,
        label: String,
        source: &GraphInput,
    ) -> Result<(), GraphError> {
        if self.nodes.contains_key(&id) {
            return Ok(());
        }
        if self.nodes.len() >= self.max_nodes {
            return Err(GraphError::NodeLimit);
        }
        self.nodes.insert(
            id.clone(),
            GraphNode {
                id,
                kind,
                label,
                source: source.path.clone(),
                provenance_digest: digest(source.provenance.as_bytes()),
            },
        );
        Ok(())
    }

    fn add_edge(&mut self, from: &str, to: &str, relation: &str) -> Result<(), GraphError> {
        if self.edges.len() >= self.max_edges
            && !self
                .edges
                .contains(&(from.to_owned(), to.to_owned(), relation.to_owned()))
        {
            return Err(GraphError::EdgeLimit);
        }
        self.edges
            .insert((from.to_owned(), to.to_owned(), relation.to_owned()));
        Ok(())
    }

    fn add_file(&mut self, input: &GraphInput, kind: GraphNodeKind) -> Result<String, GraphError> {
        let id = format!("file:{}", input.path);
        self.add_node(id.clone(), kind, input.path.clone(), input)?;
        Ok(id)
    }

    fn add_code(&mut self, input: &GraphInput) -> Result<(), GraphError> {
        let file = self.add_file(input, GraphNodeKind::File)?;
        for line in input.content.lines() {
            let trimmed = line.trim();
            let target = trimmed
                .strip_prefix("use ")
                .or_else(|| trimmed.strip_prefix("mod "))
                .or_else(|| trimmed.strip_prefix("import "))
                .or_else(|| trimmed.strip_prefix("from "))
                .map(|value| value.split([' ', ';', ':', '{']).next().unwrap_or(value));
            let Some(target) = target.filter(|value| !value.is_empty()) else {
                continue;
            };
            let module = format!("module:{target}");
            self.add_node(
                module.clone(),
                GraphNodeKind::Module,
                target.to_owned(),
                input,
            )?;
            self.add_edge(&file, &module, "imports")?;
        }
        Ok(())
    }

    fn add_knowledge(&mut self, input: &GraphInput) -> Result<(), GraphError> {
        let document = self.add_file(input, GraphNodeKind::Document)?;
        for (line_number, line) in input.content.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(heading) = trimmed.strip_prefix('#') {
                let heading = heading.trim();
                if !heading.is_empty() {
                    let id = format!("heading:{}:{line_number}", input.path);
                    self.add_node(
                        id.clone(),
                        GraphNodeKind::Heading,
                        heading.to_owned(),
                        input,
                    )?;
                    self.add_edge(&document, &id, "contains")?;
                }
            }
            if let Some(target) = markdown_link_target(trimmed) {
                let linked = format!("document:{target}");
                self.add_node(
                    linked.clone(),
                    GraphNodeKind::Document,
                    target.to_owned(),
                    input,
                )?;
                self.add_edge(&document, &linked, "links_to")?;
            }
        }
        Ok(())
    }

    fn add_review(&mut self, input: &GraphInput) -> Result<(), GraphError> {
        let file = self.add_file(input, GraphNodeKind::File)?;
        for (line_number, line) in input.content.lines().enumerate() {
            let Some(marker) = ["TODO", "FIXME", "HACK", "XXX"]
                .iter()
                .find(|marker| line.contains(**marker))
            else {
                continue;
            };
            let id = format!("finding:{}:{}:{}", input.path, line_number, marker);
            self.add_node(
                id.clone(),
                GraphNodeKind::Finding,
                (*marker).to_owned(),
                input,
            )?;
            self.add_edge(&file, &id, "has_finding")?;
        }
        Ok(())
    }

    fn add_architecture(&mut self, input: &GraphInput) -> Result<(), GraphError> {
        let file = self.add_file(input, GraphNodeKind::File)?;
        let layer = input.path.split('/').next().unwrap_or("root");
        let layer_id = format!("layer:{layer}");
        self.add_node(
            layer_id.clone(),
            GraphNodeKind::Layer,
            layer.to_owned(),
            input,
        )?;
        self.add_edge(&file, &layer_id, "belongs_to")?;
        for line in input.content.lines() {
            let trimmed = line.trim();
            let target = trimmed
                .strip_prefix("use ")
                .or_else(|| trimmed.strip_prefix("import "))
                .or_else(|| trimmed.strip_prefix("from "))
                .map(|value| value.split([' ', ';', ':', '{']).next().unwrap_or(value));
            let Some(target) = target.filter(|value| !value.is_empty()) else {
                continue;
            };
            let module = format!("module:{target}");
            self.add_node(
                module.clone(),
                GraphNodeKind::Module,
                target.to_owned(),
                input,
            )?;
            self.add_edge(&file, &module, "depends_on")?;
        }
        Ok(())
    }

    fn finish(self, source_count: usize) -> Result<GraphSnapshot, GraphError> {
        let nodes = self.nodes.into_values().collect::<Vec<_>>();
        let edges = self
            .edges
            .into_iter()
            .map(|(from, to, relation)| GraphEdge { from, to, relation })
            .collect::<Vec<_>>();
        let digest = snapshot_digest(self.kind, &self.scope, &nodes, &edges);
        Ok(GraphSnapshot {
            kind: self.kind,
            scope: self.scope,
            source_count,
            nodes,
            edges,
            digest,
        })
    }
}

fn markdown_link_target(line: &str) -> Option<&str> {
    let end_label = line.find("](")?;
    let rest = &line[end_label + 2..];
    let end_target = rest.find(')')?;
    let target = &rest[..end_target];
    (!target.is_empty() && !target.chars().any(char::is_control)).then_some(target)
}

fn validate_path(value: String) -> Result<String, GraphError> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(GraphError::InvalidPath);
    }
    let value = value.replace('\\', "/");
    let path = std::path::Path::new(&value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(GraphError::InvalidPath);
    }
    Ok(value)
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, GraphError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(GraphError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(GraphError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(GraphError::FieldTooLong(field));
    }
    Ok(value)
}

fn snapshot_digest(
    kind: GraphKind,
    scope: &GraphScope,
    nodes: &[GraphNode],
    edges: &[GraphEdge],
) -> String {
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, "pandora.graph.v1");
    digest_text(&mut hasher, kind.as_str());
    digest_text(&mut hasher, &scope.tenant);
    digest_text(&mut hasher, &scope.workspace);
    for node in nodes {
        digest_text(&mut hasher, &node.id);
        digest_text(&mut hasher, node.kind_as_str());
        digest_text(&mut hasher, &node.label);
        digest_text(&mut hasher, &node.source);
        digest_text(&mut hasher, &node.provenance_digest);
    }
    for edge in edges {
        digest_text(&mut hasher, &edge.from);
        digest_text(&mut hasher, &edge.to);
        digest_text(&mut hasher, &edge.relation);
    }
    format!("sha256:{}", encode_hex(hasher.finalize().as_slice()))
}

impl GraphNode {
    fn kind_as_str(&self) -> &'static str {
        match self.kind {
            GraphNodeKind::File => "file",
            GraphNodeKind::Module => "module",
            GraphNodeKind::Document => "document",
            GraphNodeKind::Heading => "heading",
            GraphNodeKind::Finding => "finding",
            GraphNodeKind::Layer => "layer",
        }
    }
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", encode_hex(Sha256::digest(bytes).as_slice()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> GraphScope {
        GraphScope::new("tenant-1", "workspace-1").unwrap()
    }

    fn input(path: &str, content: &str) -> GraphInput {
        GraphInput::new(path, content, "session:execution-1").unwrap()
    }

    #[test]
    fn code_graph_is_deterministic_and_provenance_bound() {
        let engine = GraphIntelligenceEngine::new();
        let first = engine
            .code(
                scope(),
                [input("src/lib.rs", "use crate::module;\nmod module;")],
            )
            .unwrap();
        let second = engine
            .code(
                scope(),
                [input("src/lib.rs", "use crate::module;\nmod module;")],
            )
            .unwrap();

        assert_eq!(first.digest(), second.digest());
        assert!(first.nodes().iter().any(|node| node.id() == "module:crate"));
        assert!(
            first
                .edges()
                .iter()
                .any(|edge| edge.relation() == "imports")
        );
        assert!(
            first
                .nodes()
                .iter()
                .all(|node| node.provenance_digest().starts_with("sha256:"))
        );
    }

    #[test]
    fn knowledge_graph_tracks_headings_and_links() {
        let snapshot = GraphIntelligenceEngine::new()
            .knowledge(
                scope(),
                [input("docs/guide.md", "# Guide\nSee [API](api.md).")],
            )
            .unwrap();

        assert!(
            snapshot
                .nodes()
                .iter()
                .any(|node| node.kind() == GraphNodeKind::Heading)
        );
        assert!(
            snapshot
                .edges()
                .iter()
                .any(|edge| edge.relation() == "links_to")
        );
    }

    #[test]
    fn review_graph_records_bounded_findings() {
        let snapshot = GraphIntelligenceEngine::new()
            .review(
                scope(),
                [input("src/lib.rs", "// TODO: verify\n// FIXME: fix")],
            )
            .unwrap();

        assert_eq!(snapshot.source_count(), 1);
        assert_eq!(
            snapshot
                .nodes()
                .iter()
                .filter(|node| node.kind() == GraphNodeKind::Finding)
                .count(),
            2
        );
    }

    #[test]
    fn architecture_graph_groups_files_into_layers() {
        let snapshot = GraphIntelligenceEngine::new()
            .architecture(
                scope(),
                [input("crates/runtime/src/lib.rs", "use types::Effect;")],
            )
            .unwrap();

        assert!(
            snapshot
                .nodes()
                .iter()
                .any(|node| node.id() == "layer:crates")
        );
        assert!(
            snapshot
                .edges()
                .iter()
                .any(|edge| edge.relation() == "belongs_to")
        );
    }

    #[test]
    fn unsafe_or_duplicate_inputs_fail_closed() {
        assert_eq!(
            GraphInput::new("../outside", "content", "source"),
            Err(GraphError::InvalidPath)
        );
        let engine = GraphIntelligenceEngine::new();
        assert_eq!(
            engine.code(
                scope(),
                [input("src/lib.rs", "one"), input("src/lib.rs", "two")]
            ),
            Err(GraphError::DuplicateInput)
        );
    }

    #[test]
    fn graph_limits_are_enforced() {
        let engine = GraphIntelligenceEngine::with_limits(1, 1, 1);
        let error = engine
            .code(scope(), [input("a.rs", "use a;"), input("b.rs", "use b;")])
            .unwrap_err();
        assert_eq!(error, GraphError::TooManyInputs);
    }

    #[test]
    fn graph_store_reloads_and_replaces_stale_scope_data() {
        let root = crate::test_support::new_temp_dir("pandora-graph-store").unwrap();
        let store = GraphStore::open(root.join("graphs.sqlite3")).unwrap();
        let engine = GraphIntelligenceEngine::new();
        let first = engine
            .code(scope(), [input("src/old.rs", "use crate::old_module;")])
            .unwrap();
        store.replace(&first).unwrap();

        assert_eq!(
            store
                .load(GraphKind::Code, &scope())
                .unwrap()
                .unwrap()
                .digest(),
            first.digest()
        );

        let second = engine
            .code(scope(), [input("src/new.rs", "fn main() {}")])
            .unwrap();
        store.replace(&second).unwrap();
        let loaded = store.load(GraphKind::Code, &scope()).unwrap().unwrap();

        assert_eq!(loaded, second);
        assert!(loaded.nodes().iter().all(|node| !node.id().contains("old")));
        assert!(store.remove(GraphKind::Code, &scope()).unwrap());
        assert!(store.load(GraphKind::Code, &scope()).unwrap().is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn graph_store_rejects_corrupted_persisted_digest() {
        let root = crate::test_support::new_temp_dir("pandora-graph-store-corrupt").unwrap();
        let path = root.join("graphs.sqlite3");
        let store = GraphStore::open(&path).unwrap();
        let snapshot = GraphIntelligenceEngine::new()
            .code(scope(), [input("src/lib.rs", "fn main() {}")])
            .unwrap();
        store.replace(&snapshot).unwrap();
        drop(store);

        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute("UPDATE graph_snapshots SET digest = 'sha256:corrupted'", [])
            .unwrap();
        drop(connection);

        let store = GraphStore::open(&path).unwrap();
        assert_eq!(
            store.load(GraphKind::Code, &scope()),
            Err(GraphStoreError::CorruptSnapshot)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_graph_refresh_does_not_replace_existing_snapshot() {
        let root = crate::test_support::new_temp_dir("pandora-graph-store-refresh").unwrap();
        let store = GraphStore::open(root.join("graphs.sqlite3")).unwrap();
        let engine = GraphIntelligenceEngine::new();
        let first = engine
            .code(scope(), [input("src/lib.rs", "fn main() {}")])
            .unwrap();
        store.replace(&first).unwrap();

        let error = store.refresh(
            &engine,
            GraphKind::Code,
            scope(),
            [input("src/lib.rs", "one"), input("src/lib.rs", "two")],
        );
        assert_eq!(
            error,
            Err(GraphStoreError::Graph(GraphError::DuplicateInput))
        );
        assert_eq!(store.load(GraphKind::Code, &scope()).unwrap(), Some(first));

        let _ = std::fs::remove_dir_all(root);
    }
}
