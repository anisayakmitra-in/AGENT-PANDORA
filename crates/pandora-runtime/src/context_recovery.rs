#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryStep {
    PruneLowValue,
    RestoreCore,
    RebuildFromL1,
    RetrieveFreshEvidence,
    ReduceScope,
    Pause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryInput {
    pub has_verified_l1: bool,
    pub has_fresh_evidence: bool,
    pub fresh_evidence_is_trusted: bool,
    pub can_reduce_scope: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryDecision {
    steps: Vec<RecoveryStep>,
    paused: bool,
}

impl RecoveryDecision {
    pub fn steps(&self) -> &[RecoveryStep] {
        &self.steps
    }

    pub const fn is_paused(&self) -> bool {
        self.paused
    }
}

pub struct ContextRecovery;

impl ContextRecovery {
    pub const fn new() -> Self {
        Self
    }

    pub fn plan(&self, input: RecoveryInput) -> RecoveryDecision {
        let mut steps = vec![RecoveryStep::PruneLowValue, RecoveryStep::RestoreCore];

        steps.push(RecoveryStep::RebuildFromL1);
        if input.has_verified_l1 {
            return RecoveryDecision {
                steps,
                paused: false,
            };
        }

        steps.push(RecoveryStep::RetrieveFreshEvidence);
        if input.has_fresh_evidence && input.fresh_evidence_is_trusted {
            return RecoveryDecision {
                steps,
                paused: false,
            };
        }

        steps.push(RecoveryStep::ReduceScope);
        if input.can_reduce_scope {
            return RecoveryDecision {
                steps,
                paused: false,
            };
        }

        steps.push(RecoveryStep::Pause);
        RecoveryDecision {
            steps,
            paused: true,
        }
    }
}

impl Default for ContextRecovery {
    fn default() -> Self {
        Self::new()
    }
}
