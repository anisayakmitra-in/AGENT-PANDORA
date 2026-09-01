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
    DuplicateRoleBudget(RoleId),
    NonCanonicalRoleBudgets,
    MissingRoleBudget(RoleId),
    UnknownBudgetRole(RoleId),
    RoleBudgetExceedsCeiling(RoleId),
    AggregateBudgetOverflow,
    DuplicateUsageReceipt(ReceiptId),
    NonCanonicalUsageReceipts,
    UsageReceiptNotGoverned(ReceiptId),
    EmptyUsageEvidence,
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
            Self::MissingRoleBinding(id) => {
                write!(formatter, "role {id} has no repository binding")
            }
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
            Self::EmptyReceiptEvidence => {
                formatter.write_str("role receipt requires evidence or governed effect receipts")
            }
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
            Self::ReceiptCommitMismatch => {
                formatter.write_str("role receipt does not match the exact repository commit")
            }
            Self::DuplicateRoleBudget(id) => write!(formatter, "role {id} has multiple budgets"),
            Self::NonCanonicalRoleBudgets => {
                formatter.write_str("role budgets are not in canonical order")
            }
            Self::MissingRoleBudget(id) => write!(formatter, "role {id} has no budget"),
            Self::UnknownBudgetRole(id) => write!(formatter, "budget role {id} is unknown"),
            Self::RoleBudgetExceedsCeiling(id) => {
                write!(formatter, "role {id} budget exceeds the aggregate ceiling")
            }
            Self::AggregateBudgetOverflow => {
                formatter.write_str("aggregate orchestration budget overflowed")
            }
            Self::DuplicateUsageReceipt(id) => {
                write!(formatter, "usage receipt {id} is duplicated")
            }
            Self::NonCanonicalUsageReceipts => {
                formatter.write_str("usage receipts are not in canonical order")
            }
            Self::UsageReceiptNotGoverned(id) => {
                write!(
                    formatter,
                    "usage receipt {id} is not a governed effect receipt"
                )
            }
            Self::EmptyUsageEvidence => {
                formatter.write_str("measured usage requires receipt evidence")
            }
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationBudgetAmount {
    tokens: u64,
    tools: u64,
    elapsed_ms: u64,
    cost_micros: u64,
}

impl OrchestrationBudgetAmount {
    pub const fn new(tokens: u64, tools: u64, elapsed_ms: u64, cost_micros: u64) -> Self {
        Self {
            tokens,
            tools,
            elapsed_ms,
            cost_micros,
        }
    }

    pub const fn tokens(self) -> u64 {
        self.tokens
    }

    pub const fn tools(self) -> u64 {
        self.tools
    }

    pub const fn elapsed_ms(self) -> u64 {
        self.elapsed_ms
    }

    pub const fn cost_micros(self) -> u64 {
        self.cost_micros
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            tokens: self.tokens.checked_add(other.tokens)?,
            tools: self.tools.checked_add(other.tools)?,
            elapsed_ms: self.elapsed_ms.checked_add(other.elapsed_ms)?,
            cost_micros: self.cost_micros.checked_add(other.cost_micros)?,
        })
    }

    pub const fn fits_within(self, ceiling: Self) -> bool {
        self.tokens <= ceiling.tokens
            && self.tools <= ceiling.tools
            && self.elapsed_ms <= ceiling.elapsed_ms
            && self.cost_micros <= ceiling.cost_micros
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationRoleBudget {
    role_id: RoleId,
    reservation: OrchestrationBudgetAmount,
}

impl OrchestrationRoleBudget {
    pub const fn new(role_id: RoleId, reservation: OrchestrationBudgetAmount) -> Self {
        Self {
            role_id,
            reservation,
        }
    }

    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub const fn reservation(&self) -> OrchestrationBudgetAmount {
        self.reservation
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationAggregateBudget {
    ceiling: OrchestrationBudgetAmount,
    roles: Vec<OrchestrationRoleBudget>,
}

impl OrchestrationAggregateBudget {
    pub fn new(
        ceiling: OrchestrationBudgetAmount,
        mut roles: Vec<OrchestrationRoleBudget>,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        roles.sort_by(|left, right| left.role_id().cmp(right.role_id()));
        if let Some(duplicate) = roles
            .windows(2)
            .find(|pair| pair[0].role_id() == pair[1].role_id())
        {
            return Err(WorkspaceOrchestrationError::DuplicateRoleBudget(
                duplicate[0].role_id().clone(),
            ));
        }
        Ok(Self { ceiling, roles })
    }

    pub const fn ceiling(&self) -> OrchestrationBudgetAmount {
        self.ceiling
    }

    pub fn roles(&self) -> &[OrchestrationRoleBudget] {
        &self.roles
    }

    pub fn reservation_for_role(&self, role_id: &RoleId) -> Option<OrchestrationBudgetAmount> {
        self.roles
            .iter()
            .find(|budget| budget.role_id() == role_id)
            .map(OrchestrationRoleBudget::reservation)
    }

    fn validate_for_plan(
        &self,
        plan: &OrchestrationPlan,
    ) -> Result<(), WorkspaceOrchestrationError> {
        let rebuilt = Self::new(self.ceiling, self.roles.clone())?;
        if rebuilt != *self {
            return Err(WorkspaceOrchestrationError::NonCanonicalRoleBudgets);
        }
        let mut total = OrchestrationBudgetAmount::default();
        for budget in &self.roles {
            if plan.role(budget.role_id()).is_none() {
                return Err(WorkspaceOrchestrationError::UnknownBudgetRole(
                    budget.role_id().clone(),
                ));
            }
            if !budget.reservation().fits_within(self.ceiling) {
                return Err(WorkspaceOrchestrationError::RoleBudgetExceedsCeiling(
                    budget.role_id().clone(),
                ));
            }
            total = total
                .checked_add(budget.reservation())
                .ok_or(WorkspaceOrchestrationError::AggregateBudgetOverflow)?;
        }
        if let Some(role) = plan
            .roles()
            .iter()
            .find(|role| self.reservation_for_role(role.id()).is_none())
        {
            return Err(WorkspaceOrchestrationError::MissingRoleBudget(
                role.id().clone(),
            ));
        }
        if !total.fits_within(self.ceiling) {
            return Err(WorkspaceOrchestrationError::AggregateBudgetOverflow);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationUsage {
    tokens: u64,
    tools: u64,
    elapsed_ms: u64,
    cost_micros: Option<u64>,
    source_receipts: Vec<ReceiptId>,
}

impl OrchestrationUsage {
    pub fn new(
        tokens: u64,
        tools: u64,
        elapsed_ms: u64,
        cost_micros: Option<u64>,
        mut source_receipts: Vec<ReceiptId>,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        source_receipts.sort();
        if let Some(duplicate) = source_receipts.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(WorkspaceOrchestrationError::DuplicateUsageReceipt(
                duplicate[0].clone(),
            ));
        }
        if source_receipts.is_empty()
            && (tokens != 0 || tools != 0 || elapsed_ms != 0 || cost_micros.is_none())
        {
            return Err(WorkspaceOrchestrationError::EmptyUsageEvidence);
        }
        Ok(Self {
            tokens,
            tools,
            elapsed_ms,
            cost_micros,
            source_receipts,
        })
    }

    pub fn validate(&self) -> Result<(), WorkspaceOrchestrationError> {
        let rebuilt = Self::new(
            self.tokens,
            self.tools,
            self.elapsed_ms,
            self.cost_micros,
            self.source_receipts.clone(),
        )?;
        if rebuilt != *self {
            return Err(WorkspaceOrchestrationError::NonCanonicalUsageReceipts);
        }
        Ok(())
    }

    pub const fn tokens(&self) -> u64 {
        self.tokens
    }

    pub const fn tools(&self) -> u64 {
        self.tools
    }

    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    pub const fn cost_micros(&self) -> Option<u64> {
        self.cost_micros
    }

    pub fn source_receipts(&self) -> &[ReceiptId] {
        &self.source_receipts
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    aggregate_budget: Option<OrchestrationAggregateBudget>,
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
        if let Some(role) = plan.roles().iter().find(|role| !roles.contains(role.id())) {
            return Err(WorkspaceOrchestrationError::MissingRoleBinding(
                role.id().clone(),
            ));
        }
        Ok(Self {
            plan,
            meta_composition,
            repositories,
            role_repositories,
            aggregate_budget: None,
        })
    }

    pub fn validate(&self) -> Result<(), WorkspaceOrchestrationError> {
        Self::new(
            self.plan.clone(),
            self.meta_composition.clone(),
            self.repositories.clone(),
            self.role_repositories.clone(),
        )
        .and_then(|mut rebuilt| {
            rebuilt.aggregate_budget.clone_from(&self.aggregate_budget);
            if let Some(budget) = &rebuilt.aggregate_budget {
                budget.validate_for_plan(&rebuilt.plan)?;
            }
            Ok(())
        })
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

    pub fn with_aggregate_budget(
        mut self,
        aggregate_budget: OrchestrationAggregateBudget,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        aggregate_budget.validate_for_plan(&self.plan)?;
        self.aggregate_budget = Some(aggregate_budget);
        Ok(self)
    }

    pub fn aggregate_budget(&self) -> Option<&OrchestrationAggregateBudget> {
        self.aggregate_budget.as_ref()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<OrchestrationUsage>,
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
            usage: None,
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
        if let Some(usage) = &self.usage {
            usage.validate()?;
            for receipt in usage.source_receipts() {
                if !receipts.contains(receipt) {
                    return Err(WorkspaceOrchestrationError::UsageReceiptNotGoverned(
                        receipt.clone(),
                    ));
                }
            }
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

    pub fn with_usage(
        mut self,
        usage: OrchestrationUsage,
    ) -> Result<Self, WorkspaceOrchestrationError> {
        self.usage = Some(usage);
        self.validate()?;
        Ok(self)
    }

    pub fn usage(&self) -> Option<&OrchestrationUsage> {
        self.usage.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Handoff, OrchestrationRole, PlanId, RoleAssignment};

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
            governed.validate_receipt(&run_id, &RoleId::new("planner").unwrap(), &receipt,),
            Err(WorkspaceOrchestrationError::ReceiptRepositoryMismatch(
                RepositoryId::new("api").unwrap()
            ))
        );
    }

    #[test]
    fn governed_plan_requires_one_budget_for_every_role() {
        let governed = governed_plan();
        let budget = OrchestrationAggregateBudget::new(
            OrchestrationBudgetAmount::new(100, 10, 1_000, 100),
            vec![OrchestrationRoleBudget::new(
                RoleId::new("planner").unwrap(),
                OrchestrationBudgetAmount::new(50, 5, 500, 50),
            )],
        )
        .unwrap();

        assert_eq!(
            governed.with_aggregate_budget(budget),
            Err(WorkspaceOrchestrationError::MissingRoleBudget(
                RoleId::new("maker").unwrap()
            ))
        );
    }

    #[test]
    fn summed_role_reservations_must_fit_the_aggregate_ceiling() {
        let governed = governed_plan();
        let budget = OrchestrationAggregateBudget::new(
            OrchestrationBudgetAmount::new(100, 10, 1_000, 100),
            vec![
                OrchestrationRoleBudget::new(
                    RoleId::new("planner").unwrap(),
                    OrchestrationBudgetAmount::new(60, 6, 600, 60),
                ),
                OrchestrationRoleBudget::new(
                    RoleId::new("maker").unwrap(),
                    OrchestrationBudgetAmount::new(60, 6, 600, 60),
                ),
            ],
        )
        .unwrap();

        assert_eq!(
            governed.with_aggregate_budget(budget),
            Err(WorkspaceOrchestrationError::AggregateBudgetOverflow)
        );
    }

    #[test]
    fn deserialized_role_budgets_must_keep_canonical_order() {
        let mut value = serde_json::to_value(
            governed_plan()
                .with_aggregate_budget(
                    OrchestrationAggregateBudget::new(
                        OrchestrationBudgetAmount::new(100, 10, 1_000, 100),
                        vec![
                            OrchestrationRoleBudget::new(
                                RoleId::new("planner").unwrap(),
                                OrchestrationBudgetAmount::new(50, 5, 500, 50),
                            ),
                            OrchestrationRoleBudget::new(
                                RoleId::new("maker").unwrap(),
                                OrchestrationBudgetAmount::new(50, 5, 500, 50),
                            ),
                        ],
                    )
                    .unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
        value["aggregate_budget"]["roles"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let decoded: GovernedOrchestrationPlan = serde_json::from_value(value).unwrap();

        assert_eq!(
            decoded.validate(),
            Err(WorkspaceOrchestrationError::NonCanonicalRoleBudgets)
        );
    }

    #[test]
    fn usage_must_be_bound_to_a_governed_effect_receipt() {
        let receipt = OrchestrationRoleReceipt::new(
            ReceiptId::new("role-receipt-usage").unwrap(),
            OrchestrationRunId::new("run-usage").unwrap(),
            RoleId::new("planner").unwrap(),
            RepositoryId::new("api").unwrap(),
            WorkspaceId::new("workspace-api").unwrap(),
            "commit-api",
            vec![ReceiptId::new("effect-governed").unwrap()],
            None,
        )
        .unwrap();
        let usage = OrchestrationUsage::new(
            10,
            1,
            50,
            Some(5),
            vec![ReceiptId::new("effect-unbound").unwrap()],
        )
        .unwrap();

        assert_eq!(
            receipt.with_usage(usage),
            Err(WorkspaceOrchestrationError::UsageReceiptNotGoverned(
                ReceiptId::new("effect-unbound").unwrap()
            ))
        );
    }

    #[test]
    fn unknown_cost_is_explicit_and_survives_round_trip() {
        let usage = OrchestrationUsage::new(
            10,
            1,
            50,
            None,
            vec![ReceiptId::new("effect-cost-unknown").unwrap()],
        )
        .unwrap();
        let encoded = serde_json::to_vec(&usage).unwrap();
        let decoded: OrchestrationUsage = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded, usage);
        assert_eq!(decoded.cost_micros(), None);
        assert_eq!(
            decoded.source_receipts(),
            &[ReceiptId::new("effect-cost-unknown").unwrap()]
        );
    }
}
