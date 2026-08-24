use crate::PlanningContext;
use crate::genes::{validate_path, validate_query};
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const CITATION_MARKERS: [&str; 3] = ["http://", "https://", "doi:"];
const RESEARCH_GUIDE: &str = "Evidence inventory maps the bounded workspace.\nEvidence search locates relevant local material.\nSource read and source compare preserve exact evidence.\nCitation inventory finds explicit URL and DOI markers without claiming they are valid.\nAll filesystem effects require Pandora permits and receipts; network retrieval requires a separately governed tool.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchAction {
    Inventory,
    Search,
    Read,
    Compare,
    CitationInventory,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchRequest {
    action: ResearchAction,
    context: PlanningContext,
    query: Option<String>,
    paths: Vec<String>,
}

impl ResearchRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::empty(ResearchAction::Inventory, context)
    }

    pub fn search(context: PlanningContext, query: impl Into<String>) -> Self {
        Self {
            action: ResearchAction::Search,
            context,
            query: Some(query.into()),
            paths: Vec::new(),
        }
    }

    pub fn read(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::paths(ResearchAction::Read, context, [path.into()])
    }

    pub fn compare(
        context: PlanningContext,
        left: impl Into<String>,
        right: impl Into<String>,
    ) -> Self {
        Self::paths(
            ResearchAction::Compare,
            context,
            [left.into(), right.into()],
        )
    }

    pub fn citation_inventory(context: PlanningContext) -> Self {
        Self::empty(ResearchAction::CitationInventory, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::empty(ResearchAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        validate_request(&self)?;
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("research input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("research input must be valid JSON"))
    }

    fn empty(action: ResearchAction, context: PlanningContext) -> Self {
        Self {
            action,
            context,
            query: None,
            paths: Vec::new(),
        }
    }

    fn paths(
        action: ResearchAction,
        context: PlanningContext,
        paths: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            action,
            context,
            query: None,
            paths: paths.into_iter().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchGeneRole {
    EvidenceInventory,
    EvidenceSearch,
    SourceRead,
    SourceCompare,
    CitationInventory,
    ResearchGuide,
}

impl ResearchGeneRole {
    const fn action(self) -> ResearchAction {
        match self {
            Self::EvidenceInventory => ResearchAction::Inventory,
            Self::EvidenceSearch => ResearchAction::Search,
            Self::SourceRead => ResearchAction::Read,
            Self::SourceCompare => ResearchAction::Compare,
            Self::CitationInventory => ResearchAction::CitationInventory,
            Self::ResearchGuide => ResearchAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::EvidenceInventory => "evidence.inventory",
            Self::EvidenceSearch => "evidence.search",
            Self::SourceRead => "source.read",
            Self::SourceCompare => "source.compare",
            Self::CitationInventory => "citation.inventory",
            Self::ResearchGuide => "research.guide",
        }
    }

    const fn kind(self) -> GeneKind {
        match self {
            Self::EvidenceSearch | Self::SourceRead => GeneKind::Tool,
            Self::EvidenceInventory
            | Self::SourceCompare
            | Self::CitationInventory
            | Self::ResearchGuide => GeneKind::Workflow,
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::ResearchGuide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct ResearchGene {
    manifest: GeneManifest,
    role: ResearchGeneRole,
}

impl ResearchGene {
    pub fn new(role: ResearchGeneRole) -> Result<Self, GeneError> {
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
            ResearchGeneRole::EvidenceInventory,
            ResearchGeneRole::EvidenceSearch,
            ResearchGeneRole::SourceRead,
            ResearchGeneRole::SourceCompare,
            ResearchGeneRole::CitationInventory,
            ResearchGeneRole::ResearchGuide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Research Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }
}

impl Gene for ResearchGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = ResearchRequest::parse(input)?;
        validate_request(&request)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "research input action does not match Gene",
            ));
        }

        let targets = match self.role {
            ResearchGeneRole::EvidenceInventory => vec![EffectTarget::path(".")],
            ResearchGeneRole::EvidenceSearch => vec![EffectTarget::path(
                request
                    .query
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("query is required"))?,
            )],
            ResearchGeneRole::SourceRead | ResearchGeneRole::SourceCompare => request
                .paths
                .iter()
                .map(|path| EffectTarget::path(path.clone()))
                .collect(),
            ResearchGeneRole::CitationInventory => CITATION_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            ResearchGeneRole::ResearchGuide => return Ok(Vec::new()),
        };

        targets
            .into_iter()
            .map(|target| self.operation_request(&request, target))
            .collect()
    }
}

impl ResearchGene {
    fn operation_request(
        &self,
        request: &ResearchRequest,
        target: EffectTarget,
    ) -> Result<OperationRequest, GeneError> {
        Ok(OperationRequest::new(
            request.context.execution_id().clone(),
            request.context.session_id().clone(),
            request.context.principal_id().clone(),
            request.context.execution_profile().clone(),
            self.manifest.id().clone(),
            request.context.artifact_id().cloned(),
            Capability::FilesystemRead,
            Operation::Read,
            target,
            ResourceScope::workspace(request.context.workspace_id().as_str()),
        )?)
    }
}

pub fn is_research_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "evidence.inventory"
            | "evidence.search"
            | "source.read"
            | "source.compare"
            | "citation.inventory"
            | "research.guide"
    )
}

pub fn research_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "research.guide").then_some(RESEARCH_GUIDE)
}

fn validate_request(request: &ResearchRequest) -> Result<(), GeneError> {
    match request.action {
        ResearchAction::Inventory | ResearchAction::CitationInventory | ResearchAction::Guide => {
            if request.query.is_some() || !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "query and paths are not valid for this research action",
                ));
            }
        }
        ResearchAction::Search => {
            validate_query(
                request
                    .query
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("query is required"))?,
            )?;
            if !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "paths are not valid for evidence search",
                ));
            }
        }
        ResearchAction::Read => validate_paths(request, 1)?,
        ResearchAction::Compare => {
            validate_paths(request, 2)?;
            if request.paths.iter().any(|path| path.contains('|')) {
                return Err(GeneError::InvalidInput(
                    "source comparison paths cannot contain the delimiter",
                ));
            }
            if request.paths[0] == request.paths[1] {
                return Err(GeneError::InvalidInput(
                    "source comparison requires two distinct paths",
                ));
            }
        }
    }
    Ok(())
}

fn validate_paths(request: &ResearchRequest, expected: usize) -> Result<(), GeneError> {
    if request.query.is_some() || request.paths.len() != expected {
        return Err(GeneError::InvalidInput(
            "research action has an invalid path count",
        ));
    }
    for path in &request.paths {
        validate_path(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        ExecutionId, ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind,
        PrincipalId, SessionId, WorkspaceId, hash_artifact,
    };

    fn context() -> PlanningContext {
        let profile = ExecutionProfile::new(
            env!("CARGO_PKG_VERSION"),
            "windows",
            "x86_64",
            1,
            "workspace-1",
            hash_artifact(b"containment"),
            vec![
                ExecutionProfileBinding::new(
                    ExecutionProfileBindingKind::Executor,
                    "filesystem",
                    Some(env!("CARGO_PKG_VERSION")),
                    hash_artifact(b"filesystem"),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        PlanningContext::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            profile,
        )
    }

    #[test]
    fn comparison_plans_two_bounded_read_requests() {
        let gene = ResearchGene::new(ResearchGeneRole::SourceCompare).unwrap();
        let input = ResearchRequest::compare(context(), "first.md", "second.md")
            .into_gene_input()
            .unwrap();

        let requests = gene.plan(&input).unwrap();

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].target(), &EffectTarget::path("first.md"));
        assert_eq!(requests[1].target(), &EffectTarget::path("second.md"));
        assert!(
            requests
                .iter()
                .all(|request| request.capability() == Capability::FilesystemRead)
        );
    }

    #[test]
    fn citation_inventory_uses_only_fixed_markers() {
        let gene = ResearchGene::new(ResearchGeneRole::CitationInventory).unwrap();
        let requests = gene
            .plan(
                &ResearchRequest::citation_inventory(context())
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();
        let targets = requests
            .iter()
            .map(|request| match request.target() {
                EffectTarget::Path { path } => path.as_str(),
                _ => panic!("citation inventory must remain filesystem read-only"),
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, CITATION_MARKERS);
    }

    #[test]
    fn comparison_rejects_duplicate_or_escaping_paths() {
        assert!(
            ResearchRequest::compare(context(), "same.md", "same.md")
                .into_gene_input()
                .is_err()
        );
        assert!(
            ResearchRequest::compare(context(), "first|draft.md", "second.md")
                .into_gene_input()
                .is_err()
        );
        assert!(
            ResearchRequest::read(context(), "../outside.md")
                .into_gene_input()
                .is_err()
        );
    }

    #[test]
    fn research_guide_is_pure_static_guidance() {
        let gene = ResearchGene::new(ResearchGeneRole::ResearchGuide).unwrap();
        let requests = gene
            .plan(&ResearchRequest::guide(context()).into_gene_input().unwrap())
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(research_static_output(gene.manifest().id()).is_some());
    }
}
