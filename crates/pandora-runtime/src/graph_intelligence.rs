use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_GRAPH_INPUTS: usize = 2_048;
pub const MAX_GRAPH_INPUT_BYTES: usize = 1_048_576;
pub const MAX_GRAPH_NODES: usize = 8_192;
pub const MAX_GRAPH_EDGES: usize = 16_384;

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
}
