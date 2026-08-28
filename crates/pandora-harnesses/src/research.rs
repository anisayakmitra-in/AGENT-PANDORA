use crate::PlanningContext;
use crate::genes::{validate_path, validate_query};
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};
use url::{Host, Url};

const CITATION_MARKERS: [&str; 3] = ["http://", "https://", "doi:"];
const RESEARCH_GUIDE: &str = "Evidence inventory maps the bounded workspace.\nEvidence search locates relevant local material.\nSource read and source compare preserve exact evidence.\nBrowser fetch retrieves one exact, approval-bound text URL without following redirects.\nCitation inventory finds explicit URL and DOI markers without claiming they are valid.\nAll filesystem and network effects require Pandora permits and receipts.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchAction {
    Inventory,
    Search,
    Read,
    Compare,
    Fetch,
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
    url: Option<String>,
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
            url: None,
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

    pub fn fetch(context: PlanningContext, url: impl Into<String>) -> Self {
        Self {
            action: ResearchAction::Fetch,
            context,
            query: None,
            paths: Vec::new(),
            url: Some(url.into()),
        }
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
            url: None,
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
            url: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchGeneRole {
    EvidenceInventory,
    EvidenceSearch,
    SourceRead,
    SourceCompare,
    BrowserFetch,
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
            Self::BrowserFetch => ResearchAction::Fetch,
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
            Self::BrowserFetch => "browser.fetch",
            Self::CitationInventory => "citation.inventory",
            Self::ResearchGuide => "research.guide",
        }
    }

    const fn kind(self) -> GeneKind {
        match self {
            Self::EvidenceSearch | Self::SourceRead | Self::BrowserFetch => GeneKind::Tool,
            Self::EvidenceInventory
            | Self::SourceCompare
            | Self::CitationInventory
            | Self::ResearchGuide => GeneKind::Workflow,
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::ResearchGuide => None,
            Self::BrowserFetch => Some(Capability::NetworkConnect),
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
            ResearchGeneRole::BrowserFetch,
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

        if self.role == ResearchGeneRole::BrowserFetch {
            let source = request
                .url
                .as_deref()
                .ok_or(GeneError::InvalidInput("URL is required"))?;
            let (host, port) = validate_fetch_url(source)?;
            let operation = OperationRequest::new(
                request.context.execution_id().clone(),
                request.context.session_id().clone(),
                request.context.principal_id().clone(),
                request.context.execution_profile().clone(),
                self.manifest.id().clone(),
                request.context.artifact_id().cloned(),
                Capability::NetworkConnect,
                Operation::Connect,
                EffectTarget::network(host.clone(), port),
                ResourceScope::host(host),
            )?
            .with_payload_digest(source.as_bytes())?;
            return Ok(vec![operation]);
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
            ResearchGeneRole::BrowserFetch => unreachable!("browser fetch is planned above"),
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
            | "browser.fetch"
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
            if request.query.is_some() || !request.paths.is_empty() || request.url.is_some() {
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
            if !request.paths.is_empty() || request.url.is_some() {
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
        ResearchAction::Fetch => {
            if request.query.is_some() || !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "query and paths are not valid for browser fetch",
                ));
            }
            validate_fetch_url(
                request
                    .url
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("URL is required"))?,
            )?;
        }
    }
    Ok(())
}

fn validate_paths(request: &ResearchRequest, expected: usize) -> Result<(), GeneError> {
    if request.query.is_some() || request.url.is_some() || request.paths.len() != expected {
        return Err(GeneError::InvalidInput(
            "research action has an invalid path count",
        ));
    }
    for path in &request.paths {
        validate_path(path)?;
    }
    Ok(())
}

fn validate_fetch_url(value: &str) -> Result<(String, u16), GeneError> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(GeneError::InvalidInput(
            "browser URL is invalid or too long",
        ));
    }
    let parsed =
        Url::parse(value).map_err(|_| GeneError::InvalidInput("browser URL is invalid"))?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(GeneError::InvalidInput(
            "browser URL cannot contain credentials, query data, or a fragment",
        ));
    }
    let host = parsed
        .host()
        .ok_or(GeneError::InvalidInput("browser URL requires a host"))?;
    let loopback = match host {
        Host::Domain(value) => value.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(value) => value.is_loopback(),
        Host::Ipv6(value) => value.is_loopback(),
    };
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        return Err(GeneError::InvalidInput(
            "browser URL must use HTTPS; HTTP is allowed only for loopback",
        ));
    }
    let port = parsed
        .port_or_known_default()
        .ok_or(GeneError::InvalidInput("browser URL requires a valid port"))?;
    Ok((
        parsed
            .host_str()
            .ok_or(GeneError::InvalidInput("browser URL requires a host"))?
            .to_owned(),
        port,
    ))
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

    #[test]
    fn browser_fetch_binds_the_exact_url_payload_and_host_scope() {
        let gene = ResearchGene::new(ResearchGeneRole::BrowserFetch).unwrap();
        let source = "https://example.test/docs";
        let requests = gene
            .plan(
                &ResearchRequest::fetch(context(), source)
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability(), Capability::NetworkConnect);
        assert_eq!(requests[0].operation(), Operation::Connect);
        assert_eq!(
            requests[0].target(),
            &EffectTarget::network("example.test", 443)
        );
        assert_eq!(
            requests[0].resource_scope(),
            &ResourceScope::host("example.test")
        );
        assert!(requests[0].payload_digest_matches(source.as_bytes()));
    }

    #[test]
    fn browser_fetch_rejects_unsafe_or_secret_bearing_urls() {
        for source in [
            "http://example.test/docs",
            "https://token@example.test/docs",
            "https://example.test/docs?token=secret",
            "file:///etc/passwd",
        ] {
            assert!(
                ResearchRequest::fetch(context(), source)
                    .into_gene_input()
                    .is_err()
            );
        }
        assert!(
            ResearchRequest::fetch(context(), "http://127.0.0.1:5173/")
                .into_gene_input()
                .is_ok()
        );
    }
}
