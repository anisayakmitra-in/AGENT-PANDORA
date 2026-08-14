use crate::capability::Capability;
use crate::effect::{OperationRequest, RequestError};
use crate::ids::{GeneId, IdError};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GeneKind {
    Pure,
    Tool,
    Workflow,
    Agent,
    Benchmark,
    Mcp,
    Security,
}

impl GeneKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Tool => "tool",
            Self::Workflow => "workflow",
            Self::Agent => "agent",
            Self::Benchmark => "benchmark",
            Self::Mcp => "mcp",
            Self::Security => "security",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneManifest {
    id: GeneId,
    version: String,
    kind: GeneKind,
    capabilities: Vec<Capability>,
}

impl GeneManifest {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        kind: GeneKind,
        capabilities: Vec<Capability>,
    ) -> Result<Self, GeneError> {
        let id = GeneId::new(id).map_err(GeneError::InvalidId)?;
        let version = version.into();
        if version.trim().is_empty() {
            return Err(GeneError::EmptyField("version"));
        }
        Ok(Self {
            id,
            version,
            kind,
            capabilities,
        })
    }

    pub fn id(&self) -> &GeneId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn kind(&self) -> GeneKind {
        self.kind
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneInput(String);

impl GeneInput {
    pub fn new(value: impl Into<String>) -> Result<Self, GeneError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GeneError::EmptyField("gene input"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeneError {
    InvalidId(IdError),
    EmptyField(&'static str),
    Request(RequestError),
}

impl fmt::Display for GeneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::Request(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for GeneError {}

impl From<RequestError> for GeneError {
    fn from(error: RequestError) -> Self {
        Self::Request(error)
    }
}

pub trait Gene: Send + Sync {
    fn manifest(&self) -> &GeneManifest;
    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError>;
}
