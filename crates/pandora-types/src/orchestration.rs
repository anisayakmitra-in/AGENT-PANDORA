use crate::ids::{HarnessId, IdError, PlanId, RoleId, RunLoopId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationRole {
    Planner,
    Maker,
    Critic,
    Verifier,
    Custom(String),
}

impl OrchestrationRole {
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Planner => "planner",
            Self::Maker => "maker",
            Self::Critic => "critic",
            Self::Verifier => "verifier",
            Self::Custom(value) => value.as_str(),
        }
    }

    pub fn standard() -> [Self; 4] {
        [Self::Planner, Self::Maker, Self::Critic, Self::Verifier]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationContractError {
    InvalidId(IdError),
    EmptyField(&'static str),
    InvalidLimit(&'static str),
    DuplicateRole(RoleId),
    UnknownDependency(RoleId),
    SelfDependency(RoleId),
    DependencyCycle,
    UnknownHandoffRole(RoleId),
    InvalidHandoff,
    UncoordinatedCrossDomain {
        from: HarnessId,
        to: HarnessId,
    },
    MetaDomainNotAllowed {
        harness_id: HarnessId,
    },
    MetaHandoffLimitExceeded {
        limit: u32,
    },
    DomainRoleOutsideHarness {
        expected: HarnessId,
        actual: HarnessId,
    },
    InvalidSwarmWorkers,
    SwarmExceedsParallelism {
        workers: usize,
        max_parallelism: usize,
    },
    TooManyHandoffs,
}

impl fmt::Display for OrchestrationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::InvalidLimit(field) => write!(formatter, "{field} must be greater than zero"),
            Self::DuplicateRole(id) => write!(formatter, "role {id} is duplicated"),
            Self::UnknownDependency(id) => write!(formatter, "role dependency {id} is unknown"),
            Self::SelfDependency(id) => write!(formatter, "role {id} cannot depend on itself"),
            Self::DependencyCycle => formatter.write_str("role dependencies contain a cycle"),
            Self::UnknownHandoffRole(id) => write!(formatter, "handoff role {id} is unknown"),
            Self::InvalidHandoff => formatter.write_str("handoff source and target must differ"),
            Self::UncoordinatedCrossDomain { from, to } => write!(
                formatter,
                "cross-domain handoff from {from} to {to} requires Meta Harness coordination"
            ),
            Self::MetaDomainNotAllowed { harness_id } => {
                write!(
                    formatter,
                    "Meta Harness does not allow Domain Harness {harness_id}"
                )
            }
            Self::MetaHandoffLimitExceeded { limit } => {
                write!(formatter, "Meta Harness allows at most {limit} handoffs")
            }
            Self::DomainRoleOutsideHarness { expected, actual } => write!(
                formatter,
                "Domain profile for {expected} cannot include role from {actual}"
            ),
            Self::InvalidSwarmWorkers => {
                formatter.write_str("Swarm profiles require at least two workers")
            }
            Self::SwarmExceedsParallelism {
                workers,
                max_parallelism,
            } => write!(
                formatter,
                "Swarm worker count {workers} exceeds plan parallelism {max_parallelism}"
            ),
            Self::TooManyHandoffs => formatter.write_str("plan exceeds its handoff limit"),
        }
    }
}

impl std::error::Error for OrchestrationContractError {}

impl From<IdError> for OrchestrationContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoleAssignment {
    id: RoleId,
    role: OrchestrationRole,
    harness_id: HarnessId,
    depends_on: Vec<RoleId>,
}

impl RoleAssignment {
    pub fn new(
        id: RoleId,
        role: OrchestrationRole,
        harness_id: HarnessId,
        depends_on: Vec<RoleId>,
    ) -> Result<Self, OrchestrationContractError> {
        if id.as_str().trim().is_empty() {
            return Err(OrchestrationContractError::EmptyField("role ID"));
        }
        if harness_id.as_str().trim().is_empty() {
            return Err(OrchestrationContractError::EmptyField("harness ID"));
        }
        let mut seen = BTreeSet::new();
        if depends_on.iter().any(|dependency| !seen.insert(dependency)) {
            return Err(OrchestrationContractError::DuplicateRole(id));
        }
        Ok(Self {
            id,
            role,
            harness_id,
            depends_on,
        })
    }

    pub fn id(&self) -> &RoleId {
        &self.id
    }

    pub const fn role(&self) -> &OrchestrationRole {
        &self.role
    }

    pub fn harness_id(&self) -> &HarnessId {
        &self.harness_id
    }

    pub fn depends_on(&self) -> &[RoleId] {
        &self.depends_on
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    from: RoleId,
    to: RoleId,
    meta_harness: Option<HarnessId>,
}

impl Handoff {
    pub fn new(from: RoleId, to: RoleId, meta_harness: Option<HarnessId>) -> Self {
        Self {
            from,
            to,
            meta_harness,
        }
    }

    pub fn from(&self) -> &RoleId {
        &self.from
    }

    pub fn to(&self) -> &RoleId {
        &self.to
    }

    pub fn meta_harness(&self) -> Option<&HarnessId> {
        self.meta_harness.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationPlan {
    id: PlanId,
    roles: Vec<RoleAssignment>,
    max_parallelism: usize,
    max_handoffs: u32,
    handoffs: Vec<Handoff>,
}

impl OrchestrationPlan {
    pub fn new(
        id: PlanId,
        roles: Vec<RoleAssignment>,
        max_parallelism: usize,
        max_handoffs: u32,
        handoffs: Vec<Handoff>,
    ) -> Result<Self, OrchestrationContractError> {
        if roles.is_empty() {
            return Err(OrchestrationContractError::EmptyField("roles"));
        }
        if max_parallelism == 0 {
            return Err(OrchestrationContractError::InvalidLimit("max parallelism"));
        }
        if handoffs.len() as u32 > max_handoffs {
            return Err(OrchestrationContractError::TooManyHandoffs);
        }

        let mut by_id = BTreeMap::new();
        for role in &roles {
            if by_id.insert(role.id().clone(), role).is_some() {
                return Err(OrchestrationContractError::DuplicateRole(role.id().clone()));
            }
        }
        for role in &roles {
            for dependency in role.depends_on() {
                if !by_id.contains_key(dependency) {
                    return Err(OrchestrationContractError::UnknownDependency(
                        dependency.clone(),
                    ));
                }
                if dependency == role.id() {
                    return Err(OrchestrationContractError::SelfDependency(
                        role.id().clone(),
                    ));
                }
            }
        }
        if has_dependency_cycle(&roles, &by_id) {
            return Err(OrchestrationContractError::DependencyCycle);
        }
        for handoff in &handoffs {
            if handoff.from() == handoff.to() {
                return Err(OrchestrationContractError::InvalidHandoff);
            }
            let from = by_id.get(handoff.from()).ok_or_else(|| {
                OrchestrationContractError::UnknownHandoffRole(handoff.from().clone())
            })?;
            let to = by_id.get(handoff.to()).ok_or_else(|| {
                OrchestrationContractError::UnknownHandoffRole(handoff.to().clone())
            })?;
            if from.harness_id() != to.harness_id() && handoff.meta_harness().is_none() {
                return Err(OrchestrationContractError::UncoordinatedCrossDomain {
                    from: from.harness_id().clone(),
                    to: to.harness_id().clone(),
                });
            }
        }

        Ok(Self {
            id,
            roles,
            max_parallelism,
            max_handoffs,
            handoffs,
        })
    }

    pub fn id(&self) -> &PlanId {
        &self.id
    }

    pub fn roles(&self) -> &[RoleAssignment] {
        &self.roles
    }

    pub const fn max_parallelism(&self) -> usize {
        self.max_parallelism
    }

    pub const fn max_handoffs(&self) -> u32 {
        self.max_handoffs
    }

    pub fn handoffs(&self) -> &[Handoff] {
        &self.handoffs
    }

    pub fn role(&self, id: &RoleId) -> Option<&RoleAssignment> {
        self.roles.iter().find(|role| role.id() == id)
    }

    pub fn ready_roles(&self, completed: &[RoleId]) -> Vec<RoleAssignment> {
        let completed = completed.iter().collect::<BTreeSet<_>>();
        self.roles
            .iter()
            .filter(|role| {
                !completed.contains(&role.id())
                    && role
                        .depends_on()
                        .iter()
                        .all(|dependency| completed.contains(&dependency))
            })
            .take(self.max_parallelism)
            .cloned()
            .collect()
    }

    pub fn handoff(&self, from: &RoleId, to: &RoleId) -> Option<&Handoff> {
        self.handoffs
            .iter()
            .find(|handoff| handoff.from() == from && handoff.to() == to)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainProfileMode {
    Agent,
    Swarm { workers: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DomainAgentProfile {
    harness_id: HarnessId,
    plan: OrchestrationPlan,
    loop_config: RunLoopConfig,
    mode: DomainProfileMode,
}

impl DomainAgentProfile {
    pub fn new(
        harness_id: HarnessId,
        plan: OrchestrationPlan,
        loop_config: RunLoopConfig,
        mode: DomainProfileMode,
    ) -> Result<Self, OrchestrationContractError> {
        if let Some(role) = plan
            .roles()
            .iter()
            .find(|role| role.harness_id() != &harness_id)
        {
            return Err(OrchestrationContractError::DomainRoleOutsideHarness {
                expected: harness_id,
                actual: role.harness_id().clone(),
            });
        }
        if let DomainProfileMode::Swarm { workers } = mode {
            if workers < 2 {
                return Err(OrchestrationContractError::InvalidSwarmWorkers);
            }
            if workers > plan.max_parallelism() {
                return Err(OrchestrationContractError::SwarmExceedsParallelism {
                    workers,
                    max_parallelism: plan.max_parallelism(),
                });
            }
        }
        Ok(Self {
            harness_id,
            plan,
            loop_config,
            mode,
        })
    }

    pub fn harness_id(&self) -> &HarnessId {
        &self.harness_id
    }

    pub fn plan(&self) -> &OrchestrationPlan {
        &self.plan
    }

    pub fn loop_config(&self) -> &RunLoopConfig {
        &self.loop_config
    }

    pub const fn mode(&self) -> DomainProfileMode {
        self.mode
    }
}

fn has_dependency_cycle(
    roles: &[RoleAssignment],
    by_id: &BTreeMap<RoleId, &RoleAssignment>,
) -> bool {
    let mut completed = BTreeSet::new();
    loop {
        let mut changed = false;
        for role in roles {
            if !completed.contains(role.id())
                && role
                    .depends_on()
                    .iter()
                    .all(|dependency| completed.contains(dependency))
            {
                completed.insert(role.id().clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    completed.len() != by_id.len()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopTermination {
    GoalReached,
    NoProgress,
    ExplicitStop,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLoopState {
    Ready,
    Running,
    Paused,
    Completed,
    Cancelled,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum LoopDecision {
    Continue,
    Retry,
    Completed,
    Exhausted,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Usage {
    tokens: u64,
    tools: u32,
    duration_seconds: u64,
    cost_micros: u64,
}

impl Usage {
    pub const fn new(tokens: u64, tools: u32, duration_seconds: u64, cost_micros: u64) -> Self {
        Self {
            tokens,
            tools,
            duration_seconds,
            cost_micros,
        }
    }

    pub const fn tokens(self) -> u64 {
        self.tokens
    }

    pub const fn tools(self) -> u32 {
        self.tools
    }

    pub const fn duration_seconds(self) -> u64 {
        self.duration_seconds
    }

    pub const fn cost_micros(self) -> u64 {
        self.cost_micros
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IterationOutcome {
    usage: Usage,
    progress: bool,
    goal_reached: bool,
    failed: bool,
    retryable: bool,
}

impl IterationOutcome {
    pub const fn new(
        usage: Usage,
        progress: bool,
        goal_reached: bool,
        failed: bool,
        retryable: bool,
    ) -> Self {
        Self {
            usage,
            progress,
            goal_reached,
            failed,
            retryable,
        }
    }

    pub const fn usage(&self) -> Usage {
        self.usage
    }

    pub const fn progress(&self) -> bool {
        self.progress
    }

    pub const fn goal_reached(&self) -> bool {
        self.goal_reached
    }

    pub const fn failed(&self) -> bool {
        self.failed
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunLoopConfig {
    max_iterations: u32,
    max_tokens: u64,
    max_tools: u32,
    max_duration_seconds: u64,
    max_cost_micros: u64,
    max_retries: u32,
    termination: LoopTermination,
}

impl RunLoopConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_iterations: u32,
        max_tokens: u64,
        max_tools: u32,
        max_duration_seconds: u64,
        max_cost_micros: u64,
        max_retries: u32,
        termination: LoopTermination,
    ) -> Result<Self, OrchestrationContractError> {
        if max_iterations == 0 {
            return Err(OrchestrationContractError::InvalidLimit("max iterations"));
        }
        if max_tokens == 0 {
            return Err(OrchestrationContractError::InvalidLimit("max tokens"));
        }
        if max_tools == 0 {
            return Err(OrchestrationContractError::InvalidLimit("max tools"));
        }
        if max_duration_seconds == 0 {
            return Err(OrchestrationContractError::InvalidLimit(
                "max duration seconds",
            ));
        }
        if max_cost_micros == 0 {
            return Err(OrchestrationContractError::InvalidLimit("max cost micros"));
        }
        Ok(Self {
            max_iterations,
            max_tokens,
            max_tools,
            max_duration_seconds,
            max_cost_micros,
            max_retries,
            termination,
        })
    }

    pub const fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    pub const fn max_tokens(&self) -> u64 {
        self.max_tokens
    }

    pub const fn max_tools(&self) -> u32 {
        self.max_tools
    }

    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }

    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }

    pub const fn max_retries(&self) -> u32 {
        self.max_retries
    }

    pub const fn termination(&self) -> LoopTermination {
        self.termination
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunLoopSnapshot {
    id: RunLoopId,
    plan_id: PlanId,
    config: RunLoopConfig,
    state: RunLoopState,
    iterations: u32,
    retries: u32,
    used_tokens: u64,
    used_tools: u32,
    used_duration_seconds: u64,
    used_cost_micros: u64,
}

impl RunLoopSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: RunLoopId,
        plan_id: PlanId,
        config: RunLoopConfig,
        state: RunLoopState,
        iterations: u32,
        retries: u32,
        used_tokens: u64,
        used_tools: u32,
        used_duration_seconds: u64,
        used_cost_micros: u64,
    ) -> Self {
        Self {
            id,
            plan_id,
            config,
            state,
            iterations,
            retries,
            used_tokens,
            used_tools,
            used_duration_seconds,
            used_cost_micros,
        }
    }

    pub fn id(&self) -> &RunLoopId {
        &self.id
    }

    pub fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    pub fn config(&self) -> &RunLoopConfig {
        &self.config
    }

    pub const fn state(&self) -> RunLoopState {
        self.state
    }

    pub const fn iterations(&self) -> u32 {
        self.iterations
    }

    pub const fn retries(&self) -> u32 {
        self.retries
    }

    pub const fn used_tokens(&self) -> u64 {
        self.used_tokens
    }

    pub const fn used_tools(&self) -> u32 {
        self.used_tools
    }

    pub const fn used_duration_seconds(&self) -> u64 {
        self.used_duration_seconds
    }

    pub const fn used_cost_micros(&self) -> u64 {
        self.used_cost_micros
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HarnessId, PlanId, RoleId};

    fn role(
        id: &str,
        role: OrchestrationRole,
        harness: &str,
        dependencies: &[&str],
    ) -> RoleAssignment {
        RoleAssignment::new(
            RoleId::new(id).unwrap(),
            role,
            HarnessId::new(harness).unwrap(),
            dependencies
                .iter()
                .map(|value| RoleId::new(*value).unwrap())
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn plans_ready_roles_in_dependency_order() {
        let plan = OrchestrationPlan::new(
            PlanId::new("coding").unwrap(),
            vec![
                role("planner", OrchestrationRole::Planner, "coding-domain", &[]),
                role(
                    "maker",
                    OrchestrationRole::Maker,
                    "coding-domain",
                    &["planner"],
                ),
                role(
                    "critic",
                    OrchestrationRole::Critic,
                    "coding-domain",
                    &["maker"],
                ),
            ],
            2,
            4,
            Vec::new(),
        )
        .unwrap();

        assert_eq!(plan.ready_roles(&[])[0].id().as_str(), "planner");
        assert_eq!(
            plan.ready_roles(&[RoleId::new("planner").unwrap()])[0]
                .id()
                .as_str(),
            "maker"
        );
    }

    #[test]
    fn cyclic_dependencies_are_rejected() {
        let result = OrchestrationPlan::new(
            PlanId::new("cycle").unwrap(),
            vec![
                role("a", OrchestrationRole::Planner, "coding-domain", &["b"]),
                role("b", OrchestrationRole::Maker, "coding-domain", &["a"]),
            ],
            1,
            1,
            Vec::new(),
        );

        assert_eq!(result, Err(OrchestrationContractError::DependencyCycle));
    }

    #[test]
    fn cross_domain_handoffs_require_meta_coordination() {
        let handoff = Handoff::new(
            RoleId::new("planner").unwrap(),
            RoleId::new("designer").unwrap(),
            Some(HarnessId::new("planning-meta").unwrap()),
        );
        let plan = OrchestrationPlan::new(
            PlanId::new("cross-domain").unwrap(),
            vec![
                role("planner", OrchestrationRole::Planner, "coding-domain", &[]),
                role("designer", OrchestrationRole::Maker, "design-domain", &[]),
            ],
            2,
            1,
            vec![handoff],
        );

        assert!(plan.is_ok());
    }

    #[test]
    fn cross_domain_handoffs_without_meta_are_rejected() {
        let result = OrchestrationPlan::new(
            PlanId::new("cross-domain").unwrap(),
            vec![
                role("planner", OrchestrationRole::Planner, "coding-domain", &[]),
                role("designer", OrchestrationRole::Maker, "design-domain", &[]),
            ],
            2,
            1,
            vec![Handoff::new(
                RoleId::new("planner").unwrap(),
                RoleId::new("designer").unwrap(),
                None,
            )],
        );

        assert!(matches!(
            result,
            Err(OrchestrationContractError::UncoordinatedCrossDomain { .. })
        ));
    }

    #[test]
    fn domain_agent_profile_keeps_one_harness_and_loop_budget() {
        let harness_id = HarnessId::new("coding-domain").unwrap();
        let plan = OrchestrationPlan::new(
            PlanId::new("coding-agent").unwrap(),
            vec![role(
                "planner",
                OrchestrationRole::Planner,
                "coding-domain",
                &[],
            )],
            1,
            0,
            Vec::new(),
        )
        .unwrap();
        let loop_config =
            RunLoopConfig::new(3, 1_000, 4, 60, 2_000, 1, LoopTermination::GoalReached).unwrap();

        let profile = DomainAgentProfile::new(
            harness_id.clone(),
            plan.clone(),
            loop_config.clone(),
            DomainProfileMode::Agent,
        )
        .unwrap();

        assert_eq!(profile.harness_id(), &harness_id);
        assert_eq!(profile.plan(), &plan);
        assert_eq!(profile.loop_config(), &loop_config);
        assert_eq!(profile.mode(), DomainProfileMode::Agent);
    }

    #[test]
    fn domain_agent_profile_rejects_roles_from_another_harness() {
        let plan = OrchestrationPlan::new(
            PlanId::new("mixed-domain").unwrap(),
            vec![
                role("planner", OrchestrationRole::Planner, "coding-domain", &[]),
                role("designer", OrchestrationRole::Maker, "design-domain", &[]),
            ],
            2,
            0,
            Vec::new(),
        )
        .unwrap();
        let loop_config =
            RunLoopConfig::new(1, 100, 1, 30, 100, 0, LoopTermination::ExplicitStop).unwrap();

        let result = DomainAgentProfile::new(
            HarnessId::new("coding-domain").unwrap(),
            plan,
            loop_config,
            DomainProfileMode::Agent,
        );

        assert!(matches!(
            result,
            Err(OrchestrationContractError::DomainRoleOutsideHarness { .. })
        ));
    }

    #[test]
    fn swarm_profile_requires_multiple_workers_within_plan_capacity() {
        let harness_id = HarnessId::new("coding-domain").unwrap();
        let plan = OrchestrationPlan::new(
            PlanId::new("coding-swarm").unwrap(),
            vec![
                role("maker-a", OrchestrationRole::Maker, "coding-domain", &[]),
                role("maker-b", OrchestrationRole::Maker, "coding-domain", &[]),
            ],
            2,
            0,
            Vec::new(),
        )
        .unwrap();
        let loop_config =
            RunLoopConfig::new(2, 500, 4, 30, 500, 0, LoopTermination::NoProgress).unwrap();

        let profile = DomainAgentProfile::new(
            harness_id.clone(),
            plan.clone(),
            loop_config.clone(),
            DomainProfileMode::Swarm { workers: 2 },
        )
        .unwrap();
        assert_eq!(profile.mode(), DomainProfileMode::Swarm { workers: 2 });

        assert_eq!(
            DomainAgentProfile::new(
                harness_id.clone(),
                plan.clone(),
                loop_config.clone(),
                DomainProfileMode::Swarm { workers: 1 },
            ),
            Err(OrchestrationContractError::InvalidSwarmWorkers)
        );
        assert_eq!(
            DomainAgentProfile::new(
                harness_id,
                plan,
                loop_config,
                DomainProfileMode::Swarm { workers: 3 },
            ),
            Err(OrchestrationContractError::SwarmExceedsParallelism {
                workers: 3,
                max_parallelism: 2,
            })
        );
    }
}
