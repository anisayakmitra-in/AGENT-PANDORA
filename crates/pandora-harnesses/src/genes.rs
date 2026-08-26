use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    ArtifactId, Capability, EffectTarget, ExecutionId, ExecutionProfile, Gene, GeneId, GeneInput,
    GeneManifest, Operation, OperationRequest, PrincipalId, ResourceScope, SessionId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

const MAX_PATH_BYTES: usize = 4_096;
const MAX_PATCH_BYTES: usize = 1_048_576;
const VERIFICATION_SPEC: &str = "cargo check --locked";
const TEST_SPEC: &str = "cargo test --locked";
const FORMAT_SPEC: &str = "cargo fmt --all -- --check";
const LINT_SPEC: &str = "cargo clippy --workspace --all-targets --locked -- -D warnings";
const BUILD_SPEC: &str = "cargo build --locked";
const STATUS_SPEC: &str = "git status --short";
const DEBT_MARKERS: [&str; 4] = ["TODO", "FIXME", "HACK", "XXX"];
const CODING_GUIDE: &str = "Daedalus inventories the workspace for evidence-led audits.\nArgus reviews one scoped change.\nAriadne finds explicit debt markers.\nHephaestus measures the repository with the fixed verifier.\nAthena explains the governed coding workflow.\nAll filesystem and process effects require Pandora permits and receipts.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAction {
    Audit,
    Read,
    Search,
    Patch,
    Verify,
    Test,
    Format,
    Lint,
    Build,
    Status,
    Review,
    DeepReview,
    Debt,
    Measure,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanningContext {
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    workspace_id: WorkspaceId,
    execution_profile: ExecutionProfile,
    artifact_id: Option<ArtifactId>,
}

impl PlanningContext {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        workspace_id: WorkspaceId,
        execution_profile: ExecutionProfile,
    ) -> Self {
        Self {
            execution_id,
            session_id,
            principal_id,
            workspace_id,
            execution_profile,
            artifact_id: None,
        }
    }

    pub fn with_artifact(mut self, artifact_id: ArtifactId) -> Self {
        self.artifact_id = Some(artifact_id);
        self
    }

    pub(crate) fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub(crate) fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub(crate) fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub(crate) fn execution_profile(&self) -> &ExecutionProfile {
        &self.execution_profile
    }

    pub(crate) fn artifact_id(&self) -> Option<&ArtifactId> {
        self.artifact_id.as_ref()
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
    pub fn audit(context: PlanningContext) -> Self {
        Self::without_path(CodingAction::Audit, context)
    }

    pub fn read(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Read, context, path)
    }

    pub fn search(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Search, context, path)
    }

    pub fn review(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::Review, context, path)
    }

    pub fn argus_review(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::for_path(CodingAction::DeepReview, context, path)
    }

    pub fn debt(context: PlanningContext) -> Self {
        Self::without_path(CodingAction::Debt, context)
    }

    pub fn measure(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Measure,
            context,
            path: None,
            content: None,
            command: Some(VERIFICATION_SPEC.to_owned()),
        }
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::without_path(CodingAction::Guide, context)
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

    pub fn test(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Test,
            context,
            path: None,
            content: None,
            command: Some(TEST_SPEC.to_owned()),
        }
    }

    pub fn format(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Format,
            context,
            path: None,
            content: None,
            command: Some(FORMAT_SPEC.to_owned()),
        }
    }

    pub fn lint(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Lint,
            context,
            path: None,
            content: None,
            command: Some(LINT_SPEC.to_owned()),
        }
    }

    pub fn build(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Build,
            context,
            path: None,
            content: None,
            command: Some(BUILD_SPEC.to_owned()),
        }
    }

    pub fn status(context: PlanningContext) -> Self {
        Self {
            action: CodingAction::Status,
            context,
            path: None,
            content: None,
            command: Some(STATUS_SPEC.to_owned()),
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

    fn without_path(action: CodingAction, context: PlanningContext) -> Self {
        Self {
            action,
            context,
            path: None,
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
    Test,
    Format,
    Lint,
    Build,
    Status,
    Review,
    DaedalusAudit,
    ArgusReview,
    AriadneDebt,
    HephaestusMeasure,
    AthenaGuide,
}

impl CodingGeneRole {
    const fn action(self) -> CodingAction {
        match self {
            Self::Read => CodingAction::Read,
            Self::Search => CodingAction::Search,
            Self::Patch => CodingAction::Patch,
            Self::Verify => CodingAction::Verify,
            Self::Test => CodingAction::Test,
            Self::Format => CodingAction::Format,
            Self::Lint => CodingAction::Lint,
            Self::Build => CodingAction::Build,
            Self::Status => CodingAction::Status,
            Self::Review => CodingAction::Review,
            Self::DaedalusAudit => CodingAction::Audit,
            Self::ArgusReview => CodingAction::DeepReview,
            Self::AriadneDebt => CodingAction::Debt,
            Self::HephaestusMeasure => CodingAction::Measure,
            Self::AthenaGuide => CodingAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Read => "workspace.read",
            Self::Search => "workspace.search",
            Self::Patch => "patch.apply",
            Self::Verify => "verification.run",
            Self::Test => "tests.run",
            Self::Format => "format.check",
            Self::Lint => "lint.check",
            Self::Build => "build.check",
            Self::Status => "workspace.status",
            Self::Review => "change.review",
            Self::DaedalusAudit => "daedalus.audit",
            Self::ArgusReview => "argus.review",
            Self::AriadneDebt => "ariadne.debt",
            Self::HephaestusMeasure => "hephaestus.measure",
            Self::AthenaGuide => "athena.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Patch => Some(Capability::FilesystemWrite),
            Self::Verify
            | Self::Test
            | Self::Format
            | Self::Lint
            | Self::Build
            | Self::Status
            | Self::HephaestusMeasure => Some(Capability::ProcessExecute),
            Self::Read
            | Self::Search
            | Self::Review
            | Self::DaedalusAudit
            | Self::ArgusReview
            | Self::AriadneDebt => Some(Capability::FilesystemRead),
            Self::AthenaGuide => None,
        }
    }

    const fn operation(self) -> Option<Operation> {
        match self {
            Self::Patch => Some(Operation::Write),
            Self::Verify
            | Self::Test
            | Self::Format
            | Self::Lint
            | Self::Build
            | Self::Status
            | Self::HephaestusMeasure => Some(Operation::Execute),
            Self::Read
            | Self::Search
            | Self::Review
            | Self::DaedalusAudit
            | Self::ArgusReview
            | Self::AriadneDebt => Some(Operation::Read),
            Self::AthenaGuide => None,
        }
    }

    const fn kind(self) -> GeneKind {
        match self {
            Self::DaedalusAudit
            | Self::ArgusReview
            | Self::AriadneDebt
            | Self::HephaestusMeasure
            | Self::AthenaGuide => GeneKind::Workflow,
            _ => GeneKind::Tool,
        }
    }
}

pub struct CodingGene {
    manifest: GeneManifest,
    role: CodingGeneRole,
}

impl CodingGene {
    pub fn new(role: CodingGeneRole) -> Result<Self, GeneError> {
        let manifest = GeneManifest::new(
            role.id(),
            "0.1.0",
            role.kind(),
            role.capability().into_iter().collect(),
        )?;
        Ok(Self { manifest, role })
    }

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            CodingGeneRole::Read,
            CodingGeneRole::Search,
            CodingGeneRole::Patch,
            CodingGeneRole::Verify,
            CodingGeneRole::Test,
            CodingGeneRole::Format,
            CodingGeneRole::Lint,
            CodingGeneRole::Build,
            CodingGeneRole::Status,
            CodingGeneRole::Review,
            CodingGeneRole::DaedalusAudit,
            CodingGeneRole::ArgusReview,
            CodingGeneRole::AriadneDebt,
            CodingGeneRole::HephaestusMeasure,
            CodingGeneRole::AthenaGuide,
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
        if self.role == CodingGeneRole::AthenaGuide {
            return Ok(Vec::new());
        }
        if self.role == CodingGeneRole::AriadneDebt {
            return DEBT_MARKERS
                .into_iter()
                .map(|marker| self.operation_request(&request, EffectTarget::path(marker)))
                .collect();
        }
        let target = match self.role {
            CodingGeneRole::Verify | CodingGeneRole::HephaestusMeasure => {
                EffectTarget::process(VERIFICATION_SPEC)
            }
            CodingGeneRole::Test => EffectTarget::process(TEST_SPEC),
            CodingGeneRole::Format => EffectTarget::process(FORMAT_SPEC),
            CodingGeneRole::Lint => EffectTarget::process(LINT_SPEC),
            CodingGeneRole::Build => EffectTarget::process(BUILD_SPEC),
            CodingGeneRole::Status => EffectTarget::process(STATUS_SPEC),
            CodingGeneRole::DaedalusAudit => EffectTarget::path("."),
            CodingGeneRole::Read
            | CodingGeneRole::Search
            | CodingGeneRole::Review
            | CodingGeneRole::ArgusReview
            | CodingGeneRole::Patch => EffectTarget::path(
                request
                    .path
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("path is required"))?,
            ),
            CodingGeneRole::AriadneDebt | CodingGeneRole::AthenaGuide => unreachable!(),
        };
        let planned = self.operation_request(&request, target)?;
        if self.role == CodingGeneRole::Patch {
            let content = request
                .content
                .as_deref()
                .ok_or(GeneError::InvalidInput("patch content is required"))?;
            return Ok(vec![planned.with_payload_digest(content.as_bytes())?]);
        }
        Ok(vec![planned])
    }
}

impl CodingGene {
    fn operation_request(
        &self,
        request: &CodingRequest,
        target: EffectTarget,
    ) -> Result<OperationRequest, GeneError> {
        Ok(OperationRequest::new(
            request.context.execution_id.clone(),
            request.context.session_id.clone(),
            request.context.principal_id.clone(),
            request.context.execution_profile.clone(),
            self.manifest.id().clone(),
            request.context.artifact_id.clone(),
            self.role
                .capability()
                .ok_or(GeneError::InvalidInput("workflow has no effect capability"))?,
            self.role
                .operation()
                .ok_or(GeneError::InvalidInput("workflow has no effect operation"))?,
            target,
            ResourceScope::workspace(request.context.workspace_id.as_str()),
        )?)
    }
}

pub fn coding_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "athena.guide").then_some(CODING_GUIDE)
}

fn validate_request(request: &CodingRequest) -> Result<(), GeneError> {
    match request.action {
        CodingAction::Audit | CodingAction::Debt | CodingAction::Guide => {
            if request.path.is_some() || request.content.is_some() || request.command.is_some() {
                return Err(GeneError::InvalidInput(
                    "path, content, and command are not valid for this workflow",
                ));
            }
        }
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
        CodingAction::Read | CodingAction::Review | CodingAction::DeepReview => {
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
        CodingAction::Verify
        | CodingAction::Test
        | CodingAction::Format
        | CodingAction::Lint
        | CodingAction::Build
        | CodingAction::Status
        | CodingAction::Measure => {
            let expected_command = match request.action {
                CodingAction::Test => TEST_SPEC,
                CodingAction::Format => FORMAT_SPEC,
                CodingAction::Lint => LINT_SPEC,
                CodingAction::Build => BUILD_SPEC,
                CodingAction::Status => STATUS_SPEC,
                CodingAction::Verify | CodingAction::Measure => VERIFICATION_SPEC,
                _ => unreachable!(),
            };
            if request.command.as_deref() != Some(expected_command) {
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

pub(crate) fn validate_query(query: &str) -> Result<(), GeneError> {
    if query.trim().is_empty() || query.len() > MAX_PATH_BYTES || query.contains('\0') {
        return Err(GeneError::InvalidInput("invalid search query"));
    }
    Ok(())
}

pub(crate) fn validate_path(path: &str) -> Result<(), GeneError> {
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
        EffectTarget, ExecutionId, ExecutionProfile, ExecutionProfileBinding,
        ExecutionProfileBindingKind, Gene, Operation, PrincipalId, SessionId, WorkspaceId,
        hash_artifact,
    };

    fn execution_profile() -> ExecutionProfile {
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
        .unwrap()
    }

    fn context() -> PlanningContext {
        PlanningContext::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            execution_profile(),
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
    fn test_gene_plans_the_fixed_test_command() {
        let gene = CodingGene::new(CodingGeneRole::Test).unwrap();
        let input = CodingRequest::test(context()).into_gene_input().unwrap();
        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::ProcessExecute);
        assert_eq!(request.target(), &EffectTarget::process(TEST_SPEC));
    }

    #[test]
    fn format_gene_plans_the_fixed_format_check() {
        let gene = CodingGene::new(CodingGeneRole::Format).unwrap();
        let input = CodingRequest::format(context()).into_gene_input().unwrap();
        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::ProcessExecute);
        assert_eq!(request.target(), &EffectTarget::process(FORMAT_SPEC));
    }

    #[test]
    fn lint_gene_plans_the_fixed_lint_check() {
        let gene = CodingGene::new(CodingGeneRole::Lint).unwrap();
        let input = CodingRequest::lint(context()).into_gene_input().unwrap();
        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::ProcessExecute);
        assert_eq!(request.target(), &EffectTarget::process(LINT_SPEC));
    }

    #[test]
    fn build_gene_plans_the_fixed_build_check() {
        let gene = CodingGene::new(CodingGeneRole::Build).unwrap();
        let input = CodingRequest::build(context()).into_gene_input().unwrap();
        let request = &gene.plan(&input).unwrap()[0];

        assert_eq!(request.capability(), Capability::ProcessExecute);
        assert_eq!(request.target(), &EffectTarget::process(BUILD_SPEC));
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

    #[test]
    fn daedalus_audit_requests_a_bounded_workspace_inventory() {
        let gene = CodingGene::new(CodingGeneRole::DaedalusAudit).unwrap();
        let requests = gene
            .plan(&CodingRequest::audit(context()).into_gene_input().unwrap())
            .unwrap();

        assert_eq!(gene.manifest().kind(), GeneKind::Workflow);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability(), Capability::FilesystemRead);
        assert_eq!(requests[0].target(), &EffectTarget::path("."));
    }

    #[test]
    fn ariadne_debt_uses_only_the_fixed_evidence_markers() {
        let gene = CodingGene::new(CodingGeneRole::AriadneDebt).unwrap();
        let requests = gene
            .plan(&CodingRequest::debt(context()).into_gene_input().unwrap())
            .unwrap();
        let targets = requests
            .iter()
            .map(|request| match request.target() {
                EffectTarget::Path { path } => path.as_str(),
                _ => panic!("debt discovery must remain read-only"),
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, ["TODO", "FIXME", "HACK", "XXX"]);
        assert!(
            requests
                .iter()
                .all(|request| request.capability() == Capability::FilesystemRead)
        );
    }

    #[test]
    fn athena_guide_is_a_pure_workflow_with_bounded_static_output() {
        let gene = CodingGene::new(CodingGeneRole::AthenaGuide).unwrap();
        let requests = gene
            .plan(&CodingRequest::guide(context()).into_gene_input().unwrap())
            .unwrap();

        assert!(requests.is_empty());
        assert!(coding_static_output(gene.manifest().id()).is_some());
        assert!(gene.manifest().capabilities().is_empty());
    }

    #[test]
    fn hephaestus_measure_uses_only_the_fixed_verifier() {
        let gene = CodingGene::new(CodingGeneRole::HephaestusMeasure).unwrap();
        let requests = gene
            .plan(&CodingRequest::measure(context()).into_gene_input().unwrap())
            .unwrap();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability(), Capability::ProcessExecute);
        assert_eq!(
            requests[0].target(),
            &EffectTarget::process("cargo check --locked")
        );
    }
}
