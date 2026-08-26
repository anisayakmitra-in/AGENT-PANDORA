use crate::PlanningContext;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const SCHEMA_MARKERS: [&str; 8] = [
    "CREATE TABLE",
    "CREATE VIEW",
    "CREATE TYPE",
    "schema",
    "migration",
    "data model",
    "interface",
    "struct",
];
const QUALITY_MARKERS: [&str; 8] = [
    "null",
    "duplicate",
    "missing",
    "outlier",
    "invalid",
    "constraint",
    "unique",
    "validation",
];
const LINEAGE_MARKERS: [&str; 8] = [
    "source",
    "sink",
    "transform",
    "pipeline",
    "ETL",
    "ELT",
    "provenance",
    "lineage",
];
const ANALYSIS_MARKERS: [&str; 8] = [
    "mean",
    "median",
    "aggregate",
    "group by",
    "correlation",
    "distribution",
    "metric",
    "SELECT",
];
const DATA_GUIDE: &str = "Data inventory maps the bounded workspace.\nSchema evidence searches fixed schema and data-model markers.\nQuality evidence searches fixed validation and integrity markers.\nLineage evidence searches source, transformation, pipeline, and provenance markers.\nAnalysis evidence searches fixed statistical and aggregation markers.\nAll filesystem effects require Pandora permits and receipts; database, network, process, and mutation actions require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataAction {
    Inventory,
    Schema,
    Quality,
    Lineage,
    Analysis,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataRequest {
    action: DataAction,
    context: PlanningContext,
}

impl DataRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::new(DataAction::Inventory, context)
    }

    pub fn schema(context: PlanningContext) -> Self {
        Self::new(DataAction::Schema, context)
    }

    pub fn quality(context: PlanningContext) -> Self {
        Self::new(DataAction::Quality, context)
    }

    pub fn lineage(context: PlanningContext) -> Self {
        Self::new(DataAction::Lineage, context)
    }

    pub fn analysis(context: PlanningContext) -> Self {
        Self::new(DataAction::Analysis, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::new(DataAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("data input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("data input must be valid JSON"))
    }

    const fn new(action: DataAction, context: PlanningContext) -> Self {
        Self { action, context }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataGeneRole {
    Inventory,
    Schema,
    Quality,
    Lineage,
    Analysis,
    Guide,
}

impl DataGeneRole {
    const fn action(self) -> DataAction {
        match self {
            Self::Inventory => DataAction::Inventory,
            Self::Schema => DataAction::Schema,
            Self::Quality => DataAction::Quality,
            Self::Lineage => DataAction::Lineage,
            Self::Analysis => DataAction::Analysis,
            Self::Guide => DataAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "data.inventory",
            Self::Schema => "data.schema",
            Self::Quality => "data.quality",
            Self::Lineage => "data.lineage",
            Self::Analysis => "data.analysis",
            Self::Guide => "data.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct DataGene {
    manifest: GeneManifest,
    role: DataGeneRole,
}

impl DataGene {
    pub fn new(role: DataGeneRole) -> Result<Self, GeneError> {
        let manifest = GeneManifest::new(
            role.id(),
            "0.1.0",
            GeneKind::Workflow,
            role.capability().into_iter().collect(),
        )?;
        Ok(Self { manifest, role })
    }

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            DataGeneRole::Inventory,
            DataGeneRole::Schema,
            DataGeneRole::Quality,
            DataGeneRole::Lineage,
            DataGeneRole::Analysis,
            DataGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Data Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &DataRequest,
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

impl Gene for DataGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = DataRequest::parse(input)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "data input action does not match Gene",
            ));
        }
        let targets: &[&str] = match self.role {
            DataGeneRole::Inventory => {
                return Ok(vec![
                    self.operation_request(&request, EffectTarget::path("."))?,
                ]);
            }
            DataGeneRole::Schema => &SCHEMA_MARKERS,
            DataGeneRole::Quality => &QUALITY_MARKERS,
            DataGeneRole::Lineage => &LINEAGE_MARKERS,
            DataGeneRole::Analysis => &ANALYSIS_MARKERS,
            DataGeneRole::Guide => return Ok(Vec::new()),
        };
        targets
            .iter()
            .map(|marker| self.operation_request(&request, EffectTarget::path(*marker)))
            .collect()
    }
}

pub fn is_data_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "data.inventory"
            | "data.schema"
            | "data.quality"
            | "data.lineage"
            | "data.analysis"
            | "data.guide"
    )
}

pub fn data_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "data.guide").then_some(DATA_GUIDE)
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
    fn evidence_roles_use_fixed_read_only_requests() {
        let roles = [
            (DataGeneRole::Schema, &SCHEMA_MARKERS[..]),
            (DataGeneRole::Quality, &QUALITY_MARKERS[..]),
            (DataGeneRole::Lineage, &LINEAGE_MARKERS[..]),
            (DataGeneRole::Analysis, &ANALYSIS_MARKERS[..]),
        ];
        for (role, markers) in roles {
            let gene = DataGene::new(role).unwrap();
            let requests = gene
                .plan(
                    &DataRequest::new(role.action(), context())
                        .into_gene_input()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(requests.len(), markers.len());
            assert!(requests.iter().all(|request| {
                request.capability() == Capability::FilesystemRead
                    && request.operation() == Operation::Read
                    && request.resource_scope() == &ResourceScope::workspace("workspace-1")
            }));
            assert!(requests.iter().all(|request| {
                matches!(request.target(), EffectTarget::Path { path } if markers.contains(&path.as_str()))
            }));
        }
    }

    #[test]
    fn guide_is_pure_static_guidance() {
        let gene = DataGene::new(DataGeneRole::Guide).unwrap();
        let requests = gene
            .plan(&DataRequest::guide(context()).into_gene_input().unwrap())
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(data_static_output(gene.manifest().id()).is_some());
    }

    #[test]
    fn inventory_is_scoped_to_the_workspace_root() {
        let gene = DataGene::new(DataGeneRole::Inventory).unwrap();
        let requests = gene
            .plan(&DataRequest::inventory(context()).into_gene_input().unwrap())
            .unwrap();

        assert_eq!(requests.len(), 1);
        assert!(matches!(
            requests[0].target(),
            EffectTarget::Path { path } if path == "."
        ));
    }
}
