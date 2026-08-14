use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    ArtifactId, Capability, EffectTarget, ExecutionId, Gene, GeneInput, GeneManifest, Operation,
    OperationRequest, PrincipalId, ResourceScope, SessionId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

const MAX_PATH_BYTES: usize = 4_096;
const MAX_PATCH_BYTES: usize = 1_048_576;
const VERIFICATION_SPEC: &str = "cargo check --locked";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAction {
    Read,
    Search,
    Patch,
    Verify,
    Review,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanningContext {
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    workspace_id: WorkspaceId,
    artifact_id: Option<ArtifactId>,
}

impl PlanningContext {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        workspace_id: WorkspaceId,
    ) -> Self {
        Self {
            execution_id,
            session_id,
            principal_id,
            workspace_id,
            artifact_id: None,
        }
    }

    pub fn with_artifact(mut self, artifact_id: ArtifactId) -> Self {
        self.artifact_id = Some(artifact_id);
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRequest {
    action: CodingAction,
    context: PlanningContext,
    path: Option<String>,
    content: Option<String>,
    command: Option<String>,
}

impl CodingRequest {
    pub fn read(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Read, context, path)
    }

    pub fn search(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Search, context, path)
    }

    pub fn review(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Review, context, path)
    }

    pub fn patch(
        context: PlanningContext,
        path: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            action: CodingAction::Patch,
            context,
            path: Some(path.into()),
            content: Some(content.into()),
            command: None,
        }
    }

    pub fn verify(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Verify,
            context,
            path: None,
            content: None,
            command: Some(VERIFICATION_SPEC.to_owned()),
        }
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        validate_request(&self)?;
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("coding input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("coding input must be valid JSON"))
    }

    fn for_path(action: CodingAction, context: PlanningContext, path: impl Into<String>) -> Self {
        Self {
            action,
            context,
            path: Some(path.into()),
            content: None,
            command: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodingGeneRole {
    Read,
    Search,
    Patch,
    Verify,
    Review,
}

impl CodingGeneRole {
    const fn action(self) -> CodingAction {
        match self {
            Self::Read => CodingAction::Read,
            Self::Search => CodingAction::Search,
            Self::Patch => CodingAction::Patch,
            Self::Verify => CodingAction::Verify,
            Self::Review => CodingAction::Review,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Read => "workspace.read",
            Self::Search => "workspace.search",
            Self::Patch => "patch.apply",
            Self::Verify => "verification.run",
            Self::Review => "change.review",
        }
    }

    const fn capability(self) -> Capability {
        match self {
            Self::Patch => Capability::FilesystemWrite,
            Self::Verify => Capability::ProcessExecute,
            Self::Read | Self::Search | Self::Review => Capability::FilesystemRead,
        }
    }

    const fn operation(self) -> Operation {
        match self {
            Self::Patch => Operation::Write,
            Self::Verify => Operation::Execute,
            Self::Read | Self::Search | Self::Review => Operation::Read,
        }
    }
}

pub struct CodingGene {
    manifest: GeneManifest,
    role: CodingGeneRole,
}

impl CodingGene {
    pub fn new(role: CodingGeneRole) -> Result<Self, GeneError> {
        let manifest =
            GeneManifest::new(role.id(), "0.1.0", GeneKind::Tool, vec![role.capability()])?;
        Ok(Self { manifest, role })
    }

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            CodingGeneRole::Read,
            CodingGeneRole::Search,
            CodingGeneRole::Patch,
            CodingGeneRole::Verify,
            CodingGeneRole::Review,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Coding Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }
}

impl Gene for CodingGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = CodingRequest::parse(input)?;
        validate_request(&request)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "coding input action does not match Gene",
            ));
        }
        let patch_content = request.content.as_deref();
        let path = request.path.as_deref();
        let target = match self.role {
            CodingGeneRole::Verify => EffectTarget::process(VERIFICATION_SPEC),
            CodingGeneRole::Read | CodingGeneRole::Search | CodingGeneRole::Review => {
                EffectTarget::path(path.ok_or(GeneError::InvalidInput("path is required"))?)
            }
            CodingGeneRole::Patch => {
                EffectTarget::path(path.ok_or(GeneError::InvalidInput("path is required"))?)
            }
        };
        let request = OperationRequest::new(
            request.context.execution_id.clone(),
            request.context.session_id.clone(),
            request.context.principal_id.clone(),
            self.manifest.id().clone(),
            request.context.artifact_id.clone(),
            self.role.capability(),
            self.role.operation(),
            target,
            ResourceScope::workspace(request.context.workspace_id.as_str()),
        )?;
        if self.role == CodingGeneRole::Patch {
            let content =
                patch_content.ok_or(GeneError::InvalidInput("patch content is required"))?;
            return Ok(vec![request.with_payload_digest(content.as_bytes())?]);
        }
        Ok(vec![request])
    }
}

fn validate_request(request: &CodingRequest) -> Result<(), GeneError> {
    match request.action {
        CodingAction::Search => {
            validate_query(
                request
                    .path
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("query is required"))?,
            )?;
            if request.content.is_some() {
                return Err(GeneError::InvalidInput(
                    "content is not valid for this action",
                ));
            }
        }
        CodingAction::Read | CodingAction::Review => {
            validate_path(
                request
                    .path
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("path is required"))?,
            )?;
            if request.content.is_some() {
                return Err(GeneError::InvalidInput(
                    "content is not valid for this action",
                ));
            }
        }
        CodingAction::Patch => {
            validate_path(
                request
                    .path
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("path is required"))?,
            )?;
            let content = request
                .content
                .as_deref()
                .ok_or(GeneError::InvalidInput("patch content is required"))?;
            if content.is_empty() || content.len() > MAX_PATCH_BYTES {
                return Err(GeneError::InvalidInput(
                    "patch content exceeds the size limit",
                ));
            }
        }
        CodingAction::Verify => {
            if request.command.as_deref() != Some(VERIFICATION_SPEC) {
                return Err(GeneError::InvalidInput("unsupported verification command"));
            }
            if request.path.is_some() || request.content.is_some() {
                return Err(GeneError::InvalidInput(
                    "path and content are not valid for verification",
                ));
            }
        }
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), GeneError> {
    if query.trim().is_empty() || query.len() > MAX_PATH_BYTES || query.contains('\0') {
        return Err(GeneError::InvalidInput("invalid search query"));
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), GeneError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
        return Err(GeneError::InvalidInput("invalid workspace path"));
    }
    let portable = path.replace('\\', "/");
    if portable.starts_with('/')
        || portable.len() >= 2 && portable.as_bytes()[1] == b':'
        || portable.split('/').any(|part| part == "..")
    {
        return Err(GeneError::InvalidInput("workspace path escapes the root"));
    }
    if Path::new(path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(GeneError::InvalidInput("workspace path escapes the root"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        EffectTarget, ExecutionId, Gene, Operation, PrincipalId, SessionId, WorkspaceId,
    };

    fn context() -> PlanningContext {
        PlanningContext::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
        )
    }

    #[test]
    fn read_gene_plans_a_read_scoped_path_request() {
        let gene = CodingGene::new(CodingGeneRole::Read).unwrap();
        let input = CodingRequest::read(context(), "src/lib.rs")
            .into_gene_input()
            .unwrap();

        let requests = gene.plan(&input).unwrap();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability(), Capability::FilesystemRead);
        assert_eq!(requests[0].operation(), Operation::Read);
        assert_eq!(requests[0].target(), &EffectTarget::path("src/lib.rs"));
    }

    #[test]
    fn search_and_review_are_read_scoped() {
        for (role, request) in [
            (
                CodingGeneRole::Search,
                CodingRequest::search(context(), "."),
            ),
            (
                CodingGeneRole::Review,
                CodingRequest::review(context(), "src/lib.rs"),
            ),
        ] {
            let gene = CodingGene::new(role).unwrap();
            let requests = gene.plan(&request.into_gene_input().unwrap()).unwrap();
            assert_eq!(requests[0].capability(), Capability::FilesystemRead);
        }
    }

    #[test]
    fn patch_binds_content_to_a_write_request() {
        let gene = CodingGene::new(CodingGeneRole::Patch).unwrap();
        let input = CodingRequest::patch(context(), "src/lib.rs", "new content")
            .into_gene_input()
            .unwrap();

        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::FilesystemWrite);
        assert!(request.payload_digest_matches(b"new content"));
        assert!(!request.payload_digest_matches(b"different content"));
    }

    #[test]
    fn verification_accepts_only_the_allowlisted_command() {
        let gene = CodingGene::new(CodingGeneRole::Verify).unwrap();
        let input = CodingRequest::verify(context()).into_gene_input().unwrap();

        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::ProcessExecute);
        assert_eq!(
            request.target(),
            &EffectTarget::process("cargo check --locked")
        );
    }

    #[test]
    fn invalid_path_is_rejected_before_planning() {
        assert!(
            CodingRequest::read(context(), "../outside.txt")
                .into_gene_input()
                .is_err()
        );
    }

    #[test]
    fn oversized_patch_is_rejected_before_planning() {
        let content = "x".repeat(MAX_PATCH_BYTES + 1);

        assert!(
            CodingRequest::patch(context(), "src/lib.rs", content)
                .into_gene_input()
                .is_err()
        );
    }

    #[test]
    fn unsupported_verification_command_is_rejected() {
        let request = CodingRequest::verify(context()).with_command("sh -c unsafe");

        assert!(request.into_gene_input().is_err());
    }
}
