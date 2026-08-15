use crate::{ExecutionController, RunStatus, RunSummary, RuntimeError, ToolEngine};
use pandora_provider::{
    ChatMessage, ModelRequest, Provider, ProviderError, TokenUsage, ToolCall, ToolSchema,
    TraceMetadata,
};
use pandora_types::{Session, TaskIntent, Timestamp};
use serde_json::Value;
use std::fmt;

const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;
const SYSTEM_PROMPT: &str = "You are Pandora, a bounded workspace agent. Use only the registered workspace read and search tools. Stop when the task has enough evidence. Never invent tool results.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentLoopError {
    InvalidBudget,
    InvalidTask,
    Provider(ProviderError),
    EmptyResponse,
    ToolBudgetExceeded,
    TurnBudgetExceeded,
    ApprovalRequired {
        reason: String,
        summary: AgentRunSummary,
    },
    Execution(RuntimeError),
}

impl fmt::Display for AgentLoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBudget => formatter.write_str("agent loop budget must be non-zero"),
            Self::InvalidTask => formatter.write_str("agent task cannot be empty"),
            Self::Provider(error) => error.fmt(formatter),
            Self::EmptyResponse => formatter.write_str("provider returned an empty final response"),
            Self::ToolBudgetExceeded => formatter.write_str("agent tool-call budget exceeded"),
            Self::TurnBudgetExceeded => formatter.write_str("agent turn budget exceeded"),
            Self::ApprovalRequired { reason, .. } => {
                write!(formatter, "agent action requires approval: {reason}")
            }
            Self::Execution(error) => write!(formatter, "agent tool execution failed: {error:?}"),
        }
    }
}

impl std::error::Error for AgentLoopError {}

impl From<ProviderError> for AgentLoopError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRunSummary {
    final_text: String,
    turns: u32,
    tool_calls: u32,
    usage: TokenUsage,
    runs: Vec<RunSummary>,
}

impl AgentRunSummary {
    pub fn final_text(&self) -> &str {
        &self.final_text
    }

    pub fn turns(&self) -> u32 {
        self.turns
    }

    pub fn tool_calls(&self) -> u32 {
        self.tool_calls
    }

    pub fn usage(&self) -> &TokenUsage {
        &self.usage
    }

    pub fn runs(&self) -> &[RunSummary] {
        &self.runs
    }
}

pub struct AgentLoop {
    max_turns: u32,
    max_tool_calls: u32,
    tools: ToolEngine,
}

impl AgentLoop {
    pub fn new(max_turns: u32, max_tool_calls: u32) -> Result<Self, AgentLoopError> {
        if max_turns == 0 || max_tool_calls == 0 {
            return Err(AgentLoopError::InvalidBudget);
        }
        Ok(Self {
            max_turns,
            max_tool_calls,
            tools: ToolEngine::with_builtins(),
        })
    }

    pub fn run(
        &self,
        provider: &dyn Provider,
        controller: &ExecutionController,
        session: Session,
        task: impl Into<String>,
        now: Timestamp,
    ) -> Result<AgentRunSummary, AgentLoopError> {
        let task = task.into();
        if task.trim().is_empty() {
            return Err(AgentLoopError::InvalidTask);
        }

        let tools = self.tools.list();
        let schemas = tools
            .iter()
            .map(|definition| {
                ToolSchema::new(
                    definition.id().as_str(),
                    definition.name(),
                    definition.input_schema().clone(),
                )
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let mut messages = vec![
            ChatMessage::system(SYSTEM_PROMPT)?,
            ChatMessage::user(task)?,
        ];
        let mut usage = TokenUsage::default();
        let mut runs = Vec::new();
        let mut tool_calls = 0;

        for turn in 1..=self.max_turns {
            let request = ModelRequest::new(
                provider.manifest().id().clone(),
                provider.manifest().default_model().clone(),
                messages.clone(),
            )?
            .with_tools(schemas.clone())?
            .with_max_output_tokens(1_024)?
            .with_trace_metadata(TraceMetadata::new().with_session_id(session.id().clone()));
            let response = provider.complete(request)?;
            usage = add_usage(&usage, response.usage());

            if response.tool_calls().is_empty() {
                if response.text().trim().is_empty() {
                    return Err(AgentLoopError::EmptyResponse);
                }
                return Ok(AgentRunSummary {
                    final_text: response.text().to_owned(),
                    turns: turn,
                    tool_calls,
                    usage,
                    runs,
                });
            }

            let requested = response.tool_calls().len() as u32;
            if tool_calls.saturating_add(requested) > self.max_tool_calls {
                return Err(AgentLoopError::ToolBudgetExceeded);
            }
            messages.push(ChatMessage::assistant_tool_calls(response.tool_calls())?);

            for call in response.tool_calls() {
                tool_calls = tool_calls.saturating_add(1);
                match self.execute_tool(call, controller, &session, now, &mut runs)? {
                    ToolExecution::Output(result) => {
                        messages.push(ChatMessage::tool_result(call.id(), result)?);
                    }
                    ToolExecution::Approval { reason, summary } => {
                        runs.push(summary);
                        return Err(AgentLoopError::ApprovalRequired {
                            reason,
                            summary: AgentRunSummary {
                                final_text: String::new(),
                                turns: turn,
                                tool_calls,
                                usage,
                                runs,
                            },
                        });
                    }
                }
            }
        }

        Err(AgentLoopError::TurnBudgetExceeded)
    }

    fn execute_tool(
        &self,
        call: &ToolCall,
        controller: &ExecutionController,
        session: &Session,
        now: Timestamp,
        runs: &mut Vec<RunSummary>,
    ) -> Result<ToolExecution, AgentLoopError> {
        let known = self
            .tools
            .list()
            .iter()
            .any(|definition| definition.id().as_str() == call.name());
        if !known {
            return Ok(ToolExecution::Output(
                "tool error: unsupported tool".to_owned(),
            ));
        }

        let Some(argument) = call
            .arguments()
            .get(if call.name() == "workspace.read" {
                "path"
            } else {
                "query"
            })
            .and_then(Value::as_str)
        else {
            return Ok(ToolExecution::Output(
                "tool error: required argument is missing or invalid".to_owned(),
            ));
        };
        if argument.trim().is_empty() || argument.chars().any(char::is_control) {
            return Ok(ToolExecution::Output(
                "tool error: required argument is invalid".to_owned(),
            ));
        }

        let action = if call.name() == "workspace.read" {
            "read"
        } else {
            "search"
        };
        let intent = TaskIntent::new(format!("{action}:{argument}"))
            .map_err(|_| AgentLoopError::InvalidTask)?;
        let summary = controller
            .run_at(intent, session.clone(), now)
            .map_err(AgentLoopError::Execution)?;
        let output = match summary.status() {
            RunStatus::Completed => bounded_text(summary.output().unwrap_or_default()),
            RunStatus::Denied { .. } => "tool denied by policy".to_owned(),
            RunStatus::ApprovalRequired { reason } => {
                return Ok(ToolExecution::Approval {
                    reason: reason.clone(),
                    summary,
                });
            }
            RunStatus::Failed { .. } => "tool execution failed".to_owned(),
        };
        runs.push(summary);
        Ok(ToolExecution::Output(output))
    }
}

enum ToolExecution {
    Output(String),
    Approval { reason: String, summary: RunSummary },
}

fn add_usage(total: &TokenUsage, next: &TokenUsage) -> TokenUsage {
    TokenUsage::new(
        total.prompt_tokens().saturating_add(next.prompt_tokens()),
        total
            .completion_tokens()
            .saturating_add(next.completion_tokens()),
    )
}

fn bounded_text(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_TOOL_RESULT_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = String::from_utf8_lossy(&bytes[..MAX_TOOL_RESULT_BYTES]).into_owned();
    text.push_str("\n[tool output truncated]");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionController;
    use crate::executors::WorkspaceRoot;
    use pandora_provider::{
        ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage,
        ToolCall,
    };
    use pandora_types::{PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct SequenceProvider {
        manifest: ProviderManifest,
        responses: Mutex<Vec<ModelResponse>>,
    }

    impl SequenceProvider {
        fn new(responses: Vec<ModelResponse>) -> Self {
            Self {
                manifest: ProviderManifest::new(
                    "openai-compatible",
                    "OpenAI-compatible",
                    "http://127.0.0.1:1/v1",
                    "model-a",
                    "PANDORA_PROVIDER_KEY",
                )
                .unwrap(),
                responses: Mutex::new(responses),
            }
        }
    }

    impl Provider for SequenceProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.responses
                .lock()
                .unwrap()
                .pop()
                .ok_or(ProviderError::InvalidResponse)
        }
    }

    #[test]
    fn bounded_loop_executes_a_read_then_returns_the_final_answer() {
        let fixture = Fixture::new();
        let provider = SequenceProvider::new(vec![
            ModelResponse::new("done", vec![], TokenUsage::new(4, 2)),
            ModelResponse::new(
                "",
                vec![
                    ToolCall::new(
                        "call-1",
                        "workspace.read",
                        serde_json::json!({"path": "README.md"}),
                    )
                    .unwrap(),
                ],
                TokenUsage::new(8, 3),
            ),
        ]);
        let controller = ExecutionController::new(fixture.root.clone());

        let result = AgentLoop::new(4, 4)
            .unwrap()
            .run(
                &provider,
                &controller,
                fixture.session(),
                "Read the README and report it",
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(result.final_text(), "done");
        assert_eq!(result.turns(), 2);
        assert_eq!(result.tool_calls(), 1);
        assert_eq!(result.runs().len(), 1);
        assert_eq!(result.runs()[0].output(), Some(b"fixture\n".as_slice()));
    }

    #[test]
    fn loop_rejects_zero_budgets_and_empty_tasks() {
        assert!(matches!(
            AgentLoop::new(0, 1),
            Err(AgentLoopError::InvalidBudget)
        ));
        assert!(matches!(
            AgentLoop::new(1, 0),
            Err(AgentLoopError::InvalidBudget)
        ));

        let fixture = Fixture::new();
        let provider = SequenceProvider::new(vec![ModelResponse::new(
            "done",
            vec![],
            TokenUsage::default(),
        )]);
        let controller = ExecutionController::new(fixture.root.clone());
        assert_eq!(
            AgentLoop::new(1, 1).unwrap().run(
                &provider,
                &controller,
                fixture.session(),
                "   ",
                Timestamp::from_unix_seconds(10),
            ),
            Err(AgentLoopError::InvalidTask)
        );
    }

    #[test]
    fn unknown_tool_calls_are_returned_to_the_model_without_execution() {
        let fixture = Fixture::new();
        let provider = SequenceProvider::new(vec![
            ModelResponse::new("done", vec![], TokenUsage::default()),
            ModelResponse::new(
                "",
                vec![
                    ToolCall::new(
                        "call-unknown",
                        "workspace.write",
                        serde_json::json!({"path": "README.md", "content": "changed"}),
                    )
                    .unwrap(),
                ],
                TokenUsage::default(),
            ),
        ]);
        let controller = ExecutionController::new(fixture.root.clone());

        let result = AgentLoop::new(2, 1)
            .unwrap()
            .run(
                &provider,
                &controller,
                fixture.session(),
                "Do not write files",
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(result.final_text(), "done");
        assert_eq!(result.tool_calls(), 1);
        assert!(result.runs().is_empty());
        assert_eq!(
            std::fs::read(fixture.path.join("README.md")).unwrap(),
            b"fixture\n"
        );
    }

    #[test]
    fn tool_budget_is_checked_before_execution() {
        let fixture = Fixture::new();
        let provider = SequenceProvider::new(vec![ModelResponse::new(
            "",
            vec![
                ToolCall::new(
                    "call-1",
                    "workspace.read",
                    serde_json::json!({"path": "README.md"}),
                )
                .unwrap(),
                ToolCall::new(
                    "call-2",
                    "workspace.read",
                    serde_json::json!({"path": "README.md"}),
                )
                .unwrap(),
            ],
            TokenUsage::default(),
        )]);
        let controller = ExecutionController::new(fixture.root.clone());

        assert_eq!(
            AgentLoop::new(1, 1).unwrap().run(
                &provider,
                &controller,
                fixture.session(),
                "Read the README",
                Timestamp::from_unix_seconds(10),
            ),
            Err(AgentLoopError::ToolBudgetExceeded)
        );
    }

    struct Fixture {
        path: PathBuf,
        root: WorkspaceRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pandora-agent-loop-test-{}-{}",
                std::process::id(),
                NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("README.md"), b"fixture\n").unwrap();
            let root = WorkspaceRoot::new(&path).unwrap();
            Self { path, root }
        }

        fn session(&self) -> Session {
            Session::new(
                SessionId::new("session-1").unwrap(),
                PrincipalId::new("principal-1").unwrap(),
                TenantId::new("tenant-1").unwrap(),
                WorkspaceId::new("workspace-1").unwrap(),
                Timestamp::from_unix_seconds(1),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);
}
