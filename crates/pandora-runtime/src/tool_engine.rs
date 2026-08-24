use pandora_types::{
    ArtifactId, Capability, EffectTarget, ExecutionId, ExecutionProfile, GeneId, HarnessId,
    Operation, OperationRequest, PrincipalId, RequestError, ResourceScope, SessionId, TaskIntent,
};
use serde_json::Value;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolError {
    UnknownTool(String),
    UnsupportedTool(String),
    DuplicateTool,
    InvalidSchema(String),
    InvalidArguments(String),
    IdempotencyConflict,
    InvalidIdempotencyKey,
    Request(RequestError),
}

impl ToolError {
    pub(crate) fn agent_message(&self) -> String {
        match self {
            Self::UnknownTool(tool_id) => {
                format!("tool error: unknown tool '{}'", bounded_text(tool_id, 128))
            }
            Self::UnsupportedTool(tool_id) => {
                format!(
                    "tool error: unsupported tool '{}'",
                    bounded_text(tool_id, 128)
                )
            }
            Self::InvalidArguments(reason) => format!(
                "tool error: invalid arguments: {}",
                bounded_text(reason, 256)
            ),
            Self::InvalidSchema(_) => "tool error: tool schema is invalid".to_owned(),
            Self::DuplicateTool => "tool error: tool registration conflict".to_owned(),
            Self::IdempotencyConflict => "tool error: idempotency conflict".to_owned(),
            Self::InvalidIdempotencyKey => "tool error: invalid idempotency key".to_owned(),
            Self::Request(_) => "tool error: tool request was rejected".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDefinition {
    id: GeneId,
    version: String,
    name: String,
    input_schema: Value,
    capability: Capability,
    operation: Operation,
}

impl ToolDefinition {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        name: impl Into<String>,
        input_schema: Value,
        capability: Capability,
        operation: Operation,
    ) -> Result<Self, ToolError> {
        let id =
            GeneId::new(id.into()).map_err(|error| ToolError::InvalidSchema(error.to_string()))?;
        let version = validate_text("tool version", version.into())?;
        let name = validate_text("tool name", name.into())?;
        validate_schema(&input_schema)?;
        Ok(Self {
            id,
            version,
            name,
            input_schema,
            capability,
            operation,
        })
    }

    pub fn id(&self) -> &GeneId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    pub const fn capability(&self) -> Capability {
        self.capability
    }

    pub const fn operation(&self) -> Operation {
        self.operation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolContext {
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    execution_profile: ExecutionProfile,
    artifact_id: Option<ArtifactId>,
}

impl ToolContext {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        execution_profile: ExecutionProfile,
        artifact_id: Option<ArtifactId>,
    ) -> Self {
        Self {
            execution_id,
            session_id,
            principal_id,
            execution_profile,
            artifact_id,
        }
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn execution_profile(&self) -> &ExecutionProfile {
        &self.execution_profile
    }

    pub fn artifact_id(&self) -> Option<&ArtifactId> {
        self.artifact_id.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolPlan {
    tool_id: GeneId,
    idempotency_key: String,
    arguments: Value,
    request: OperationRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocation {
    tool_id: GeneId,
    task: TaskIntent,
}

impl ToolInvocation {
    pub fn tool_id(&self) -> &GeneId {
        &self.tool_id
    }

    pub fn task(&self) -> &TaskIntent {
        &self.task
    }
}

impl ToolPlan {
    pub fn tool_id(&self) -> &GeneId {
        &self.tool_id
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub fn request(&self) -> &OperationRequest {
        &self.request
    }
}

#[derive(Clone)]
pub struct ToolEngine {
    definitions: Arc<Mutex<HashMap<GeneId, ToolDefinition>>>,
    idempotent_plans: Arc<Mutex<HashMap<String, ToolPlan>>>,
}

impl ToolEngine {
    pub fn new() -> Self {
        Self {
            definitions: Arc::new(Mutex::new(HashMap::new())),
            idempotent_plans: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_builtins() -> Self {
        let engine = Self::new();
        engine
            .register(
                ToolDefinition::new(
                    "workspace.read",
                    "1.0.0",
                    "Read a workspace file",
                    json!({
                        "type": "object",
                        "required": ["path"],
                        "properties": {"path": {"type": "string"}},
                        "additionalProperties": false
                    }),
                    Capability::FilesystemRead,
                    Operation::Read,
                )
                .expect("built-in read tool schema is valid"),
            )
            .expect("built-in read tool ID is unique");
        engine
            .register(
                ToolDefinition::new(
                    "workspace.search",
                    "1.0.0",
                    "Search workspace files",
                    json!({
                        "type": "object",
                        "required": ["query"],
                        "properties": {"query": {"type": "string"}},
                        "additionalProperties": false
                    }),
                    Capability::FilesystemRead,
                    Operation::Read,
                )
                .expect("built-in search tool schema is valid"),
            )
            .expect("built-in search tool ID is unique");
        engine
            .register(
                ToolDefinition::new(
                    "workspace.patch",
                    "1.0.0",
                    "Propose a workspace patch",
                    json!({
                        "type": "object",
                        "required": ["path", "content"],
                        "properties": {
                            "path": {"type": "string"},
                            "content": {"type": "string"}
                        },
                        "additionalProperties": false
                    }),
                    Capability::FilesystemWrite,
                    Operation::Write,
                )
                .expect("built-in patch tool schema is valid"),
            )
            .expect("built-in patch tool ID is unique");
        engine
            .register(
                ToolDefinition::new(
                    "workspace.verify",
                    "1.0.0",
                    "Run the fixed workspace verification command",
                    json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }),
                    Capability::ProcessExecute,
                    Operation::Execute,
                )
                .expect("built-in verification tool schema is valid"),
            )
            .expect("built-in verification tool ID is unique");
        for definition in [
            ToolDefinition::new(
                "daedalus.audit",
                "0.1.0",
                "Inventory the workspace for an evidence-led audit",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "argus.review",
                "0.1.0",
                "Review one workspace file",
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "ariadne.debt",
                "0.1.0",
                "Find explicit technical-debt markers",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "hephaestus.measure",
                "0.1.0",
                "Run the fixed repository verifier",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::ProcessExecute,
                Operation::Execute,
            ),
        ] {
            engine
                .register(definition.expect("built-in workflow tool schema is valid"))
                .expect("built-in workflow tool ID is unique");
        }
        for definition in [
            ToolDefinition::new(
                "evidence.inventory",
                "0.1.0",
                "Inventory files available in the bounded workspace",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "evidence.search",
                "0.1.0",
                "Search bounded workspace evidence",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {"query": {"type": "string"}},
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "source.read",
                "0.1.0",
                "Read one bounded workspace source",
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "source.compare",
                "0.1.0",
                "Read two bounded workspace sources for comparison",
                json!({
                    "type": "object",
                    "required": ["left", "right"],
                    "properties": {
                        "left": {"type": "string"},
                        "right": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "citation.inventory",
                "0.1.0",
                "Find explicit URL and DOI markers in workspace sources",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
        ] {
            engine
                .register(definition.expect("built-in research tool schema is valid"))
                .expect("built-in research tool ID is unique");
        }
        for definition in [
            ToolDefinition::new(
                "design.inventory",
                "0.1.0",
                "Inventory files available for bounded design analysis",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "design.tokens",
                "0.1.0",
                "Find explicit design-token markers in workspace sources",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "design.inspect",
                "0.1.0",
                "Read one bounded design source",
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "design.compare",
                "0.1.0",
                "Read two bounded design sources for comparison",
                json!({
                    "type": "object",
                    "required": ["left", "right"],
                    "properties": {
                        "left": {"type": "string"},
                        "right": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                Capability::FilesystemRead,
                Operation::Read,
            ),
            ToolDefinition::new(
                "accessibility.evidence",
                "0.1.0",
                "Inventory common accessibility markers without claiming conformance",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
                Capability::FilesystemRead,
                Operation::Read,
            ),
        ] {
            engine
                .register(definition.expect("built-in design tool schema is valid"))
                .expect("built-in design tool ID is unique");
        }
        engine
    }

    pub fn register(&self, definition: ToolDefinition) -> Result<(), ToolError> {
        let mut definitions = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if definitions.contains_key(definition.id()) {
            return Err(ToolError::DuplicateTool);
        }
        definitions.insert(definition.id().clone(), definition);
        Ok(())
    }

    pub fn register_batch(&self, definitions: Vec<ToolDefinition>) -> Result<(), ToolError> {
        let mut registered = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut batch_ids = HashSet::with_capacity(definitions.len());
        if definitions.iter().any(|definition| {
            registered.contains_key(definition.id()) || !batch_ids.insert(definition.id().clone())
        }) {
            return Err(ToolError::DuplicateTool);
        }
        for definition in definitions {
            registered.insert(definition.id().clone(), definition);
        }
        Ok(())
    }

    pub(crate) fn unregister_batch(&self, tool_ids: &[GeneId]) {
        let tool_ids = tool_ids.iter().collect::<HashSet<_>>();
        let mut definitions = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        definitions.retain(|tool_id, _| !tool_ids.contains(tool_id));
        drop(definitions);
        let mut plans = self
            .idempotent_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        plans.retain(|_, plan| !tool_ids.contains(plan.tool_id()));
    }

    pub fn list(&self) -> Vec<ToolDefinition> {
        let definitions = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut values = definitions.values().cloned().collect::<Vec<_>>();
        values.sort_by(|left, right| left.id().cmp(right.id()));
        values
    }

    pub(crate) fn list_for_genes(&self, gene_ids: &[GeneId]) -> Vec<ToolDefinition> {
        let gene_ids = gene_ids.iter().cloned().collect::<HashSet<_>>();
        self.list()
            .into_iter()
            .filter(|definition| {
                gene_id_for_tool(definition.id().as_str())
                    .is_ok_and(|gene_id| gene_ids.contains(&gene_id))
            })
            .collect()
    }

    pub fn validate_call(&self, tool_id: &str, arguments: &Value) -> Result<(), ToolError> {
        let definition = self.definition(tool_id)?;
        validate_arguments(definition.input_schema(), arguments)
    }

    pub fn prepare_invocation(
        &self,
        tool_id: &str,
        arguments: &Value,
    ) -> Result<ToolInvocation, ToolError> {
        let harness_id = if is_research_tool(tool_id) {
            pandora_harnesses::RESEARCH_HARNESS_ID
        } else if is_design_tool(tool_id) {
            pandora_harnesses::DESIGN_HARNESS_ID
        } else {
            pandora_harnesses::CODING_HARNESS_ID
        };
        let harness = HarnessId::new(harness_id).expect("built-in Harness ID is valid");
        self.prepare_invocation_for_harness(tool_id, arguments, &harness)
    }

    pub fn prepare_invocation_for_harness(
        &self,
        tool_id: &str,
        arguments: &Value,
        harness: &HarnessId,
    ) -> Result<ToolInvocation, ToolError> {
        let definition = self.definition(tool_id)?;
        validate_arguments(definition.input_schema(), arguments)?;
        let task = match definition.id().as_str() {
            "workspace.read" => task_from_argument(arguments, "read", "path")?,
            "workspace.search" => task_from_argument(arguments, "search", "query")?,
            "workspace.patch" => {
                let path = required_text_argument(arguments, "path")?;
                let content = required_text_argument(arguments, "content")?;
                TaskIntent::new(format!("patch:{path}:{content}"))
                    .map_err(|error| ToolError::InvalidArguments(error.to_string()))?
            }
            "workspace.verify" => TaskIntent::new("verify")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "daedalus.audit" => TaskIntent::new("audit")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "argus.review" => task_from_argument(arguments, "deep-review", "path")?,
            "ariadne.debt" => TaskIntent::new("debt")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "hephaestus.measure" => TaskIntent::new("measure")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "evidence.inventory" => TaskIntent::new("evidence-inventory")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "evidence.search" => task_from_argument(arguments, "evidence-search", "query")?,
            "source.read" => task_from_argument(arguments, "source-read", "path")?,
            "source.compare" => {
                let left = required_text_argument(arguments, "left")?;
                let right = required_text_argument(arguments, "right")?;
                TaskIntent::new(format!("source-compare:{left}|{right}"))
                    .map_err(|error| ToolError::InvalidArguments(error.to_string()))?
            }
            "citation.inventory" => TaskIntent::new("citation-inventory")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "design.inventory" => TaskIntent::new("design-inventory")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "design.tokens" => TaskIntent::new("design-tokens")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            "design.inspect" => task_from_argument(arguments, "design-inspect", "path")?,
            "design.compare" => {
                let left = required_text_argument(arguments, "left")?;
                let right = required_text_argument(arguments, "right")?;
                TaskIntent::new(format!("design-compare:{left}|{right}"))
                    .map_err(|error| ToolError::InvalidArguments(error.to_string()))?
            }
            "accessibility.evidence" => TaskIntent::new("accessibility-evidence")
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?,
            _ => {
                return Err(ToolError::UnsupportedTool(
                    definition.id().as_str().to_owned(),
                ));
            }
        };
        let gene_id = gene_id_for_tool(definition.id().as_str())?;
        let task = task.with_harness(harness.clone()).with_gene(gene_id);
        Ok(ToolInvocation {
            tool_id: definition.id().clone(),
            task,
        })
    }

    pub fn plan(
        &self,
        tool_id: &str,
        context: &ToolContext,
        arguments: Value,
        idempotency_key: &str,
        target: EffectTarget,
        resource_scope: ResourceScope,
    ) -> Result<ToolPlan, ToolError> {
        let payload = serde_json::to_vec(&arguments)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        self.plan_with_payload(
            tool_id,
            context,
            arguments,
            idempotency_key,
            target,
            resource_scope,
            &payload,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn plan_with_payload(
        &self,
        tool_id: &str,
        context: &ToolContext,
        arguments: Value,
        idempotency_key: &str,
        target: EffectTarget,
        resource_scope: ResourceScope,
        authorization_payload: &[u8],
    ) -> Result<ToolPlan, ToolError> {
        let definition = {
            let definitions = self
                .definitions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let id = GeneId::new(tool_id.to_owned())
                .map_err(|_| ToolError::UnknownTool(tool_id.to_owned()))?;
            definitions
                .get(&id)
                .cloned()
                .ok_or_else(|| ToolError::UnknownTool(tool_id.to_owned()))?
        };
        validate_idempotency_key(idempotency_key)?;
        validate_arguments(definition.input_schema(), &arguments)?;
        let request = OperationRequest::new(
            context.execution_id().clone(),
            context.session_id().clone(),
            context.principal_id().clone(),
            context.execution_profile().clone(),
            definition.id().clone(),
            context.artifact_id().cloned(),
            definition.capability(),
            definition.operation(),
            target,
            resource_scope,
        )
        .map_err(ToolError::Request)?
        .with_payload_digest(authorization_payload)
        .map_err(ToolError::Request)?;
        let plan = ToolPlan {
            tool_id: definition.id().clone(),
            idempotency_key: idempotency_key.to_owned(),
            arguments,
            request,
        };
        let mut plans = self
            .idempotent_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = plans.get(idempotency_key) {
            if existing == &plan {
                return Ok(existing.clone());
            }
            return Err(ToolError::IdempotencyConflict);
        }
        plans.insert(idempotency_key.to_owned(), plan.clone());
        Ok(plan)
    }

    fn definition(&self, tool_id: &str) -> Result<ToolDefinition, ToolError> {
        let id = GeneId::new(tool_id.to_owned())
            .map_err(|_| ToolError::UnknownTool(tool_id.to_owned()))?;
        let definitions = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        definitions
            .get(&id)
            .cloned()
            .ok_or_else(|| ToolError::UnknownTool(tool_id.to_owned()))
    }
}

fn gene_id_for_tool(tool_id: &str) -> Result<GeneId, ToolError> {
    let gene_id = match tool_id {
        "workspace.read" => "workspace.read",
        "workspace.search" => "workspace.search",
        "workspace.patch" => "patch.apply",
        "workspace.verify" => "verification.run",
        "daedalus.audit" => "daedalus.audit",
        "argus.review" => "argus.review",
        "ariadne.debt" => "ariadne.debt",
        "hephaestus.measure" => "hephaestus.measure",
        "evidence.inventory" => "evidence.inventory",
        "evidence.search" => "evidence.search",
        "source.read" => "source.read",
        "source.compare" => "source.compare",
        "citation.inventory" => "citation.inventory",
        "design.inventory" => "design.inventory",
        "design.tokens" => "design.tokens",
        "design.inspect" => "design.inspect",
        "design.compare" => "design.compare",
        "accessibility.evidence" => "accessibility.evidence",
        unknown => return Err(ToolError::UnsupportedTool(unknown.to_owned())),
    };
    Ok(GeneId::new(gene_id).expect("built-in Gene ID is valid"))
}

fn is_research_tool(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "evidence.inventory"
            | "evidence.search"
            | "source.read"
            | "source.compare"
            | "citation.inventory"
    )
}

fn is_design_tool(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "design.inventory"
            | "design.tokens"
            | "design.inspect"
            | "design.compare"
            | "accessibility.evidence"
    )
}

impl Default for ToolEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_schema(schema: &Value) -> Result<(), ToolError> {
    let Some(object) = schema.as_object() else {
        return Err(ToolError::InvalidSchema(
            "schema must be an object".to_owned(),
        ));
    };
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(ToolError::InvalidSchema(
            "schema type must be object".to_owned(),
        ));
    }
    if let Some(properties) = object.get("properties")
        && !properties.is_object()
    {
        return Err(ToolError::InvalidSchema(
            "schema properties must be an object".to_owned(),
        ));
    }
    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return Err(ToolError::InvalidSchema(
                "schema required must be an array".to_owned(),
            ));
        };
        if required.iter().any(|value| value.as_str().is_none()) {
            return Err(ToolError::InvalidSchema(
                "schema required entries must be strings".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_arguments(schema: &Value, arguments: &Value) -> Result<(), ToolError> {
    let Some(arguments) = arguments.as_object() else {
        return Err(ToolError::InvalidArguments(
            "tool arguments must be an object".to_owned(),
        ));
    };
    let schema = schema
        .as_object()
        .expect("validated tool schema is an object");
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !arguments.contains_key(name) {
                return Err(ToolError::InvalidArguments(format!(
                    "missing required argument '{name}'"
                )));
            }
        }
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let additional_properties = schema
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    for (name, value) in arguments {
        let Some(property) = properties.get(name) else {
            if !additional_properties {
                return Err(ToolError::InvalidArguments(format!(
                    "unknown argument '{name}'"
                )));
            }
            continue;
        };
        if let Some(expected) = property.get("type").and_then(Value::as_str)
            && !matches_type(value, expected)
        {
            return Err(ToolError::InvalidArguments(format!(
                "argument '{name}' must be {expected}"
            )));
        }
    }
    Ok(())
}

fn task_from_argument(
    arguments: &Value,
    action: &str,
    name: &str,
) -> Result<TaskIntent, ToolError> {
    let value = required_text_argument(arguments, name)?;
    TaskIntent::new(format!("{action}:{value}"))
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))
}

fn required_text_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    let value = arguments.get(name).and_then(Value::as_str).ok_or_else(|| {
        ToolError::InvalidArguments(format!("argument '{name}' must be a string"))
    })?;
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(ToolError::InvalidArguments(format!(
            "argument '{name}' is invalid"
        )));
    }
    Ok(value)
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn validate_idempotency_key(value: &str) -> Result<(), ToolError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(ToolError::InvalidIdempotencyKey);
    }
    Ok(())
}

fn validate_text(field: &'static str, value: String) -> Result<String, ToolError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ToolError::InvalidSchema(format!("{field} cannot be empty")));
    }
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(ToolError::InvalidSchema(format!("{field} is invalid")));
    }
    Ok(value)
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars + 1).collect::<String>();
    if result.chars().count() > max_chars {
        result.pop();
        result.push('…');
    }
    result
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{ExecutionProfileBinding, ExecutionProfileBindingKind, hash_artifact};
    use serde_json::json;

    fn definition() -> ToolDefinition {
        ToolDefinition::new(
            "workspace.read",
            "1.0.0",
            "Read workspace file",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {"path": {"type": "string"}},
                "additionalProperties": false
            }),
            Capability::FilesystemRead,
            Operation::Read,
        )
        .unwrap()
    }

    fn mcp_definition(id: &str) -> ToolDefinition {
        ToolDefinition::new(
            id,
            "2026-07-28",
            id,
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            Capability::McpInvoke,
            Operation::Invoke,
        )
        .unwrap()
    }

    fn context() -> ToolContext {
        ToolContext::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            ExecutionProfile::new(
                "2.0.0-alpha.6",
                "windows",
                "x86_64",
                1,
                "workspace-1",
                hash_artifact(b"containment"),
                vec![
                    ExecutionProfileBinding::new(
                        ExecutionProfileBindingKind::Executor,
                        "filesystem",
                        Some("2.0.0-alpha.6"),
                        hash_artifact(b"filesystem"),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
            Some(ArtifactId::new("artifact-1").unwrap()),
        )
    }

    fn plan(engine: &ToolEngine, key: &str, arguments: Value) -> Result<ToolPlan, ToolError> {
        engine.plan(
            "workspace.read",
            &context(),
            arguments,
            key,
            EffectTarget::path("README.md"),
            ResourceScope::workspace("workspace-1"),
        )
    }

    #[test]
    fn schema_rejects_missing_and_unknown_arguments() {
        let engine = ToolEngine::new();
        engine.register(definition()).unwrap();

        assert!(matches!(
            plan(&engine, "key-1", json!({})),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            plan(
                &engine,
                "key-2",
                json!({"path": "README.md", "extra": true})
            ),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn unknown_tools_fail_closed() {
        let engine = ToolEngine::new();

        assert_eq!(
            plan(&engine, "key-1", json!({"path": "README.md"})),
            Err(ToolError::UnknownTool("workspace.read".to_owned()))
        );
    }

    #[test]
    fn batch_registration_is_atomic_on_collision() {
        let engine = ToolEngine::new();
        engine
            .register(mcp_definition("mcp.local.existing"))
            .unwrap();

        assert_eq!(
            engine.register_batch(vec![
                mcp_definition("mcp.local.fresh"),
                mcp_definition("mcp.local.existing"),
            ]),
            Err(ToolError::DuplicateTool)
        );
        assert_eq!(
            engine
                .list()
                .into_iter()
                .map(|tool| tool.id().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["mcp.local.existing"]
        );
    }

    #[test]
    fn batch_registration_is_shared_and_removal_is_scoped() {
        let engine = ToolEngine::new();
        engine.register(definition()).unwrap();
        let shared = engine.clone();
        let imported = vec![
            mcp_definition("mcp.local.first"),
            mcp_definition("mcp.local.second"),
        ];
        let imported_ids = imported
            .iter()
            .map(|tool| tool.id().clone())
            .collect::<Vec<_>>();

        shared.register_batch(imported).unwrap();
        assert!(
            engine
                .list()
                .iter()
                .any(|tool| tool.id().as_str() == "mcp.local.first")
        );

        shared.unregister_batch(&imported_ids);
        assert_eq!(
            engine
                .list()
                .into_iter()
                .map(|tool| tool.id().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["workspace.read"]
        );
    }

    #[test]
    fn idempotency_makes_exact_retries_safe() {
        let engine = ToolEngine::new();
        engine.register(definition()).unwrap();
        let first = plan(&engine, "key-1", json!({"path": "README.md"})).unwrap();

        assert_eq!(
            plan(&engine, "key-1", json!({"path": "README.md"})),
            Ok(first.clone())
        );
        assert_eq!(
            plan(&engine, "key-1", json!({"path": "src/lib.rs"})),
            Err(ToolError::IdempotencyConflict)
        );
        assert_eq!(first.request().capability(), Capability::FilesystemRead);
    }

    #[test]
    fn plans_carry_scope_and_never_execute_scripts() {
        let engine = ToolEngine::with_builtins();
        let result = engine
            .plan(
                "workspace.read",
                &context(),
                json!({"path": "README.md"}),
                "key-1",
                EffectTarget::path("README.md"),
                ResourceScope::workspace("workspace-1"),
            )
            .unwrap();

        assert_eq!(
            result.request().resource_scope(),
            &ResourceScope::workspace("workspace-1")
        );
        assert!(
            engine
                .list()
                .iter()
                .any(|tool| tool.id().as_str() == "workspace.read")
        );
    }

    #[test]
    fn prepares_each_builtin_as_a_canonical_task() {
        let engine = ToolEngine::with_builtins();

        assert_eq!(
            engine
                .prepare_invocation("workspace.read", &json!({"path": "README.md"}))
                .unwrap()
                .task()
                .summary(),
            "read:README.md"
        );
        assert_eq!(
            engine
                .prepare_invocation("workspace.search", &json!({"query": "needle"}))
                .unwrap()
                .task()
                .summary(),
            "search:needle"
        );
        assert_eq!(
            engine
                .prepare_invocation(
                    "workspace.patch",
                    &json!({"path": "README.md", "content": "updated"})
                )
                .unwrap()
                .task()
                .summary(),
            "patch:README.md:updated"
        );
        assert_eq!(
            engine
                .prepare_invocation("workspace.verify", &json!({}))
                .unwrap()
                .task()
                .summary(),
            "verify"
        );
        for (tool, arguments, expected) in [
            ("daedalus.audit", json!({}), "audit"),
            (
                "argus.review",
                json!({"path": "src/lib.rs"}),
                "deep-review:src/lib.rs",
            ),
            ("ariadne.debt", json!({}), "debt"),
            ("hephaestus.measure", json!({}), "measure"),
            ("evidence.inventory", json!({}), "evidence-inventory"),
            (
                "evidence.search",
                json!({"query": "governance"}),
                "evidence-search:governance",
            ),
            (
                "source.read",
                json!({"path": "docs/source.md"}),
                "source-read:docs/source.md",
            ),
            (
                "source.compare",
                json!({"left": "first.md", "right": "second.md"}),
                "source-compare:first.md|second.md",
            ),
            ("citation.inventory", json!({}), "citation-inventory"),
            ("design.inventory", json!({}), "design-inventory"),
            ("design.tokens", json!({}), "design-tokens"),
            (
                "design.inspect",
                json!({"path": "ui/theme.css"}),
                "design-inspect:ui/theme.css",
            ),
            (
                "design.compare",
                json!({"left": "first.css", "right": "second.css"}),
                "design-compare:first.css|second.css",
            ),
            (
                "accessibility.evidence",
                json!({}),
                "accessibility-evidence",
            ),
        ] {
            assert_eq!(
                engine
                    .prepare_invocation(tool, &arguments)
                    .unwrap()
                    .task()
                    .summary(),
                expected
            );
        }
        assert_eq!(
            engine
                .prepare_invocation("design.inventory", &json!({}))
                .unwrap()
                .task()
                .requested_harness()
                .unwrap()
                .as_str(),
            "design-domain"
        );
        for (tool, arguments, expected_gene) in [
            (
                "workspace.read",
                json!({"path": "README.md"}),
                "workspace.read",
            ),
            (
                "workspace.search",
                json!({"query": "needle"}),
                "workspace.search",
            ),
            (
                "workspace.patch",
                json!({"path": "README.md", "content": "updated"}),
                "patch.apply",
            ),
            ("workspace.verify", json!({}), "verification.run"),
            ("daedalus.audit", json!({}), "daedalus.audit"),
            (
                "argus.review",
                json!({"path": "src/lib.rs"}),
                "argus.review",
            ),
            ("ariadne.debt", json!({}), "ariadne.debt"),
            ("hephaestus.measure", json!({}), "hephaestus.measure"),
        ] {
            assert_eq!(
                engine
                    .prepare_invocation(tool, &arguments)
                    .unwrap()
                    .task()
                    .requested_gene()
                    .unwrap()
                    .as_str(),
                expected_gene
            );
        }
    }

    #[test]
    fn preparation_rejects_a_registered_tool_without_an_executor_mapping() {
        let engine = ToolEngine::new();
        engine
            .register(
                ToolDefinition::new(
                    "custom.tool",
                    "1.0.0",
                    "Custom tool",
                    json!({"type": "object", "additionalProperties": false}),
                    Capability::FilesystemRead,
                    Operation::Read,
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            engine.prepare_invocation("custom.tool", &json!({})),
            Err(ToolError::UnsupportedTool("custom.tool".to_owned()))
        );
    }
}
