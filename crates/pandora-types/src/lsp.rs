use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// Supported language server kinds.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspServerKind {
    RustAnalyzer,
    TsServer,
    Pyright,
    Gopls,
    Custom(String),
}

impl LspServerKind {
    pub const fn as_str(&self) -> &str {
        match self {
            Self::RustAnalyzer => "rust-analyzer",
            Self::TsServer => "tsserver",
            Self::Pyright => "pyright",
            Self::Gopls => "gopls",
            Self::Custom(name) => name.as_str(),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::RustAnalyzer => "Rust (rust-analyzer)",
            Self::TsServer => "TypeScript / JavaScript (typescript-language-server)",
            Self::Pyright => "Python (pyright / pylsp)",
            Self::Gopls => "Go (gopls)",
            Self::Custom(name) => name.as_str(),
        }
    }

    pub const fn default_binary(&self) -> &'static str {
        match self {
            Self::RustAnalyzer => "rust-analyzer",
            Self::TsServer => "typescript-language-server",
            Self::Pyright => "pyright-langserver",
            Self::Gopls => "gopls",
            Self::Custom(_) => "",
        }
    }

    pub const fn candidate_binaries(&self) -> &'static [&'static str] {
        match self {
            Self::RustAnalyzer => &["rust-analyzer", "rust-analyzer.exe"],
            Self::TsServer => &[
                "typescript-language-server",
                "typescript-language-server.cmd",
                "typescript-language-server.exe",
                "tsserver",
                "tsserver.cmd",
            ],
            Self::Pyright => &[
                "pyright-langserver",
                "pyright-langserver.cmd",
                "pyright-langserver.exe",
                "pyright",
                "pylsp",
                "pylsp.exe",
            ],
            Self::Gopls => &["gopls", "gopls.exe"],
            Self::Custom(_) => &[],
        }
    }

    pub const fn default_args(&self) -> &'static [&'static str] {
        match self {
            Self::RustAnalyzer => &[],
            Self::TsServer => &["--stdio"],
            Self::Pyright => &["--stdio"],
            Self::Gopls => &[],
            Self::Custom(_) => &[],
        }
    }

    pub const fn trigger_files(&self) -> &'static [&'static str] {
        match self {
            Self::RustAnalyzer => &["Cargo.toml", "Cargo.lock"],
            Self::TsServer => &["tsconfig.json", "package.json", "jsconfig.json"],
            Self::Pyright => &[
                "pyproject.toml",
                "requirements.txt",
                "setup.py",
                "pyrightconfig.json",
                "Pipfile",
            ],
            Self::Gopls => &["go.mod", "go.sum", "go.work"],
            Self::Custom(_) => &[],
        }
    }

    pub const fn file_extensions(&self) -> &'static [&'static str] {
        match self {
            Self::RustAnalyzer => &["rs"],
            Self::TsServer => &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
            Self::Pyright => &["py", "pyi"],
            Self::Gopls => &["go"],
            Self::Custom(_) => &[],
        }
    }

    pub fn detect_for_extension(ext: &str) -> Option<Self> {
        let norm = ext.trim().trim_start_matches('.').to_lowercase();
        if norm == "rs" {
            Some(Self::RustAnalyzer)
        } else if matches!(norm.as_str(), "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
            Some(Self::TsServer)
        } else if matches!(norm.as_str(), "py" | "pyi") {
            Some(Self::Pyright)
        } else if norm == "go" {
            Some(Self::Gopls)
        } else {
            None
        }
    }

    pub fn detect_for_path(path: &Path) -> Option<Self> {
        if let Some(kind) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Self::detect_for_extension)
        {
            return Some(kind);
        }
        if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
            for kind in Self::all_standard() {
                if kind
                    .trigger_files()
                    .iter()
                    .any(|&tf| tf.eq_ignore_ascii_case(file_name))
                {
                    return Some(kind.clone());
                }
            }
        }
        None
    }

    pub const fn all_standard() -> &'static [Self] {
        &[
            Self::RustAnalyzer,
            Self::TsServer,
            Self::Pyright,
            Self::Gopls,
        ]
    }
}

impl fmt::Display for LspServerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LspServerKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "rust-analyzer" | "rust" => Ok(Self::RustAnalyzer),
            "tsserver"
            | "typescript"
            | "typescript-language-server"
            | "javascript"
            | "js"
            | "ts" => Ok(Self::TsServer),
            "pyright" | "pyright-langserver" | "pylsp" | "python" | "py" => Ok(Self::Pyright),
            "gopls" | "go" | "golang" => Ok(Self::Gopls),
            custom if !custom.is_empty() => Ok(Self::Custom(custom.to_string())),
            _ => Err("empty server name".to_string()),
        }
    }
}

/// A 0-based position in a text document as defined by LSP.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

impl LspPosition {
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }

    /// Converts 1-indexed (line, column) numbers to 0-indexed LSP position.
    pub const fn from_one_indexed(line: u32, col: u32) -> Self {
        Self {
            line: if line > 0 { line - 1 } else { 0 },
            character: if col > 0 { col - 1 } else { 0 },
        }
    }

    /// Converts 0-indexed LSP position to 1-indexed (line, column).
    pub const fn to_one_indexed(&self) -> (u32, u32) {
        (
            self.line.saturating_add(1),
            self.character.saturating_add(1),
        )
    }
}

/// A range in a text document expressed as start and end positions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

impl LspRange {
    pub const fn new(start: LspPosition, end: LspPosition) -> Self {
        Self { start, end }
    }

    pub const fn point(line: u32, character: u32) -> Self {
        let pos = LspPosition::new(line, character);
        Self {
            start: pos,
            end: pos,
        }
    }
}

/// A location inside a resource, such as a line/column inside a file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
}

impl LspLocation {
    pub fn new(uri: impl Into<String>, range: LspRange) -> Self {
        Self {
            uri: uri.into(),
            range,
            relative_path: None,
        }
    }

    pub fn with_relative_path(mut self, path: impl Into<String>) -> Self {
        self.relative_path = Some(path.into());
        self
    }
}

/// LSP Diagnostic severity levels (1 = Error, 2 = Warning, 3 = Information, 4 = Hint).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum LspDiagnosticSeverity {
    #[serde(rename = "error")]
    Error = 1,
    #[serde(rename = "warning")]
    Warning = 2,
    #[serde(rename = "information")]
    Information = 3,
    #[serde(rename = "hint")]
    Hint = 4,
}

impl LspDiagnosticSeverity {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }

    pub const fn from_raw(val: u64) -> Self {
        match val {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            4 => Self::Hint,
            _ => Self::Information,
        }
    }
}

impl fmt::Display for LspDiagnosticSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LspDiagnosticSeverity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "error" | "err" | "1" => Ok(Self::Error),
            "warning" | "warn" | "2" => Ok(Self::Warning),
            "information" | "info" | "3" => Ok(Self::Information),
            "hint" | "4" => Ok(Self::Hint),
            other => Err(format!("unknown diagnostic severity '{other}'")),
        }
    }
}

/// An LSP Diagnostic object representing a compiler/linter error, warning, or hint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub uri: String,
    pub range: LspRange,
    pub severity: LspDiagnosticSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
}

impl LspDiagnostic {
    pub fn new(
        uri: impl Into<String>,
        range: LspRange,
        severity: LspDiagnosticSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            uri: uri.into(),
            range,
            severity,
            code: None,
            source: None,
            message: message.into(),
            relative_path: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_relative_path(mut self, path: impl Into<String>) -> Self {
        self.relative_path = Some(path.into());
        self
    }
}

/// Status and metadata for a language server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspServerStatus {
    pub kind: LspServerKind,
    pub name: String,
    pub binary: String,
    pub available: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
    pub capabilities: Vec<String>,
}

/// Result of a goto definition request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspDefinitionResult {
    pub locations: Vec<LspLocation>,
}

/// Result of a find references request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspReferencesResult {
    pub references: Vec<LspLocation>,
}

/// Result of a type definition request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspTypeDefinitionResult {
    pub locations: Vec<LspLocation>,
}

/// Result of a compiler diagnostics check.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspDiagnosticsResult {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<LspDiagnostic>,
}

impl LspDiagnosticsResult {
    pub fn from_diagnostics(diagnostics: Vec<LspDiagnostic>) -> Self {
        let mut errors = 0;
        let mut warnings = 0;
        for d in &diagnostics {
            match d.severity {
                LspDiagnosticSeverity::Error => errors += 1,
                LspDiagnosticSeverity::Warning => warnings += 1,
                _ => {}
            }
        }
        let total = diagnostics.len();
        Self {
            total,
            errors,
            warnings,
            diagnostics,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.errors > 0
    }
}
