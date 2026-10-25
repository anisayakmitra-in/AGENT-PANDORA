//! Reusable Prompt Presets & Engine for AGENT-PANDORA.
//!
//! Provides multi-tier storage (.pandora/prompts/, user config dir, and built-ins),
//! parameter interpolation ({{workspace}}, {{task}}, {{lang}}, custom parameters),
//! and preset management.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetKind {
    Persona,
    Template,
}

impl PresetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Persona => "persona",
            Self::Template => "template",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "persona" => Some(Self::Persona),
            "template" => Some(Self::Template),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageTier {
    Workspace,
    User,
    Builtin,
}

impl StorageTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::User => "user",
            Self::Builtin => "builtin",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromptPreset {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: PresetKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug)]
pub enum PromptError {
    NotFound(String),
    AlreadyExists(String),
    InvalidName(String),
    MissingParameter { param: String, template: String },
    Io(std::io::Error),
    Serialization(serde_json::Error),
}

impl fmt::Display for PromptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "prompt preset '{name}' not found"),
            Self::AlreadyExists(name) => write!(f, "prompt preset '{name}' already exists"),
            Self::InvalidName(name) => write!(
                f,
                "invalid preset name '{name}': use alphanumeric characters, dashes, or underscores"
            ),
            Self::MissingParameter { param, template } => write!(
                f,
                "missing required parameter '{{{{{param}}}}}' in template: {template}"
            ),
            Self::Io(err) => write!(f, "I/O error accessing prompt storage: {err}"),
            Self::Serialization(err) => write!(f, "serialization error in prompt preset: {err}"),
        }
    }
}

impl std::error::Error for PromptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Serialization(err) => Some(err),
            _ => None,
        }
    }
}

pub struct PromptEngine {
    workspace_dir: PathBuf,
    user_config_dir: PathBuf,
}

impl PromptEngine {
    pub fn open(workspace_dir: impl Into<PathBuf>, user_config_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            user_config_dir: user_config_dir.into(),
        }
    }

    pub fn workspace_prompts_dir(&self) -> PathBuf {
        self.workspace_dir.join(".pandora").join("prompts")
    }

    pub fn user_prompts_dir(&self) -> PathBuf {
        self.user_config_dir.join("prompts")
    }

    pub fn list(&self) -> Result<Vec<(PromptPreset, StorageTier)>, PromptError> {
        let mut results = BTreeMap::new();

        // 1. Built-in templates
        for preset in Self::builtins() {
            results.insert(
                preset.name.to_ascii_lowercase(),
                (preset, StorageTier::Builtin),
            );
        }

        // 2. User config directory layer
        let user_dir = self.user_prompts_dir();
        if user_dir.is_dir()
            && let Ok(entries) = fs::read_dir(&user_dir)
        {
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && let Ok(bytes) = fs::read(&path)
                    && let Ok(preset) = serde_json::from_slice::<PromptPreset>(&bytes)
                {
                    results.insert(
                        preset.name.to_ascii_lowercase(),
                        (preset, StorageTier::User),
                    );
                }
            }
        }

        // 3. Workspace layer (.pandora/prompts/) takes highest precedence
        let ws_dir = self.workspace_prompts_dir();
        if ws_dir.is_dir()
            && let Ok(entries) = fs::read_dir(&ws_dir)
        {
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && let Ok(bytes) = fs::read(&path)
                    && let Ok(preset) = serde_json::from_slice::<PromptPreset>(&bytes)
                {
                    results.insert(
                        preset.name.to_ascii_lowercase(),
                        (preset, StorageTier::Workspace),
                    );
                }
            }
        }

        Ok(results.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<(PromptPreset, StorageTier), PromptError> {
        let list = self.list()?;
        list.into_iter()
            .find(|(preset, _)| preset.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| PromptError::NotFound(name.to_owned()))
    }

    pub fn save(&self, preset: PromptPreset, workspace_tier: bool) -> Result<PathBuf, PromptError> {
        validate_preset_name(&preset.name)?;
        let target_dir = if workspace_tier {
            self.workspace_prompts_dir()
        } else {
            self.user_prompts_dir()
        };
        fs::create_dir_all(&target_dir).map_err(PromptError::Io)?;
        let target_file = target_dir.join(format!("{}.json", preset.name));
        let data = serde_json::to_vec_pretty(&preset).map_err(PromptError::Serialization)?;
        fs::write(&target_file, data).map_err(PromptError::Io)?;
        Ok(target_file)
    }

    pub fn delete(&self, name: &str, workspace_tier: bool) -> Result<bool, PromptError> {
        validate_preset_name(name)?;
        let target_dir = if workspace_tier {
            self.workspace_prompts_dir()
        } else {
            self.user_prompts_dir()
        };
        if !target_dir.is_dir() {
            return Ok(false);
        }
        let target_file = target_dir.join(format!("{name}.json"));
        if target_file.is_file() {
            fs::remove_file(target_file).map_err(PromptError::Io)?;
            Ok(true)
        } else {
            // Case-insensitive fallback on case-sensitive filesystems
            if let Ok(entries) = fs::read_dir(&target_dir) {
                let target_filename = format!("{name}.json");
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .file_name()
                            .and_then(|f| f.to_str())
                            .is_some_and(|f| f.eq_ignore_ascii_case(&target_filename))
                    {
                        fs::remove_file(path).map_err(PromptError::Io)?;
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }

    pub fn interpolate(
        template: &str,
        params: &BTreeMap<String, String>,
    ) -> Result<String, PromptError> {
        let mut output = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(start_idx) = rest.find("{{") {
            output.push_str(&rest[..start_idx]);
            let after_open = &rest[start_idx + 2..];
            if let Some(end_idx) = after_open.find("}}") {
                let var_name = after_open[..end_idx].trim();
                let value = params
                    .get(var_name)
                    .ok_or_else(|| PromptError::MissingParameter {
                        param: var_name.to_owned(),
                        template: template.to_owned(),
                    })?;
                output.push_str(value);
                rest = &after_open[end_idx + 2..];
            } else {
                output.push_str("{{");
                rest = after_open;
            }
        }
        output.push_str(rest);
        Ok(output)
    }

    pub fn extract_parameters(content: &str) -> Vec<String> {
        let mut params = Vec::new();
        let mut rest = content;
        while let Some(idx) = rest.find("{{") {
            let after = &rest[idx + 2..];
            if let Some(end) = after.find("}}") {
                let p = after[..end].trim().to_owned();
                if !params.contains(&p) {
                    params.push(p);
                }
                rest = &after[end + 2..];
            } else {
                break;
            }
        }
        params
    }

    pub fn detect_workspace_lang(path: &Path) -> String {
        if path.join("Cargo.toml").is_file() {
            "rust".to_owned()
        } else if path.join("package.json").is_file() {
            "typescript".to_owned()
        } else if path.join("pyproject.toml").is_file() || path.join("requirements.txt").is_file() {
            "python".to_owned()
        } else if path.join("go.mod").is_file() {
            "go".to_owned()
        } else {
            "text".to_owned()
        }
    }

    pub fn builtins() -> Vec<PromptPreset> {
        vec![
            PromptPreset {
                name: "reviewer".to_owned(),
                description: Some(
                    "Strict code reviewer focusing on correctness, security, tests, and clean architecture"
                        .to_owned(),
                ),
                kind: PresetKind::Persona,
                content: "You are an expert code reviewer. Analyze changes with strict scrutiny. Prioritize architectural soundness, memory safety, concurrency correctness, TOCTOU resistance, and comprehensive test coverage. Point out exact file paths, line numbers, and concrete remediation snippets."
                    .to_owned(),
                parameters: Vec::new(),
                model: None,
                mode: Some("supervised".to_owned()),
                created_at: 0,
                updated_at: 0,
            },
            PromptPreset {
                name: "security-auditor".to_owned(),
                description: Some(
                    "Security auditor examining TOCTOU, path traversal, injection, and authorization boundaries"
                        .to_owned(),
                ),
                kind: PresetKind::Persona,
                content: "You are a senior security researcher auditing repository code. Thoroughly examine effect authorization, handle-relative filesystem operations, process execution boundaries, secret leaks, and sanitization boundaries. Fail closed on any ambiguous security invariant."
                    .to_owned(),
                parameters: Vec::new(),
                model: None,
                mode: Some("read-only".to_owned()),
                created_at: 0,
                updated_at: 0,
            },
            PromptPreset {
                name: "refactor".to_owned(),
                description: Some(
                    "Structured refactoring task template with workspace and language grounding"
                        .to_owned(),
                ),
                kind: PresetKind::Template,
                content: "Inspect {{workspace}} for target module '{{target}}'. Refactor the implementation to accomplish: {{task}}. Ensure exact functional parity, maintain strict idioms of {{lang}}, preserve all public traits, and guarantee zero regressions across the test suite."
                    .to_owned(),
                parameters: vec![
                    "workspace".to_owned(),
                    "target".to_owned(),
                    "task".to_owned(),
                    "lang".to_owned(),
                ],
                model: None,
                mode: Some("supervised".to_owned()),
                created_at: 0,
                updated_at: 0,
            },
            PromptPreset {
                name: "test-writer".to_owned(),
                description: Some("Comprehensive test generation template".to_owned()),
                kind: PresetKind::Template,
                content: "Examine '{{target}}' in {{workspace}}. Generate exhaustive unit, edge-case, and negative control tests for: {{task}}. Follow the testing conventions of {{lang}} and verify all tests pass cleanly."
                    .to_owned(),
                parameters: vec![
                    "workspace".to_owned(),
                    "target".to_owned(),
                    "task".to_owned(),
                    "lang".to_owned(),
                ],
                model: None,
                mode: Some("supervised".to_owned()),
                created_at: 0,
                updated_at: 0,
            },
            PromptPreset {
                name: "explain".to_owned(),
                description: Some("Architecture and control flow explainer".to_owned()),
                kind: PresetKind::Template,
                content: "Examine '{{target}}' in {{workspace}}. Provide a detailed architectural breakdown explaining control flow, data invariants, and edge cases to accomplish: {{task}}."
                    .to_owned(),
                parameters: vec![
                    "workspace".to_owned(),
                    "target".to_owned(),
                    "task".to_owned(),
                ],
                model: None,
                mode: Some("read-only".to_owned()),
                created_at: 0,
                updated_at: 0,
            },
        ]
    }
}

pub fn validate_preset_name(name: &str) -> Result<(), PromptError> {
    if name.trim().is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PromptError::InvalidName(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets_are_valid() {
        let builtins = PromptEngine::builtins();
        assert_eq!(builtins.len(), 5);
        for b in &builtins {
            validate_preset_name(&b.name).unwrap();
            assert!(!b.content.is_empty());
        }
    }

    #[test]
    fn test_interpolation_standard_and_custom() {
        let mut params = BTreeMap::new();
        params.insert("workspace".to_owned(), "/work/test".to_owned());
        params.insert("target".to_owned(), "crates/core".to_owned());
        params.insert("task".to_owned(), "add telemetry".to_owned());
        params.insert("lang".to_owned(), "rust".to_owned());

        let template = "In {{workspace}}, refactor {{target}} for {{task}} using {{lang}}.";
        let res = PromptEngine::interpolate(template, &params).unwrap();
        assert_eq!(
            res,
            "In /work/test, refactor crates/core for add telemetry using rust."
        );
    }

    #[test]
    fn test_missing_parameter_fails() {
        let mut params = BTreeMap::new();
        params.insert("workspace".to_owned(), "/work/test".to_owned());

        let template = "In {{workspace}}, fix {{bug}}.";
        let err = PromptEngine::interpolate(template, &params).unwrap_err();
        match err {
            PromptError::MissingParameter { param, .. } => assert_eq!(param, "bug"),
            _ => panic!("unexpected error {err:?}"),
        }
    }

    #[test]
    fn test_workspace_overrides_builtin_and_user() {
        let temp_dir = std::env::temp_dir().join(format!(
            "pandora_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ws_dir = temp_dir.join("ws");
        let user_dir = temp_dir.join("user");

        let engine = PromptEngine::open(&ws_dir, &user_dir);

        // Before any override, reviewer is Builtin
        let (p, tier) = engine.get("reviewer").unwrap();
        assert_eq!(tier, StorageTier::Builtin);
        assert_eq!(p.kind, PresetKind::Persona);

        // Save a user tier preset
        let custom_user = PromptPreset {
            name: "reviewer".to_owned(),
            description: Some("User custom reviewer".to_owned()),
            kind: PresetKind::Persona,
            content: "User custom content".to_owned(),
            parameters: Vec::new(),
            model: None,
            mode: None,
            created_at: 1,
            updated_at: 1,
        };
        engine.save(custom_user, false).unwrap();

        // Now reviewer resolves to User tier
        let (p, tier) = engine.get("reviewer").unwrap();
        assert_eq!(tier, StorageTier::User);
        assert_eq!(p.content, "User custom content");

        // Save a workspace tier preset
        let custom_ws = PromptPreset {
            name: "reviewer".to_owned(),
            description: Some("Workspace custom reviewer".to_owned()),
            kind: PresetKind::Persona,
            content: "Workspace custom content".to_owned(),
            parameters: Vec::new(),
            model: None,
            mode: None,
            created_at: 2,
            updated_at: 2,
        };
        engine.save(custom_ws, true).unwrap();

        // Now reviewer resolves to Workspace tier
        let (p, tier) = engine.get("reviewer").unwrap();
        assert_eq!(tier, StorageTier::Workspace);
        assert_eq!(p.content, "Workspace custom content");

        // Delete from workspace
        let deleted = engine.delete("reviewer", true).unwrap();
        assert!(deleted);

        // Falls back to User
        let (_p, tier) = engine.get("reviewer").unwrap();
        assert_eq!(tier, StorageTier::User);

        // Delete from user
        let deleted = engine.delete("reviewer", false).unwrap();
        assert!(deleted);

        // Falls back to Builtin
        let (_p, tier) = engine.get("reviewer").unwrap();
        assert_eq!(tier, StorageTier::Builtin);

        let _ = fs::remove_dir_all(temp_dir);
    }
}
