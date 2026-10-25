#![forbid(unsafe_code)]

use pandora_provider::proxy::build_proxy_from_spec;
use pandora_provider::{ChatMessage, MessageRole, ToolCall};
use pandora_runtime::agent_loop::{DEFAULT_HISTORY_WINDOW_TURNS, compact_session_history};

#[test]
fn test_sliding_window_compaction_on_50_turn_session() {
    let mut history = Vec::new();

    // Construct a 50-turn conversational history:
    // Turn i: User request -> Assistant tool call -> Tool result
    for i in 1..=50 {
        let user_msg = ChatMessage::user(format!(
            "Task step {i}: Inspect subsystem {i} and apply updates."
        ))
        .expect("valid user msg");
        let tool_call = ToolCall::new(
            format!("call_subsystem_{i}"),
            "workspace_inspect",
            serde_json::json!({"path": format!("src/subsystem_{i}.rs")}),
        )
        .expect("valid tool call");
        let assistant_msg =
            ChatMessage::assistant_tool_calls(&[tool_call]).expect("valid assistant msg");
        let tool_result = ChatMessage::tool_result(
            format!("call_subsystem_{i}"),
            format!("Inspected subsystem {i}: all invariants normal"),
        )
        .expect("valid tool result");

        history.push(user_msg);
        history.push(assistant_msg);
        history.push(tool_result);
    }

    assert_eq!(
        history.len(),
        150,
        "Generated 50-turn history has 150 messages"
    );

    // Compact history with window W = 12 turns
    let compacted = compact_session_history(history.clone(), DEFAULT_HISTORY_WINDOW_TURNS)
        .expect("compaction must succeed");

    // Compacted history must be substantially smaller than 120 (preventing context overflow)
    // 12 turns * 3 messages/turn + 2 summary messages = 38 messages
    assert!(
        compacted.len() <= 40,
        "Compacted history length ({}) must be bounded by 3*W + 2",
        compacted.len()
    );

    // Message 0: Historical Context Summary
    assert_eq!(compacted[0].role(), MessageRole::User);
    assert!(
        compacted[0]
            .content()
            .contains("[Historical Context Summary")
    );
    assert!(compacted[0].content().contains("workspace_inspect"));

    // Message 1: Assistant Acknowledgement
    assert_eq!(compacted[1].role(), MessageRole::Assistant);
    assert!(
        compacted[1]
            .content()
            .contains("Acknowledged historical session state")
    );

    // Tail of compacted history must preserve recent turns verbatim
    let tail_len = compacted.len() - 2;
    assert_eq!(
        &compacted[2..],
        &history[history.len() - tail_len..],
        "Recent sliding window must match original tail verbatim"
    );
}

#[test]
fn test_authenticated_corporate_proxy_resolution() {
    // Basic auth proxy with user and password
    let spec = "http://domain%5Cuser:CorpPass2026!@proxy.corp.internal:8080";
    let proxy = build_proxy_from_spec(spec);
    assert!(
        proxy.is_some(),
        "Authenticated corporate proxy spec must build successfully"
    );

    // Bare IP:port proxy spec
    let spec_bare = "10.128.0.254:3128";
    let proxy_bare = build_proxy_from_spec(spec_bare);
    assert!(
        proxy_bare.is_some(),
        "Bare IP:port corporate proxy spec must build successfully"
    );
}
