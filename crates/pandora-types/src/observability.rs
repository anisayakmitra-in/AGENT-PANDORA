use crate::effect::Timestamp;
use crate::events::{EventType, RuntimeEvent};
use crate::ids::EventId;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservabilityContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
}

impl fmt::Display for ObservabilityContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
        }
    }
}

impl std::error::Error for ObservabilityContractError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservabilitySample {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    sequence: u64,
    event: RuntimeEvent,
    at: Timestamp,
    token_count: u64,
    cost_micros: u64,
    latency_ms: u64,
    error_code: Option<String>,
    drift_score: Option<u8>,
}

impl ObservabilitySample {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trace_id: impl Into<String>,
        span_id: impl Into<String>,
        parent_span_id: Option<String>,
        sequence: u64,
        event: RuntimeEvent,
        at: Timestamp,
        token_count: u64,
        cost_micros: u64,
        latency_ms: u64,
        error_code: Option<String>,
        drift_score: Option<u8>,
    ) -> Result<Self, ObservabilityContractError> {
        Ok(Self {
            trace_id: validate_text("trace ID", trace_id.into())?,
            span_id: validate_text("span ID", span_id.into())?,
            parent_span_id: parent_span_id
                .map(|value| validate_text("parent span ID", value))
                .transpose()?,
            sequence,
            event,
            at,
            token_count,
            cost_micros,
            latency_ms,
            error_code: error_code
                .map(|value| validate_text("error code", value))
                .transpose()?,
            drift_score,
        })
    }

    pub fn with_error_code(
        mut self,
        error_code: impl Into<String>,
    ) -> Result<Self, ObservabilityContractError> {
        self.error_code = Some(validate_text("error code", error_code.into())?);
        Ok(self)
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn span_id(&self) -> &str {
        &self.span_id
    }

    pub fn parent_span_id(&self) -> Option<&str> {
        self.parent_span_id.as_deref()
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn event(&self) -> &RuntimeEvent {
        &self.event
    }

    pub const fn at(&self) -> Timestamp {
        self.at
    }

    pub const fn token_count(&self) -> u64 {
        self.token_count
    }

    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }

    pub const fn latency_ms(&self) -> u64 {
        self.latency_ms
    }

    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub const fn drift_score(&self) -> Option<u8> {
        self.drift_score
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpanView {
    span_id: String,
    parent_span_id: Option<String>,
    event_id: EventId,
    event_type: EventType,
    sequence: u64,
    at: Timestamp,
    token_count: u64,
    cost_micros: u64,
    latency_ms: u64,
    error_code: Option<String>,
    drift_score: Option<u8>,
}

impl SpanView {
    pub fn from_sample(sample: &ObservabilitySample) -> Self {
        Self {
            span_id: sample.span_id.clone(),
            parent_span_id: sample.parent_span_id.clone(),
            event_id: sample.event().event_id().clone(),
            event_type: sample.event().event_type(),
            sequence: sample.sequence,
            at: sample.at,
            token_count: sample.token_count,
            cost_micros: sample.cost_micros,
            latency_ms: sample.latency_ms,
            error_code: sample.error_code.clone(),
            drift_score: sample.drift_score,
        }
    }

    pub fn span_id(&self) -> &str {
        &self.span_id
    }

    pub fn parent_span_id(&self) -> Option<&str> {
        self.parent_span_id.as_deref()
    }

    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    pub const fn event_type(&self) -> EventType {
        self.event_type
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn at(&self) -> Timestamp {
        self.at
    }

    pub const fn token_count(&self) -> u64 {
        self.token_count
    }

    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }

    pub const fn latency_ms(&self) -> u64 {
        self.latency_ms
    }

    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub const fn drift_score(&self) -> Option<u8> {
        self.drift_score
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceView {
    trace_id: String,
    spans: Vec<SpanView>,
}

impl TraceView {
    pub fn new(trace_id: impl Into<String>, spans: Vec<SpanView>) -> Self {
        Self {
            trace_id: trace_id.into(),
            spans,
        }
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn spans(&self) -> &[SpanView] {
        &self.spans
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservabilitySnapshot {
    traces: Vec<TraceView>,
    total_tokens: u64,
    total_cost_micros: u64,
    total_latency_ms: u64,
    error_count: u64,
    reliability_bps: u16,
    drift_score: Option<u8>,
}

impl ObservabilitySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        traces: Vec<TraceView>,
        total_tokens: u64,
        total_cost_micros: u64,
        total_latency_ms: u64,
        error_count: u64,
        reliability_bps: u16,
        drift_score: Option<u8>,
    ) -> Self {
        Self {
            traces,
            total_tokens,
            total_cost_micros,
            total_latency_ms,
            error_count,
            reliability_bps,
            drift_score,
        }
    }

    pub fn traces(&self) -> &[TraceView] {
        &self.traces
    }

    pub const fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    pub const fn total_cost_micros(&self) -> u64 {
        self.total_cost_micros
    }

    pub const fn total_latency_ms(&self) -> u64 {
        self.total_latency_ms
    }

    pub const fn error_count(&self) -> u64 {
        self.error_count
    }

    pub const fn reliability_bps(&self) -> u16 {
        self.reliability_bps
    }

    pub const fn drift_score(&self) -> Option<u8> {
        self.drift_score
    }
}

fn validate_text(field: &'static str, value: String) -> Result<String, ObservabilityContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ObservabilityContractError::EmptyField(field));
    }
    if value.len() > 4096 {
        return Err(ObservabilityContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(ObservabilityContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}
