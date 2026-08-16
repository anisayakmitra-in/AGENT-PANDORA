use pandora_types::{DomainAgentProfile, OrchestrationPlan, PlanId, RoleAssignment, RoleId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationError {
    Plan(pandora_types::OrchestrationContractError),
    DuplicatePlan(PlanId),
    PlanNotFound(PlanId),
    RoleNotFound(RoleId),
    RoleNotActive(RoleId),
    RoleAlreadyCompleted(RoleId),
    HandoffNotDeclared { from: RoleId, to: RoleId },
    HandoffSourceIncomplete,
    HandoffLimit,
    InvalidSnapshot,
}

impl fmt::Display for OrchestrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(formatter),
            Self::DuplicatePlan(id) => write!(formatter, "orchestration plan {id} is duplicated"),
            Self::PlanNotFound(id) => write!(formatter, "orchestration plan {id} was not found"),
            Self::RoleNotFound(id) => write!(formatter, "role {id} was not found"),
            Self::RoleNotActive(id) => write!(formatter, "role {id} is not active"),
            Self::RoleAlreadyCompleted(id) => write!(formatter, "role {id} is already complete"),
            Self::HandoffNotDeclared { from, to } => {
                write!(formatter, "handoff from {from} to {to} is not declared")
            }
            Self::HandoffSourceIncomplete => formatter.write_str("handoff source is not complete"),
            Self::HandoffLimit => formatter.write_str("plan handoff budget is exhausted"),
            Self::InvalidSnapshot => formatter.write_str("orchestration snapshot is invalid"),
        }
    }
}

impl std::error::Error for OrchestrationError {}

impl From<pandora_types::OrchestrationContractError> for OrchestrationError {
    fn from(error: pandora_types::OrchestrationContractError) -> Self {
        Self::Plan(error)
    }
}

pub struct OrchestrationEngine {
    plans: Mutex<BTreeMap<PlanId, OrchestrationPlan>>,
    profiles: Mutex<BTreeMap<PlanId, DomainAgentProfile>>,
}

impl OrchestrationEngine {
    pub fn new() -> Self {
        Self {
            plans: Mutex::new(BTreeMap::new()),
            profiles: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn register(&self, plan: OrchestrationPlan) -> Result<(), OrchestrationError> {
        let mut plans = self
            .plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if plans.contains_key(plan.id()) {
            return Err(OrchestrationError::DuplicatePlan(plan.id().clone()));
        }
        plans.insert(plan.id().clone(), plan);
        Ok(())
    }

    pub fn register_for_meta(
        &self,
        plan: OrchestrationPlan,
        composition: &pandora_types::MetaComposition,
    ) -> Result<(), OrchestrationError> {
        for role in plan.roles() {
            if !composition.allows_domain(role.harness_id()) {
                return Err(OrchestrationError::Plan(
                    pandora_types::OrchestrationContractError::MetaDomainNotAllowed {
                        harness_id: role.harness_id().clone(),
                    },
                ));
            }
        }
        if plan.handoffs().len() as u32 > composition.max_handoffs() {
            return Err(OrchestrationError::Plan(
                pandora_types::OrchestrationContractError::MetaHandoffLimitExceeded {
                    limit: composition.max_handoffs(),
                },
            ));
        }
        self.register(plan)
    }

    pub fn list(&self) -> Vec<OrchestrationPlan> {
        let plans = self
            .plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        plans.values().cloned().collect()
    }

    pub fn register_domain_profile(
        &self,
        profile: DomainAgentProfile,
    ) -> Result<(), OrchestrationError> {
        let plan_id = profile.plan().id().clone();
        self.register(profile.plan().clone())?;
        self.profiles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(plan_id, profile);
        Ok(())
    }

    pub fn domain_profiles(&self) -> Vec<DomainAgentProfile> {
        let profiles = self
            .profiles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        profiles.values().cloned().collect()
    }

    pub fn start(&self, plan_id: &PlanId) -> Result<OrchestrationRun, OrchestrationError> {
        let plan = self
            .plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plan_id)
            .cloned()
            .ok_or_else(|| OrchestrationError::PlanNotFound(plan_id.clone()))?;
        Ok(OrchestrationRun::new(plan))
    }

    pub fn start_domain_profile(
        &self,
        plan_id: &PlanId,
    ) -> Result<DomainProfileRun, OrchestrationError> {
        let profile = self
            .profiles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plan_id)
            .cloned()
            .ok_or_else(|| OrchestrationError::PlanNotFound(plan_id.clone()))?;
        Ok(DomainProfileRun {
            run: OrchestrationRun::new(profile.plan().clone()),
            profile,
        })
    }
}

pub struct OrchestrationRun {
    plan: OrchestrationPlan,
    completed: BTreeSet<RoleId>,
    active: BTreeSet<RoleId>,
    handoffs_used: u32,
}

pub struct DomainProfileRun {
    profile: DomainAgentProfile,
    run: OrchestrationRun,
}

impl DomainProfileRun {
    pub fn profile(&self) -> &DomainAgentProfile {
        &self.profile
    }

    pub fn run(&self) -> &OrchestrationRun {
        &self.run
    }

    pub fn run_mut(&mut self) -> &mut OrchestrationRun {
        &mut self.run
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrchestrationRunSnapshot {
    plan: OrchestrationPlan,
    completed: Vec<RoleId>,
    active: Vec<RoleId>,
    handoffs_used: u32,
}

impl OrchestrationRunSnapshot {
    pub fn plan(&self) -> &OrchestrationPlan {
        &self.plan
    }

    pub fn active_roles(&self) -> &[RoleId] {
        &self.active
    }

    pub fn completed_roles(&self) -> &[RoleId] {
        &self.completed
    }

    pub const fn handoffs_used(&self) -> u32 {
        self.handoffs_used
    }
}

impl OrchestrationRun {
    fn new(plan: OrchestrationPlan) -> Self {
        Self {
            plan,
            completed: BTreeSet::new(),
            active: BTreeSet::new(),
            handoffs_used: 0,
        }
    }

    pub fn plan(&self) -> &OrchestrationPlan {
        &self.plan
    }

    pub fn ready_roles(&self) -> Result<Vec<RoleAssignment>, OrchestrationError> {
        let completed = self.completed.iter().cloned().collect::<Vec<_>>();
        Ok(self
            .plan
            .ready_roles(&completed)
            .into_iter()
            .filter(|role| !self.active.contains(role.id()))
            .take(
                self.plan
                    .max_parallelism()
                    .saturating_sub(self.active.len()),
            )
            .collect())
    }

    pub fn start_ready(&mut self) -> Result<Vec<RoleAssignment>, OrchestrationError> {
        let ready = self.ready_roles()?;
        for role in &ready {
            self.active.insert(role.id().clone());
        }
        Ok(ready)
    }

    pub fn complete(&mut self, role_id: &RoleId) -> Result<(), OrchestrationError> {
        if self.completed.contains(role_id) {
            return Err(OrchestrationError::RoleAlreadyCompleted(role_id.clone()));
        }
        if !self.active.remove(role_id) {
            if self.plan.role(role_id).is_none() {
                return Err(OrchestrationError::RoleNotFound(role_id.clone()));
            }
            return Err(OrchestrationError::RoleNotActive(role_id.clone()));
        }
        self.completed.insert(role_id.clone());
        Ok(())
    }

    pub fn handoff(&mut self, from: &RoleId, to: &RoleId) -> Result<(), OrchestrationError> {
        if self.plan.handoff(from, to).is_none() {
            return Err(OrchestrationError::HandoffNotDeclared {
                from: from.clone(),
                to: to.clone(),
            });
        }
        if !self.completed.contains(from) {
            return Err(OrchestrationError::HandoffSourceIncomplete);
        }
        if self.handoffs_used >= self.plan.max_handoffs() {
            return Err(OrchestrationError::HandoffLimit);
        }
        self.handoffs_used += 1;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.completed.len() == self.plan.roles().len()
    }

    pub fn snapshot(&self) -> OrchestrationRunSnapshot {
        OrchestrationRunSnapshot {
            plan: self.plan.clone(),
            completed: self.completed.iter().cloned().collect(),
            active: self.active.iter().cloned().collect(),
            handoffs_used: self.handoffs_used,
        }
    }

    pub fn from_snapshot(snapshot: OrchestrationRunSnapshot) -> Result<Self, OrchestrationError> {
        let completed = snapshot.completed.iter().cloned().collect::<BTreeSet<_>>();
        let active = snapshot.active.iter().cloned().collect::<BTreeSet<_>>();
        if completed.len() != snapshot.completed.len()
            || active.len() != snapshot.active.len()
            || completed.iter().any(|id| snapshot.plan.role(id).is_none())
            || active.iter().any(|id| snapshot.plan.role(id).is_none())
            || completed.intersection(&active).next().is_some()
            || snapshot.handoffs_used > snapshot.plan.max_handoffs()
        {
            return Err(OrchestrationError::InvalidSnapshot);
        }
        Ok(Self {
            plan: snapshot.plan,
            completed,
            active,
            handoffs_used: snapshot.handoffs_used,
        })
    }
}

impl Default for OrchestrationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        DomainAgentProfile, DomainProfileMode, Handoff, HarnessId, LoopTermination,
        MetaComposition, OrchestrationPlan, OrchestrationRole, PlanId, RoleAssignment, RoleId,
        RunLoopConfig,
    };

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

    fn plan() -> OrchestrationPlan {
        OrchestrationPlan::new(
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
                role(
                    "verifier",
                    OrchestrationRole::Verifier,
                    "coding-domain",
                    &["critic"],
                ),
            ],
            2,
            2,
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn single_agent_plan_completes_without_effects() {
        let engine = OrchestrationEngine::new();
        engine.register(plan()).unwrap();
        let mut run = engine.start(&PlanId::new("coding").unwrap()).unwrap();

        let ready = run.start_ready().unwrap();
        assert_eq!(ready.len(), 1);
        run.complete(ready[0].id()).unwrap();
        assert!(run.snapshot().active_roles().is_empty());
    }

    #[test]
    fn meta_registration_rejects_domains_outside_its_composition() {
        let composition =
            MetaComposition::new(vec![HarnessId::new("research-domain").unwrap()], 2).unwrap();
        let engine = OrchestrationEngine::new();

        assert_eq!(
            engine.register_for_meta(plan(), &composition),
            Err(OrchestrationError::Plan(
                pandora_types::OrchestrationContractError::MetaDomainNotAllowed {
                    harness_id: HarnessId::new("coding-domain").unwrap(),
                }
            ))
        );
        assert!(engine.list().is_empty());
    }

    #[test]
    fn meta_registration_accepts_only_declared_domains() {
        let composition =
            MetaComposition::new(vec![HarnessId::new("coding-domain").unwrap()], 2).unwrap();
        let engine = OrchestrationEngine::new();

        engine.register_for_meta(plan(), &composition).unwrap();

        assert_eq!(engine.list().len(), 1);
    }

    #[test]
    fn dependencies_schedule_planner_maker_critic_verifier() {
        let engine = OrchestrationEngine::new();
        engine.register(plan()).unwrap();
        let mut run = engine.start(&PlanId::new("coding").unwrap()).unwrap();

        for expected in ["planner", "maker", "critic", "verifier"] {
            let ready = run.start_ready().unwrap();
            assert_eq!(ready.len(), 1);
            assert_eq!(ready[0].id().as_str(), expected);
            run.complete(ready[0].id()).unwrap();
        }
        assert!(run.is_complete());
    }

    #[test]
    fn parallelism_is_bounded_and_deterministic() {
        let plan = OrchestrationPlan::new(
            PlanId::new("parallel").unwrap(),
            vec![
                role("a", OrchestrationRole::Planner, "coding-domain", &[]),
                role("b", OrchestrationRole::Maker, "coding-domain", &[]),
                role("c", OrchestrationRole::Critic, "coding-domain", &[]),
            ],
            2,
            1,
            Vec::new(),
        )
        .unwrap();
        let engine = OrchestrationEngine::new();
        engine.register(plan).unwrap();
        let mut run = engine.start(&PlanId::new("parallel").unwrap()).unwrap();

        let ready = run.start_ready().unwrap();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].id().as_str(), "a");
        assert_eq!(ready[1].id().as_str(), "b");
    }

    #[test]
    fn failed_handoff_is_rejected() {
        let handoff = Handoff::new(
            RoleId::new("planner").unwrap(),
            RoleId::new("critic").unwrap(),
            None,
        );
        let plan = OrchestrationPlan::new(
            PlanId::new("handoff").unwrap(),
            vec![
                role("planner", OrchestrationRole::Planner, "coding-domain", &[]),
                role("critic", OrchestrationRole::Critic, "coding-domain", &[]),
            ],
            1,
            1,
            vec![handoff],
        )
        .unwrap();
        let engine = OrchestrationEngine::new();
        engine.register(plan).unwrap();
        let mut run = engine.start(&PlanId::new("handoff").unwrap()).unwrap();

        assert!(matches!(
            run.handoff(
                &RoleId::new("planner").unwrap(),
                &RoleId::new("critic").unwrap()
            ),
            Err(OrchestrationError::HandoffSourceIncomplete)
        ));
    }

    #[test]
    fn snapshots_replay_deterministically() {
        let engine = OrchestrationEngine::new();
        engine.register(plan()).unwrap();
        let mut run = engine.start(&PlanId::new("coding").unwrap()).unwrap();
        let ready = run.start_ready().unwrap();
        run.complete(ready[0].id()).unwrap();
        let snapshot = run.snapshot();
        let restored = OrchestrationRun::from_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.snapshot(), snapshot);
        assert_eq!(restored.ready_roles().unwrap()[0].id().as_str(), "maker");
    }

    #[test]
    fn domain_profile_registration_retains_mode_and_loop_budget() {
        let profile = DomainAgentProfile::new(
            HarnessId::new("coding-domain").unwrap(),
            plan(),
            RunLoopConfig::new(3, 1_000, 4, 60, 2_000, 1, LoopTermination::GoalReached).unwrap(),
            DomainProfileMode::Swarm { workers: 2 },
        )
        .unwrap();
        let engine = OrchestrationEngine::new();

        engine.register_domain_profile(profile.clone()).unwrap();

        assert_eq!(engine.domain_profiles(), vec![profile.clone()]);
        let run = engine.start_domain_profile(profile.plan().id()).unwrap();
        assert_eq!(run.profile(), &profile);
        assert_eq!(run.run().plan(), profile.plan());
    }
}
