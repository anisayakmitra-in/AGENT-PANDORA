use pandora_types::{
    ArtifactId, Capability, EffectTarget, ExecutionId, GeneId, Operation, OperationRequest,
    PrincipalId, RequestError, ResourceScope, SessionId, TaskIntent,
};
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

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
    artifact_id: Option<ArtifactId>,
}

impl ToolContext {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        artifact_id: Option<ArtifactId>,
    ) -> Self {
        Self {
            execution_id,
            session_id,
            principal_id,
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

pub struct ToolEngine {
    definitions: Mutex<HashMap<GeneId, ToolDefinition>>,
    idempotent_plans: Mutex<HashMap<String, ToolPlan>>,
}

impl ToolEngine {
    pub fn new() -> Self {
        Self {
            definitions: Mutex::new(HashMap::new()),
            idempotent_plans: Mutex::new(HashMap::new()),
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

    pub fn list(&self) -> Vec<ToolDefinition> {
        let definitions = self
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut values = definitions.values().cloned().collect::<Vec<_>>();
        values.sort_by(|left, right| left.id().cmp(right.id()));
        values
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
            _ => {
                return Err(ToolError::UnsupportedTool(
                    definition.id().as_str().to_owned(),
                ));
            }
        };
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
        let payload = serde_json::to_vec(&arguments)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        let request = OperationRequest::new(
            context.execution_id().clone(),
            context.session_id().clone(),
            context.principal_id().clone(),
            definition.id().clone(),
            context.artifact_id().cloned(),
            definition.capability(),
            definition.operation(),
            target,
            resource_scope,
        )
        .map_err(ToolError::Request)?
        .with_payload_digest(&payload)
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

#[cfg(test)]
mod tests {
    use super::*;
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

    fn context() -> ToolContext {
        ToolContext::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
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
