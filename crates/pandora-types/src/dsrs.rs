#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Errors arising during Declarative Structured Reasoning (DSRs) processing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DsrsError {
    MissingInputField(String),
    MissingOutputField(String),
    DuplicateFieldName(String),
    EmptySignatureName,
    ParseError(String),
}

impl fmt::Display for DsrsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInputField(field) => {
                write!(formatter, "missing required input field: '{field}'")
            }
            Self::MissingOutputField(field) => {
                write!(
                    formatter,
                    "missing required output field in response: '{field}'"
                )
            }
            Self::DuplicateFieldName(field) => {
                write!(formatter, "duplicate signature field name: '{field}'")
            }
            Self::EmptySignatureName => {
                write!(formatter, "signature name cannot be empty")
            }
            Self::ParseError(msg) => {
                write!(
                    formatter,
                    "failed to parse structured reasoning output: {msg}"
                )
            }
        }
    }
}

impl std::error::Error for DsrsError {}

/// The role of a field within a Declarative Signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureFieldKind {
    Input,
    Output,
}

/// A strongly typed field specification in a Declarative Signature.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignatureField {
    pub name: String,
    pub description: String,
    pub kind: SignatureFieldKind,
    pub prefix: String,
}

impl SignatureField {
    pub fn new_input(name: impl Into<String>, description: impl Into<String>) -> Self {
        let name_str = name.into();
        let prefix = format!("{}:", capitalize_field(&name_str));
        Self {
            name: name_str,
            description: description.into(),
            kind: SignatureFieldKind::Input,
            prefix,
        }
    }

    pub fn new_output(name: impl Into<String>, description: impl Into<String>) -> Self {
        let name_str = name.into();
        let prefix = format!("{}:", capitalize_field(&name_str));
        Self {
            name: name_str,
            description: description.into(),
            kind: SignatureFieldKind::Output,
            prefix,
        }
    }
}

/// A Declarative Signature: defines the contract (instructions, inputs, outputs)
/// for structured language model reasoning, following the DSRs (DSPy in Rust) paradigm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub instructions: String,
    pub fields: Vec<SignatureField>,
}

impl Signature {
    pub fn new(
        name: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, DsrsError> {
        let name_str = name.into();
        if name_str.trim().is_empty() {
            return Err(DsrsError::EmptySignatureName);
        }
        Ok(Self {
            name: name_str,
            instructions: instructions.into(),
            fields: Vec::new(),
        })
    }

    pub fn with_input(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, DsrsError> {
        let field = SignatureField::new_input(name, description);
        if self.fields.iter().any(|f| f.name == field.name) {
            return Err(DsrsError::DuplicateFieldName(field.name));
        }
        self.fields.push(field);
        Ok(self)
    }

    pub fn with_output(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, DsrsError> {
        let field = SignatureField::new_output(name, description);
        if self.fields.iter().any(|f| f.name == field.name) {
            return Err(DsrsError::DuplicateFieldName(field.name));
        }
        self.fields.push(field);
        Ok(self)
    }

    pub fn inputs(&self) -> impl Iterator<Item = &SignatureField> {
        self.fields
            .iter()
            .filter(|f| f.kind == SignatureFieldKind::Input)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &SignatureField> {
        self.fields
            .iter()
            .filter(|f| f.kind == SignatureFieldKind::Output)
    }

    /// Formats the prompt buffer incorporating system instructions, field schemas,
    /// and supplied input values.
    pub fn format_prompt(
        &self,
        inputs: &BTreeMap<String, String>,
        demonstrations: &[DsrDemonstration],
    ) -> Result<String, DsrsError> {
        let mut prompt = String::new();

        // System instructions & schema contract
        prompt.push_str(&self.instructions);
        prompt.push_str("\n\nFollow the following format strictly:\n\n");

        for field in &self.fields {
            prompt.push_str(&format!(
                "{}: {}\n",
                field.prefix.trim_end_matches(':'),
                field.description
            ));
        }

        // Few-shot demonstrations
        for (idx, demo) in demonstrations.iter().enumerate() {
            prompt.push_str(&format!("\n--- Example {} ---\n", idx + 1));
            for input_field in self.inputs() {
                if let Some(val) = demo.inputs.get(&input_field.name) {
                    prompt.push_str(&format!("{} {}\n", input_field.prefix, val));
                }
            }
            for output_field in self.outputs() {
                if let Some(val) = demo.outputs.get(&output_field.name) {
                    prompt.push_str(&format!("{} {}\n", output_field.prefix, val));
                }
            }
        }

        // Test input instance
        prompt.push_str("\n--- Current Task ---\n");
        for input_field in self.inputs() {
            let val = inputs
                .get(&input_field.name)
                .ok_or_else(|| DsrsError::MissingInputField(input_field.name.clone()))?;
            prompt.push_str(&format!("{} {}\n", input_field.prefix, val));
        }

        // Prompt the first output field
        if let Some(first_out) = self.outputs().next() {
            prompt.push_str(&first_out.prefix);
            prompt.push(' ');
        }

        Ok(prompt)
    }

    /// Parses structured field values from model completion text.
    pub fn parse_outputs(&self, text: &str) -> Result<BTreeMap<String, String>, DsrsError> {
        let mut result = BTreeMap::new();
        let output_fields: Vec<&SignatureField> = self.outputs().collect();

        if output_fields.is_empty() {
            return Ok(result);
        }

        // When only 1 output field exists and the prefix was primed in the prompt
        if output_fields.len() == 1 {
            let field = output_fields[0];
            let clean = text.trim();
            let val = if let Some(stripped) = clean.strip_prefix(&field.prefix) {
                stripped.trim()
            } else {
                clean
            };
            result.insert(field.name.clone(), val.to_owned());
            return Ok(result);
        }

        // Multi-output field parsing using delimiter headers
        let mut current_field: Option<&SignatureField> = output_fields.first().copied();
        let mut current_accumulator = String::new();

        for line in text.lines() {
            let mut matched_new_field = false;
            for out_field in &output_fields {
                let candidate_prefix = &out_field.prefix;
                if line.starts_with(candidate_prefix) {
                    if let Some(prev) = current_field.take() {
                        result.insert(prev.name.clone(), current_accumulator.trim().to_owned());
                        current_accumulator.clear();
                    }
                    current_field = Some(out_field);
                    let remainder = &line[candidate_prefix.len()..];
                    current_accumulator.push_str(remainder.trim());
                    current_accumulator.push('\n');
                    matched_new_field = true;
                    break;
                }
            }
            if !matched_new_field {
                current_accumulator.push_str(line);
                current_accumulator.push('\n');
            }
        }

        if let Some(prev) = current_field.take() {
            result.insert(prev.name.clone(), current_accumulator.trim().to_owned());
        }

        // Validate all required outputs were produced
        for out_field in &output_fields {
            if !result.contains_key(&out_field.name) {
                return Err(DsrsError::MissingOutputField(out_field.name.clone()));
            }
        }

        Ok(result)
    }

    /// Creates a Chain-of-Thought (CoT) enhanced signature by prepending a 'rationale'
    /// output field before original outputs.
    pub fn into_chain_of_thought(mut self) -> Self {
        let has_rationale = self.fields.iter().any(|f| f.name == "rationale");
        if !has_rationale {
            let rationale_field = SignatureField::new_output(
                "rationale",
                "Think step-by-step and produce a clear reasoning justification for the subsequent output fields",
            );
            // Insert as first output field
            let first_output_idx = self
                .fields
                .iter()
                .position(|f| f.kind == SignatureFieldKind::Output)
                .unwrap_or(self.fields.len());
            self.fields.insert(first_output_idx, rationale_field);
        }
        self
    }
}

/// A few-shot demonstration for Declarative Structured Reasoning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DsrDemonstration {
    pub inputs: BTreeMap<String, String>,
    pub outputs: BTreeMap<String, String>,
}

/// Capitalizes field name for clean prompt headers (e.g. `question` -> `Question`).
fn capitalize_field(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_builder_and_prompt_formatting() {
        let sig = Signature::new("CodeReviewer", "You are an expert Rust code reviewer.")
            .unwrap()
            .with_input("code", "Rust source code to review")
            .unwrap()
            .with_output("verdict", "Approve or Reject")
            .unwrap()
            .with_output("feedback", "Actionable reviewer feedback")
            .unwrap();

        assert_eq!(sig.inputs().count(), 1);
        assert_eq!(sig.outputs().count(), 2);

        let mut inputs = BTreeMap::new();
        inputs.insert(
            "code".to_owned(),
            "fn main() { println!(\"hi\"); }".to_owned(),
        );

        let prompt = sig.format_prompt(&inputs, &[]).unwrap();
        assert!(prompt.contains("Code: fn main()"));
        assert!(prompt.contains("Verdict: Approve or Reject"));
        assert!(prompt.ends_with("Verdict: "));
    }

    #[test]
    fn chain_of_thought_transformation() {
        let sig = Signature::new("MathSolver", "Solve the math problem.")
            .unwrap()
            .with_input("problem", "The arithmetic expression")
            .unwrap()
            .with_output("answer", "The numeric answer")
            .unwrap()
            .into_chain_of_thought();

        let outputs: Vec<&SignatureField> = sig.outputs().collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "rationale");
        assert_eq!(outputs[1].name, "answer");
    }

    #[test]
    fn multi_field_output_parsing() {
        let sig = Signature::new("Classifier", "Classify sentiment.")
            .unwrap()
            .with_input("text", "Input sentence")
            .unwrap()
            .with_output("sentiment", "Positive, Negative, or Neutral")
            .unwrap()
            .with_output("confidence", "High, Medium, or Low")
            .unwrap();

        let completion = "Sentiment: Positive\nConfidence: High";
        let parsed = sig.parse_outputs(completion).unwrap();

        assert_eq!(
            parsed.get("sentiment").map(String::as_str),
            Some("Positive")
        );
        assert_eq!(parsed.get("confidence").map(String::as_str), Some("High"));
    }

    #[test]
    fn missing_input_field_fails_closed() {
        let sig = Signature::new("QA", "Answer question.")
            .unwrap()
            .with_input("question", "User question")
            .unwrap()
            .with_output("answer", "Answer text")
            .unwrap();

        let inputs = BTreeMap::new();
        assert!(sig.format_prompt(&inputs, &[]).is_err());
    }
}
