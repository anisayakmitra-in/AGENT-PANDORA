use pandora_types::{EventId, ObservabilitySample, ObservabilitySnapshot, SpanView, TraceView};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservabilityError {
    DuplicateEvent(EventId),
    OutOfOrder { previous: u64, received: u64 },
}

struct ObservationStore {
    samples: Vec<ObservabilitySample>,
    event_ids: HashSet<EventId>,
    last_sequence: Option<u64>,
}

pub struct ObservabilityEngine {
    store: Mutex<ObservationStore>,
}

impl ObservabilityEngine {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(ObservationStore {
                samples: Vec::new(),
                event_ids: HashSet::new(),
                last_sequence: None,
            }),
        }
    }

    pub fn record(&self, sample: ObservabilitySample) -> Result<(), ObservabilityError> {
        let mut store = self
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if store.event_ids.contains(sample.event().event_id()) {
            return Err(ObservabilityError::DuplicateEvent(
                sample.event().event_id().clone(),
            ));
        }
        if let Some(previous) = store.last_sequence
            && sample.sequence() <= previous
        {
            return Err(ObservabilityError::OutOfOrder {
                previous,
                received: sample.sequence(),
            });
        }
        store.event_ids.insert(sample.event().event_id().clone());
        store.last_sequence = Some(sample.sequence());
        store.samples.push(sample);
        Ok(())
    }

    pub fn snapshot(&self) -> ObservabilitySnapshot {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut traces = BTreeMap::<String, Vec<SpanView>>::new();
        let mut total_tokens = 0u64;
        let mut total_cost_micros = 0u64;
        let mut total_latency_ms = 0u64;
        let mut error_count = 0u64;
        let mut drift_total = 0u64;
        let mut drift_count = 0u64;
        for sample in &store.samples {
            traces
                .entry(sample.trace_id().to_owned())
                .or_default()
                .push(SpanView::from_sample(sample));
            total_tokens = total_tokens.saturating_add(sample.token_count());
            total_cost_micros = total_cost_micros.saturating_add(sample.cost_micros());
            total_latency_ms = total_latency_ms.saturating_add(sample.latency_ms());
            if sample.error_code().is_some() {
                error_count = error_count.saturating_add(1);
            }
            if let Some(score) = sample.drift_score() {
                drift_total = drift_total.saturating_add(u64::from(score));
                drift_count = drift_count.saturating_add(1);
            }
        }
        let total_samples = store.samples.len() as u64;
        let reliability_bps = if total_samples == 0 {
            10_000
        } else {
            total_samples
                .saturating_sub(error_count)
                .saturating_mul(10_000)
                .checked_div(total_samples)
                .unwrap_or(0) as u16
        };
        let traces = traces
            .into_iter()
            .map(|(trace_id, spans)| TraceView::new(trace_id, spans))
            .collect();
        ObservabilitySnapshot::new(
            traces,
            total_tokens,
            total_cost_micros,
            total_latency_ms,
            error_count,
            reliability_bps,
            (drift_count > 0).then_some((drift_total / drift_count) as u8),
        )
    }
}

impl Default for ObservabilityEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        EventContext, EventId, EventPayload, EventType, TenantId, Timestamp, WorkspaceId,
    };

    fn sample(sequence: u64, event_id: &str, trace_id: &str, span_id: &str) -> ObservabilitySample {
        let event = pandora_types::RuntimeEvent::new(
            EventId::new(event_id).unwrap(),
            EventType::EffectCompleted,
            EventContext::new(
                TenantId::new("tenant-1").unwrap(),
                WorkspaceId::new("workspace-1").unwrap(),
            ),
            EventPayload::Empty,
        );
        ObservabilitySample::new(
            trace_id,
            span_id,
            None,
            sequence,
            event,
            Timestamp::from_unix_seconds(sequence),
            10,
            25,
            100,
            None,
            Some(2),
        )
        .unwrap()
    }

    #[test]
    fn trace_correlation_preserves_event_order_and_metrics() {
        let engine = ObservabilityEngine::new();
        engine
            .record(sample(1, "event-1", "trace-1", "span-1"))
            .unwrap();
        engine
            .record(sample(2, "event-2", "trace-1", "span-2"))
            .unwrap();

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.traces().len(), 1);
        assert_eq!(snapshot.traces()[0].spans().len(), 2);
        assert_eq!(snapshot.total_tokens(), 20);
        assert_eq!(snapshot.total_cost_micros(), 50);
        assert_eq!(snapshot.total_latency_ms(), 200);
        assert_eq!(snapshot.drift_score(), Some(2));
    }

    #[test]
    fn duplicate_and_out_of_order_events_fail_closed() {
        let engine = ObservabilityEngine::new();
        engine
            .record(sample(2, "event-1", "trace-1", "span-1"))
            .unwrap();
        assert!(matches!(
            engine.record(sample(1, "event-2", "trace-1", "span-2")),
            Err(ObservabilityError::OutOfOrder { .. })
        ));
        assert_eq!(
            engine.record(sample(3, "event-1", "trace-1", "span-3")),
            Err(ObservabilityError::DuplicateEvent(
                EventId::new("event-1").unwrap()
            ))
        );
    }

    #[test]
    fn errors_reduce_reliability_without_exposing_raw_output() {
        let mut failing = sample(1, "event-1", "trace-1", "span-1");
        failing = failing.with_error_code("tool_failed").unwrap();
        let engine = ObservabilityEngine::new();
        engine.record(failing).unwrap();

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.error_count(), 1);
        assert_eq!(snapshot.reliability_bps(), 0);
        assert!(!format!("{snapshot:?}").contains("raw_prompt"));
    }
}
