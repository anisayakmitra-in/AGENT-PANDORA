use serde_json::Value;
use std::fmt;

const MAX_SCHEMA_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuredOutputErrorKind {
    InvalidJson,
    SchemaViolation,
    InvalidFallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaError {
    path: String,
    message: String,
}

impl SchemaError {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepairError {
    Unavailable,
}

impl fmt::Display for RepairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("repair unavailable"),
        }
    }
}

impl std::error::Error for RepairError {}

pub trait Repairer {
    fn repair(&self, value: &Value, error: &SchemaError) -> Result<Value, RepairError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FallbackPolicy {
    Reject,
    Use(Value),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationOutcome {
    Accepted,
    Repaired,
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedOutput {
    value: Value,
    outcome: ValidationOutcome,
}

impl ValidatedOutput {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn outcome(&self) -> ValidationOutcome {
        self.outcome
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructuredOutputError {
    InvalidJson,
    SchemaViolation(SchemaError),
    InvalidFallback(SchemaError),
}

impl StructuredOutputError {
    pub fn kind(&self) -> StructuredOutputErrorKind {
        match self {
            Self::InvalidJson => StructuredOutputErrorKind::InvalidJson,
            Self::SchemaViolation(_) => StructuredOutputErrorKind::SchemaViolation,
            Self::InvalidFallback(_) => StructuredOutputErrorKind::InvalidFallback,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::SchemaViolation(error) | Self::InvalidFallback(error) => error.path(),
            Self::InvalidJson => "$",
        }
    }
}

impl fmt::Display for StructuredOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson => formatter.write_str("provider returned invalid JSON"),
            Self::SchemaViolation(error) => write!(formatter, "invalid field {}", error.path),
            Self::InvalidFallback(error) => {
                write!(formatter, "invalid fallback field {}", error.path)
            }
        }
    }
}

impl std::error::Error for StructuredOutputError {}

pub fn parse_and_validate(
    raw: &str,
    schema: &Value,
    repairer: Option<&dyn Repairer>,
    fallback: FallbackPolicy,
) -> Result<ValidatedOutput, StructuredOutputError> {
    let value = serde_json::from_str(raw).map_err(|_| StructuredOutputError::InvalidJson)?;
    match validate_value(&value, schema, "$", 0) {
        Ok(()) => Ok(ValidatedOutput {
            value,
            outcome: ValidationOutcome::Accepted,
        }),
        Err(error) => recover(value, error, schema, repairer, fallback),
    }
}

fn recover(
    value: Value,
    error: SchemaError,
    schema: &Value,
    repairer: Option<&dyn Repairer>,
    fallback: FallbackPolicy,
) -> Result<ValidatedOutput, StructuredOutputError> {
    if let Some(repairer) = repairer
        && let Ok(repaired) = repairer.repair(&value, &error)
        && validate_value(&repaired, schema, "$", 0).is_ok()
    {
        return Ok(ValidatedOutput {
            value: repaired,
            outcome: ValidationOutcome::Repaired,
        });
    }

    match fallback {
        FallbackPolicy::Reject => Err(StructuredOutputError::SchemaViolation(error)),
        FallbackPolicy::Use(value) => validate_value(&value, schema, "$", 0)
            .map_err(StructuredOutputError::InvalidFallback)
            .map(|_| ValidatedOutput {
                value,
                outcome: ValidationOutcome::Fallback,
            }),
    }
}

fn validate_value(
    value: &Value,
    schema: &Value,
    path: &str,
    depth: usize,
) -> Result<(), SchemaError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(schema_error(path, "schema nesting exceeds the limit"));
    }
    let Some(schema_object) = schema.as_object() else {
        return Err(schema_error(path, "schema must be an object"));
    };
    if let Some(expected) = schema_object.get("type").and_then(Value::as_str)
        && !matches_type(value, expected)
    {
        return Err(schema_error(path, "value has the wrong type"));
    }
    if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
        let Some(object) = value.as_object() else {
            return Ok(());
        };
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                return Err(schema_error(
                    &field_path(path, field),
                    "required field is missing",
                ));
            }
        }
    }
    let Some(properties) = schema_object.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if schema_object
        .get("additionalProperties")
        .and_then(Value::as_bool)
        == Some(false)
        && let Some(unknown) = object.keys().find(|key| !properties.contains_key(*key))
    {
        return Err(schema_error(
            &field_path(path, unknown),
            "field is not allowed",
        ));
    }
    for (field, child_schema) in properties {
        if let Some(child) = object.get(field) {
            validate_value(child, child_schema, &field_path(path, field), depth + 1)?;
        }
    }
    Ok(())
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn field_path(parent: &str, field: &str) -> String {
    if parent == "$" {
        field.to_owned()
    } else {
        format!("{parent}.{field}")
    }
}

fn schema_error(path: &str, message: &str) -> SchemaError {
    SchemaError {
        path: path.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::cell::Cell;

    fn schema() -> Value {
        json!({
            "type": "object",
            "required": ["answer"],
            "properties": {
                "answer": {"type": "string"}
            },
            "additionalProperties": false
        })
    }

    #[test]
    fn valid_json_is_returned_without_repair() {
        let result = parse_and_validate(
            r#"{"answer":"ready"}"#,
            &schema(),
            None,
            FallbackPolicy::Reject,
        )
        .unwrap();

        assert_eq!(result.value()["answer"], "ready");
        assert_eq!(result.outcome(), ValidationOutcome::Accepted);
    }

    #[test]
    fn invalid_json_reports_the_exact_field_path() {
        let error = parse_and_validate(r#"{"answer":42}"#, &schema(), None, FallbackPolicy::Reject)
            .unwrap_err();

        assert_eq!(error.path(), "answer");
        assert_eq!(error.kind(), StructuredOutputErrorKind::SchemaViolation);
    }

    #[test]
    fn one_repair_attempt_can_replace_invalid_output() {
        let repairer = FixedRepairer::new(json!({"answer":"repaired"}));
        let result = parse_and_validate(
            r#"{"answer":42}"#,
            &schema(),
            Some(&repairer),
            FallbackPolicy::Reject,
        )
        .unwrap();

        assert_eq!(result.value()["answer"], "repaired");
        assert_eq!(result.outcome(), ValidationOutcome::Repaired);
        assert_eq!(repairer.calls(), 1);
    }

    #[test]
    fn failed_repair_uses_the_configured_fallback() {
        let repairer = FailingRepairer::default();
        let fallback = json!({"answer":"fallback"});
        let result = parse_and_validate(
            r#"{"answer":42}"#,
            &schema(),
            Some(&repairer),
            FallbackPolicy::Use(fallback.clone()),
        )
        .unwrap();

        assert_eq!(result.value(), &fallback);
        assert_eq!(result.outcome(), ValidationOutcome::Fallback);
        assert_eq!(repairer.calls(), 1);
    }

    struct FixedRepairer {
        replacement: Value,
        calls: Cell<u8>,
    }

    impl FixedRepairer {
        fn new(replacement: Value) -> Self {
            Self {
                replacement,
                calls: Cell::new(0),
            }
        }

        fn calls(&self) -> u8 {
            self.calls.get()
        }
    }

    impl Repairer for FixedRepairer {
        fn repair(&self, _value: &Value, _error: &SchemaError) -> Result<Value, RepairError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.replacement.clone())
        }
    }

    #[derive(Default)]
    struct FailingRepairer {
        calls: Cell<u8>,
    }

    impl FailingRepairer {
        fn calls(&self) -> u8 {
            self.calls.get()
        }
    }

    impl Repairer for FailingRepairer {
        fn repair(&self, _value: &Value, _error: &SchemaError) -> Result<Value, RepairError> {
            self.calls.set(self.calls.get() + 1);
            Err(RepairError::Unavailable)
        }
    }
}
