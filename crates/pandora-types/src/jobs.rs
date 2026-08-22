use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_JOB_ARGUMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobCommand {
    Run,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    ApprovalRequired,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::ApprovalRequired => "approval_required",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::ApprovalRequired | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobContractError {
    EmptyArguments,
    ArgumentsTooLarge,
}

impl fmt::Display for JobContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyArguments => formatter.write_str("job arguments cannot be empty"),
            Self::ArgumentsTooLarge => formatter.write_str("job arguments exceed the size limit"),
        }
    }
}

impl std::error::Error for JobContractError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRequest {
    command: JobCommand,
    arguments: Vec<String>,
}

impl JobRequest {
    pub fn new(command: JobCommand, arguments: Vec<String>) -> Result<Self, JobContractError> {
        if arguments.is_empty() {
            return Err(JobContractError::EmptyArguments);
        }
        let argument_bytes = arguments
            .iter()
            .try_fold(0usize, |total, argument| total.checked_add(argument.len()));
        if argument_bytes.is_none_or(|bytes| bytes > MAX_JOB_ARGUMENT_BYTES) {
            return Err(JobContractError::ArgumentsTooLarge);
        }
        Ok(Self { command, arguments })
    }

    pub const fn command(&self) -> JobCommand {
        self.command
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

#[derive(Deserialize)]
struct JobRequestWire {
    command: JobCommand,
    arguments: Vec<String>,
}

impl<'de> Deserialize<'de> for JobRequest {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let wire = JobRequestWire::deserialize(deserializer)?;
        Self::new(wire.command, wire.arguments).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_request_has_a_stable_serialized_contract() {
        let request = JobRequest::new(
            JobCommand::Run,
            vec!["--agent".to_owned(), "fix the failing test".to_owned()],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "command": "run",
                "arguments": ["--agent", "fix the failing test"]
            })
        );
        assert_eq!(
            serde_json::from_value::<JobRequest>(json!({
                "command": "run",
                "arguments": ["--agent", "fix the failing test"]
            }))
            .unwrap(),
            request
        );
    }

    #[test]
    fn run_request_rejects_empty_or_oversized_arguments() {
        assert_eq!(
            JobRequest::new(JobCommand::Run, Vec::new()),
            Err(JobContractError::EmptyArguments)
        );
        assert_eq!(
            JobRequest::new(
                JobCommand::Run,
                vec!["x".repeat(MAX_JOB_ARGUMENT_BYTES + 1)]
            ),
            Err(JobContractError::ArgumentsTooLarge)
        );
    }
}
