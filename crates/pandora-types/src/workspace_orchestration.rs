use crate::{
    HarnessId, MetaComposition, OrchestrationPlan, OrchestrationRunId, ReceiptId, RepositoryId,
    RequestDigest, RoleId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceOrchestrationError {
    EmptyRepositories,
    DuplicateRepository(RepositoryId),
    DuplicateRoleBinding(RoleId),
    UnknownRole(RoleId),
    UnknownRepository(RepositoryId),
    MissingRoleBinding(RoleId),
    DomainNotAllowed(HarnessId),
    HandoffLimitExceeded,
    InvalidPlan(crate::OrchestrationContractError),
    InvalidMetaComposition(crate::harness::ManifestError),
    EmptyExactCommit,
    EmptyReceiptEvidence,
    DuplicateGovernedEffectReceipt(ReceiptId),
    ReceiptRunMismatch,
    ReceiptRoleMismatch(RoleId),
    ReceiptRepositoryMismatch(RepositoryId),
    ReceiptWorkspaceMismatch(WorkspaceId),
    ReceiptCommitMismatch,
}

impl fmt::Display for WorkspaceOrchestrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRepositories => formatter.write_str("orchestration requires a repository"),
            Self::DuplicateRepository(id) => write!(formatter, "repository {id} is duplicated"),
            Self::DuplicateRoleBinding(id) => {
                write!(formatter, "role {id} has multiple repositories")
            }
            Self::UnknownRole(id) => write!(formatter, "repository binding role {id} is unknown"),
            Self::UnknownRepository(id) => {
                write!(formatter, "repository binding {id} is unknown")
            }
            Self::MissingRoleBinding(id) => write!(formatter, "role {id} has no repository binding"),
            Self::DomainNotAllowed(id) => {
                write!(formatter, "Meta Harness does not allow Domain Harness {id}")
            }
            Self::HandoffLimitExceeded => {
                formatter.write_str("plan exceeds the Meta Harness handoff limit")
            }
            Self::InvalidPlan(error) => error.fmt(formatter),
            Self::InvalidMetaComposition(error) => error.fmt(formatter),
            Self::EmptyExactCommit => {
                formatter.write_str("repository exact commit cannot be empty")
            }
            Self::EmptyReceiptEvidence => formatter
                .write_str("role receipt requires evidence or governed effect receipts"),
            Self::DuplicateGovernedEffectReceipt(id) => {
                write!(formatter, "governed effect receipt {id} is duplicated")
            }
            Self::ReceiptRunMismatch => {
                formatter.write_str("role receipt belongs to another orchestration run")
            }
            Self::ReceiptRoleMismatch(id) => {
                write!(formatter, "role receipt does not match role {id}")
            }
            Self::ReceiptRepositoryMismatch(id) => {
                write!(formatter, "role receipt does not match repository {id}")
            }
            Self::ReceiptWorkspaceMismatch(id) => {
                write!(formatter, "role receipt does not match workspace {id}")
            }
            Self::ReceiptCommitMismatch => formatter
                .write_str("role receipt does not match the exact repository commit"),
        }
    }
}

impl std::error::Error for WorkspaceOrchestrationError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryBinding {
    repository_id: RepositoryId,
    workspace_id: WorkspaceId,
    exact_commit: String,
}

impl RepositoryBinding {
    pub fn new(
        repository_id: RepositoryId,
        workspace_id: WorkspaceId,
        exact_commit: impl Into<String>,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        let exact_commit = exact_commit.into();
        if exact_commit.trim().is_empty() {
            return Err(WorkspaceOrchestrationError::EmptyExactCommit);
        }
        Ok(Self {
            repository_id,
            workspace_id,
            exact_commit,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn exact_commit(&self) -> &str {
        &self.exact_commit
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleRepositoryBinding {
    role_id: RoleId,
    repository_id: RepositoryId,
}

impl RoleRepositoryBinding {
    pub fn new(role_id: RoleId, repository_id: RepositoryId) -> Self {
        Self {
            role_id,
            repository_id,
        }
    }

    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GovernedOrchestrationPlan {
    plan: OrchestrationPlan,
    meta_composition: MetaComposition,
    repositories: Vec<RepositoryBinding>,
    role_repositories: Vec<RoleRepositoryBinding>,
}

impl GovernedOrchestrationPlan {
    pub fn new(
        plan: OrchestrationPlan,
        meta_composition: MetaComposition,
        repositories: Vec<RepositoryBinding>,
        role_repositories: Vec<RoleRepositoryBinding>,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        let rebuilt_plan = OrchestrationPlan::new(
            plan.id().clone(),
            plan.roles().to_vec(),
            plan.max_parallelism(),
            plan.max_handoffs(),
            plan.handoffs().to_vec(),
        )
        .map_err(WorkspaceOrchestrationError::InvalidPlan)?;
        if rebuilt_plan != plan {
            return Err(WorkspaceOrchestrationError::InvalidPlan(
                crate::OrchestrationContractError::DependencyCycle,
            ));
        }
        meta_composition
            .validate()
            .map_err(WorkspaceOrchestrationError::InvalidMetaComposition)?;
        if repositories.is_empty() {
            return Err(WorkspaceOrchestrationError::EmptyRepositories);
        }
        if plan.handoffs().len() as u32 > meta_composition.max_handoffs() {
            return Err(WorkspaceOrchestrationError::HandoffLimitExceeded);
        }
        for role in plan.roles() {
            if !meta_composition.allows_domain(role.harness_id()) {
                return Err(WorkspaceOrchestrationError::DomainNotAllowed(
                    role.harness_id().clone(),
                ));
            }
        }
        let mut repositories_by_id = BTreeMap::new();
        for repository in &repositories {
            if repositories_by_id
                .insert(repository.repository_id().clone(), repository)
                .is_some()
            {
                return Err(WorkspaceOrchestrationError::DuplicateRepository(
                    repository.repository_id().clone(),
                ));
            }
        }
        let mut roles = BTreeSet::new();
        for binding in &role_repositories {
            if plan.role(binding.role_id()).is_none() {
                return Err(WorkspaceOrchestrationError::UnknownRole(
                    binding.role_id().clone(),
                ));
            }
            if !repositories_by_id.contains_key(binding.repository_id()) {
                return Err(WorkspaceOrchestrationError::UnknownRepository(
                    binding.repository_id().clone(),
                ));
            }
            if !roles.insert(binding.role_id().clone()) {
                return Err(WorkspaceOrchestrationError::DuplicateRoleBinding(
                    binding.role_id().clone(),
                ));
            }
        }
        if let Some(role) = plan
            .roles()
            .iter()
            .find(|role| !roles.contains(role.id()))
        {
            return Err(WorkspaceOrchestrationError::MissingRoleBinding(
                role.id().clone(),
            ));
        }
        Ok(Self {
            plan,
            meta_composition,
            repositories,
            role_repositories,
        })
    }

    pub fn validate(&self) -> Result<(), WorkspaceOrchestrationError> {
        Self::new(
            self.plan.clone(),
            self.meta_composition.clone(),
            self.repositories.clone(),
            self.role_repositories.clone(),
        )
        .map(|_| ())
    }

    pub fn plan(&self) -> &OrchestrationPlan {
        &self.plan
    }

    pub fn meta_composition(&self) -> &MetaComposition {
        &self.meta_composition
    }

    pub fn repositories(&self) -> &[RepositoryBinding] {
        &self.repositories
    }

    pub fn role_repositories(&self) -> &[RoleRepositoryBinding] {
        &self.role_repositories
    }

    pub fn repository_for_role(&self, role_id: &RoleId) -> Option<&RepositoryBinding> {
        let repository_id = self
            .role_repositories
            .iter()
            .find(|binding| binding.role_id() == role_id)?
            .repository_id();
        self.repositories
            .iter()
            .find(|repository| repository.repository_id() == repository_id)
    }

    pub fn validate_receipt(
        &self,
        run_id: &OrchestrationRunId,
        role_id: &RoleId,
        receipt: &OrchestrationRoleReceipt,
    ) -> Result<(), WorkspaceOrchestrationError> {
        receipt.validate()?;
        if receipt.run_id() != run_id {
            return Err(WorkspaceOrchestrationError::ReceiptRunMismatch);
        }
        if receipt.role_id() != role_id {
            return Err(WorkspaceOrchestrationError::ReceiptRoleMismatch(
                role_id.clone(),
            ));
        }
        let repository = self
            .repository_for_role(role_id)
            .ok_or_else(|| WorkspaceOrchestrationError::MissingRoleBinding(role_id.clone()))?;
        if receipt.repository_id() != repository.repository_id() {
            return Err(WorkspaceOrchestrationError::ReceiptRepositoryMismatch(
                repository.repository_id().clone(),
            ));
        }
        if receipt.workspace_id() != repository.workspace_id() {
            return Err(WorkspaceOrchestrationError::ReceiptWorkspaceMismatch(
                repository.workspace_id().clone(),
            ));
        }
        if receipt.exact_commit() != repository.exact_commit() {
            return Err(WorkspaceOrchestrationError::ReceiptCommitMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationRoleReceipt {
    receipt_id: ReceiptId,
    run_id: OrchestrationRunId,
    role_id: RoleId,
    repository_id: RepositoryId,
    workspace_id: WorkspaceId,
    exact_commit: String,
    governed_effect_receipts: Vec<ReceiptId>,
    evidence_digest: Option<RequestDigest>,
}

impl OrchestrationRoleReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        receipt_id: ReceiptId,
        run_id: OrchestrationRunId,
        role_id: RoleId,
        repository_id: RepositoryId,
        workspace_id: WorkspaceId,
        exact_commit: impl Into<String>,
        governed_effect_receipts: Vec<ReceiptId>,
        evidence_digest: Option<RequestDigest>,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        let receipt = Self {
            receipt_id,
            run_id,
            role_id,
            repository_id,
            workspace_id,
            exact_commit: exact_commit.into(),
            governed_effect_receipts,
            evidence_digest,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<(), WorkspaceOrchestrationError> {
        if self.exact_commit.trim().is_empty() {
            return Err(WorkspaceOrchestrationError::EmptyExactCommit);
        }
        if self.governed_effect_receipts.is_empty() && self.evidence_digest.is_none() {
            return Err(WorkspaceOrchestrationError::EmptyReceiptEvidence);
        }
        let mut receipts = BTreeSet::new();
        if let Some(receipt) = self
            .governed_effect_receipts
            .iter()
            .find(|receipt| !receipts.insert(*receipt))
        {
            return Err(WorkspaceOrchestrationError::DuplicateGovernedEffectReceipt(
                (*receipt).clone(),
            ));
        }
        Ok(())
    }

    pub fn receipt_id(&self) -> &ReceiptId {
        &self.receipt_id
    }

    pub fn run_id(&self) -> &OrchestrationRunId {
        &self.run_id
    }

    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn exact_commit(&self) -> &str {
        &self.exact_commit
    }

    pub fn governed_effect_receipts(&self) -> &[ReceiptId] {
        &self.governed_effect_receipts
    }

    pub fn evidence_digest(&self) -> Option<&RequestDigest> {
        self.evidence_digest.as_ref()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Handoff, OrchestrationRole, PlanId, RoleAssignment,
    };

    fn role(id: &str, harness: &str, dependencies: &[&str]) -> RoleAssignment {
        RoleAssignment::new(
            RoleId::new(id).unwrap(),
            OrchestrationRole::Custom(id.to_owned()),
            HarnessId::new(harness).unwrap(),
            dependencies
                .iter()
                .map(|dependency| RoleId::new(*dependency).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn governed_plan() -> GovernedOrchestrationPlan {
        let plan = OrchestrationPlan::new(
            PlanId::new("multi-repository").unwrap(),
            vec![
                role("planner", "coding-domain", &[]),
                role("maker", "design-domain", &["planner"]),
            ],
            2,
            1,
            vec![Handoff::new(
                RoleId::new("planner").unwrap(),
                RoleId::new("maker").unwrap(),
                Some(HarnessId::new("coordination-meta").unwrap()),
            )],
        )
        .unwrap();
        GovernedOrchestrationPlan::new(
            plan,
            MetaComposition::new(
                vec![
                    HarnessId::new("coding-domain").unwrap(),
                    HarnessId::new("design-domain").unwrap(),
                ],
                1,
            )
            .unwrap(),
            vec![
                RepositoryBinding::new(
                    RepositoryId::new("api").unwrap(),
                    WorkspaceId::new("workspace-api").unwrap(),
                    "commit-api",
                )
                .unwrap(),
                RepositoryBinding::new(
                    RepositoryId::new("desktop").unwrap(),
                    WorkspaceId::new("workspace-desktop").unwrap(),
                    "commit-desktop",
                )
                .unwrap(),
            ],
            vec![
                RoleRepositoryBinding::new(
                    RoleId::new("planner").unwrap(),
                    RepositoryId::new("api").unwrap(),
                ),
                RoleRepositoryBinding::new(
                    RoleId::new("maker").unwrap(),
                    RepositoryId::new("desktop").unwrap(),
                ),
            ],
        )
        .unwrap()
    }

    #[test]
    fn every_role_requires_one_explicit_repository() {
        let governed = governed_plan();
        assert_eq!(
            governed
                .repository_for_role(&RoleId::new("maker").unwrap())
                .unwrap()
                .workspace_id()
                .as_str(),
            "workspace-desktop"
        );

        let result = GovernedOrchestrationPlan::new(
            governed.plan().clone(),
            governed.meta_composition().clone(),
            governed.repositories().to_vec(),
            governed.role_repositories()[..1].to_vec(),
        );
        assert_eq!(
            result,
            Err(WorkspaceOrchestrationError::MissingRoleBinding(
                RoleId::new("maker").unwrap()
            ))
        );
    }

    #[test]
    fn receipt_must_match_run_role_repository_workspace_and_commit() {
        let governed = governed_plan();
        let run_id = OrchestrationRunId::new("run-1").unwrap();
        let receipt = OrchestrationRoleReceipt::new(
            ReceiptId::new("role-receipt-1").unwrap(),
            run_id.clone(),
            RoleId::new("planner").unwrap(),
            RepositoryId::new("desktop").unwrap(),
            WorkspaceId::new("workspace-desktop").unwrap(),
            "commit-desktop",
            Vec::new(),
            Some(RequestDigest::new("evidence-1").unwrap()),
        )
        .unwrap();
        assert_eq!(
            governed.validate_receipt(
                &run_id,
                &RoleId::new("planner").unwrap(),
                &receipt,
            ),
            Err(WorkspaceOrchestrationError::ReceiptRepositoryMismatch(
                RepositoryId::new("api").unwrap()
            ))
        );
    }
}
