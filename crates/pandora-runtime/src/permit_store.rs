use pandora_types::{EffectPermit, OperationRequest, PermitId, Timestamp};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct PermitStore {
    permits: Mutex<HashMap<PermitId, StoredPermit>>,
}

struct StoredPermit {
    permit: EffectPermit,
    state: PermitState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PermitState {
    Active,
    Consumed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermitError {
    AlreadyRegistered,
    UnknownPermit,
    InvalidPermit,
    NotYetValid,
    Expired,
    RequestMismatch,
    AlreadyConsumed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumedPermit {
    permit: EffectPermit,
    consumed_at: Timestamp,
}

impl ConsumedPermit {
    pub fn permit(&self) -> &EffectPermit {
        &self.permit
    }

    pub fn consumed_at(&self) -> Timestamp {
        self.consumed_at
    }
}

impl PermitStore {
    pub fn new() -> Self {
        Self {
            permits: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register(&self, permit: EffectPermit) -> Result<(), PermitError> {
        let mut permits = self
            .permits
            .lock()
            .expect("permit store lock is not poisoned");
        if permits.contains_key(permit.permit_id()) {
            return Err(PermitError::AlreadyRegistered);
        }
        permits.insert(
            permit.permit_id().clone(),
            StoredPermit {
                permit,
                state: PermitState::Active,
            },
        );
        Ok(())
    }

    pub fn consume(
        &self,
        permit: EffectPermit,
        request: &OperationRequest,
        now: Timestamp,
    ) -> Result<ConsumedPermit, PermitError> {
        let mut permits = self
            .permits
            .lock()
            .expect("permit store lock is not poisoned");
        let stored = permits
            .get_mut(permit.permit_id())
            .ok_or(PermitError::UnknownPermit)?;
        if stored.permit != permit {
            return Err(PermitError::InvalidPermit);
        }
        if stored.state == PermitState::Consumed {
            return Err(PermitError::AlreadyConsumed);
        }
        if now < stored.permit.issued_at() {
            return Err(PermitError::NotYetValid);
        }
        if now >= stored.permit.expires_at() {
            return Err(PermitError::Expired);
        }
        if request.request_digest() != stored.permit.request_digest() {
            return Err(PermitError::RequestMismatch);
        }

        stored.state = PermitState::Consumed;
        Ok(ConsumedPermit {
            permit: stored.permit.clone(),
            consumed_at: now,
        })
    }
}

impl Default for PermitStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use pandora_types::{
        Capability, EffectTarget, ExecutionId, GeneId, Operation, PolicyContext, PrincipalId,
        ResourceScope, SessionId, Timestamp,
    };

    use super::*;
    use crate::parliament::Parliament;
    use crate::reference_monitor::ReferenceMonitor;

    fn make_request(session: &str, path: &str) -> pandora_types::OperationRequest {
        pandora_types::OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new(session).unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            GeneId::new("workspace.read").unwrap(),
            None,
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path(path),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    fn permit_for(
        monitor: &ReferenceMonitor,
        request: &pandora_types::OperationRequest,
        now: u64,
    ) -> pandora_types::EffectPermit {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let decision = parliament.decide(request, &context);
        monitor
            .authorize(request.clone(), decision, Timestamp::from_unix_seconds(now))
            .unwrap()
    }

    #[test]
    fn expired_permits_are_rejected() {
        let monitor = ReferenceMonitor::new(1, 1);
        let request = make_request("session-1", "src/lib.rs");
        let permit = permit_for(&monitor, &request, 10);

        assert_eq!(
            monitor
                .store()
                .consume(permit, &request, Timestamp::from_unix_seconds(11)),
            Err(PermitError::Expired)
        );
    }

    #[test]
    fn permit_is_bound_to_the_exact_request() {
        let monitor = ReferenceMonitor::new(1, 60);
        let original_request = make_request("session-1", "src/lib.rs");
        let permit = permit_for(&monitor, &original_request, 10);

        assert_eq!(
            monitor.store().consume(
                permit,
                &make_request("session-2", "src/lib.rs"),
                Timestamp::from_unix_seconds(11),
            ),
            Err(PermitError::RequestMismatch)
        );
    }

    #[test]
    fn permit_cannot_be_replayed() {
        let monitor = ReferenceMonitor::new(1, 60);
        let request = make_request("session-1", "src/lib.rs");
        let permit = permit_for(&monitor, &request, 10);

        monitor
            .store()
            .consume(permit.clone(), &request, Timestamp::from_unix_seconds(11))
            .unwrap();
        assert_eq!(
            monitor
                .store()
                .consume(permit, &request, Timestamp::from_unix_seconds(12)),
            Err(PermitError::AlreadyConsumed)
        );
    }

    #[test]
    fn concurrent_consumption_allows_only_one_consumer() {
        let monitor = Arc::new(ReferenceMonitor::new(1, 60));
        let request = Arc::new(make_request("session-1", "src/lib.rs"));
        let permit = Arc::new(permit_for(&monitor, &request, 10));
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let monitor = Arc::clone(&monitor);
            let request = Arc::clone(&request);
            let permit = Arc::clone(&permit);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                monitor.store().consume(
                    (*permit).clone(),
                    &request,
                    Timestamp::from_unix_seconds(11),
                )
            }));
        }

        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| result == &&Err(PermitError::AlreadyConsumed))
                .count(),
            1
        );
    }
}
