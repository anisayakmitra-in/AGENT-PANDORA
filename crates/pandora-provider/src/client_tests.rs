use super::*;
use crate::manifest::ProviderManifest;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn manifest() -> ProviderManifest {
    ProviderManifest::new(
        "openai-compatible",
        "OpenAI-compatible",
        "https://api.example.test/v1",
        "model-a",
        "PANDORA_PROVIDER_KEY",
    )
    .unwrap()
}

#[test]
fn request_is_bounded_and_binds_provider_and_model() {
    let manifest = manifest();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap()
    .with_max_output_tokens(128)
    .unwrap();

    assert!(request.validate().is_ok());
    assert_eq!(request.provider_id(), manifest.id());
    assert_eq!(request.model_id(), manifest.default_model());
}

#[test]
fn duplicate_tool_names_are_rejected_before_network_use() {
    let manifest = manifest();
    let tool = ToolSchema::new("read_file", "Read a file", json!({"type":"object"})).unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("read").unwrap()],
    )
    .unwrap()
    .with_tools(vec![tool.clone(), tool]);

    assert_eq!(
        request.unwrap_err(),
        ProviderError::InvalidRequest("duplicate tool name".to_owned())
    );
}

#[test]
fn response_parser_rejects_malformed_tool_arguments_without_echoing_them() {
    let error = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"read_file","arguments":"not-json"}}
                ]}}]
            }"#,
        )
        .unwrap_err();

    assert_eq!(
        error,
        ProviderError::InvalidToolArguments {
            call_id: "call-1".to_owned()
        }
    );
    assert!(!error.to_string().contains("not-json"));
}

#[test]
fn tool_call_identifiers_are_bounded_before_transcript_creation() {
    let identifier = "x".repeat(257);

    assert!(ToolCall::new("x".repeat(256), "workspace.read", json!({})).is_ok());
    assert!(ToolCall::new(identifier.clone(), "workspace.read", json!({})).is_err());
    assert!(ChatMessage::tool_result(identifier.clone(), "fixture").is_err());

    let response = json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": identifier,
                    "type": "function",
                    "function": {
                        "name": "workspace.read",
                        "arguments": "{}"
                    }
                }]
            }
        }]
    });

    assert_eq!(
        parse_response(&serde_json::to_vec(&response).unwrap()).unwrap_err(),
        ProviderError::InvalidResponse
    );
}

#[test]
fn tool_call_continuation_serializes_assistant_and_tool_messages() {
    let response = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}
                ]}}]
            }"#,
        )
        .unwrap();
    let call = &response.tool_calls()[0];
    let assistant = ChatMessage::assistant_tool_calls(response.tool_calls()).unwrap();
    let tool = ChatMessage::tool_result(call.id(), "fixture").unwrap();
    let manifest = manifest();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("read README").unwrap(), assistant, tool],
    )
    .unwrap();

    let body = serde_json::to_value(OpenAiRequest::from_request(&request)).unwrap();
    assert_eq!(
        body["messages"][1],
        json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call-1",
                "type": "function",
                "function": {
                    "name": "workspace.read",
                    "arguments": "{\"path\":\"README.md\"}"
                }
            }]
        })
    );
    assert_eq!(
        body["messages"][2],
        json!({"role": "tool", "content": "fixture", "tool_call_id": "call-1"})
    );
}

#[test]
fn persisted_chat_messages_round_trip_tool_calls() {
    let response = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}
                ]}}]
            }"#,
        )
        .unwrap();
    let messages = vec![
        ChatMessage::user("read README").unwrap(),
        ChatMessage::assistant_tool_calls(response.tool_calls()).unwrap(),
        ChatMessage::tool_result("call-1", "fixture").unwrap(),
    ];

    let encoded = serde_json::to_vec(&messages).unwrap();
    let decoded: Vec<ChatMessage> = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(decoded, messages);
}

#[test]
fn anthropic_request_translates_system_tools_and_tool_results() {
    let manifest = ProviderManifest::new_with_protocol(
        "anthropic",
        "Anthropic",
        ProviderProtocol::AnthropicMessages,
        "https://api.anthropic.com/v1",
        "claude-sonnet-4-20250514",
        "PANDORA_ANTHROPIC_API_KEY",
    )
    .unwrap();
    let call = ToolCall::new("toolu-1", "workspace.read", json!({"path": "README.md"})).unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::system("Follow policy.").unwrap(),
            ChatMessage::user("Read README.").unwrap(),
            ChatMessage::assistant_tool_calls(&[call]).unwrap(),
            ChatMessage::tool_result("toolu-1", "fixture").unwrap(),
        ],
    )
    .unwrap()
    .with_tools(vec![
        ToolSchema::new(
            "workspace.read",
            "Read a workspace file",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        )
        .unwrap(),
    ])
    .unwrap();

    let body = serde_json::to_value(AnthropicRequest::from_request(&request).unwrap()).unwrap();

    assert_eq!(body["system"], "Follow policy.");
    assert_eq!(
        body["messages"][0],
        json!({"role":"user","content":"Read README."})
    );
    assert_eq!(
        body["messages"][1],
        json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu-1",
                "name": "workspace.read",
                "input": {"path": "README.md"}
            }]
        })
    );
    assert_eq!(
        body["messages"][2],
        json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu-1",
                "content": "fixture"
            }]
        })
    );
    assert_eq!(body["tools"][0]["name"], "workspace.read");
    assert_eq!(body["tools"][0]["input_schema"]["required"][0], "path");
}

#[test]
fn anthropic_prompt_cache_marks_only_the_stable_system_prefix() {
    let manifest = ProviderManifest::new_with_protocol(
        "anthropic",
        "Anthropic",
        ProviderProtocol::AnthropicMessages,
        "https://api.anthropic.com/v1",
        "claude-sonnet-4-20250514",
        "PANDORA_ANTHROPIC_API_KEY",
    )
    .unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::system("Stable governed context.").unwrap(),
            ChatMessage::user("Dynamic task.").unwrap(),
        ],
    )
    .unwrap()
    .with_prompt_cache(PromptCacheTtl::FiveMinutes);

    let body = serde_json::to_value(AnthropicRequest::from_request(&request).unwrap()).unwrap();

    assert_eq!(
        body["system"],
        json!([{
            "type": "text",
            "text": "Stable governed context.",
            "cache_control": {"type": "ephemeral", "ttl": "5m"}
        }])
    );
    assert_eq!(
        body["messages"][0],
        json!({"role": "user", "content": "Dynamic task."})
    );
    assert!(body["messages"][0].get("cache_control").is_none());
}

#[test]
fn reasoning_effort_serializes_for_openai_and_anthropic() {
    let manifest = ProviderManifest::new_with_protocol(
        "anthropic",
        "Anthropic",
        ProviderProtocol::AnthropicMessages,
        "https://api.anthropic.com/v1",
        "claude-3-7-sonnet-20250219",
        "PANDORA_ANTHROPIC_API_KEY",
    )
    .unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("Solve this puzzle.").unwrap()],
    )
    .unwrap()
    .with_reasoning_effort(ReasoningEffort::Medium);

    let anthropic_body =
        serde_json::to_value(AnthropicRequest::from_request(&request).unwrap()).unwrap();
    assert_eq!(
        anthropic_body["thinking"],
        json!({
            "type": "enabled",
            "budget_tokens": 4096
        })
    );
    assert!(anthropic_body["max_tokens"].as_u64().unwrap() > 4096);

    let openai_body = serde_json::to_value(OpenAiRequest::from_request(&request)).unwrap();
    assert_eq!(openai_body["reasoning_effort"], "medium");
}

#[test]
fn anthropic_response_normalizes_text_tools_and_usage() {
    let response = parse_anthropic_response(
            br#"{
                "content": [
                    {"type":"text","text":"Checking."},
                    {"type":"tool_use","id":"toolu-1","name":"workspace.read","input":{"path":"README.md"}}
                ],
                "usage":{
                    "input_tokens":7,
                    "output_tokens":3,
                    "cache_creation_input_tokens":11,
                    "cache_read_input_tokens":13
                }
            }"#,
        )
        .unwrap();

    assert_eq!(response.text(), "Checking.");
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].id(), "toolu-1");
    assert_eq!(response.tool_calls()[0].name(), "workspace.read");
    assert_eq!(response.usage().prompt_tokens(), 31);
    assert_eq!(response.usage().completion_tokens(), 3);
    assert_eq!(response.usage().cached_prompt_tokens(), 13);
    assert_eq!(response.usage().cache_write_prompt_tokens(), 11);
}

#[test]
fn gemini_request_translates_system_tools_and_tool_results() {
    let manifest = ProviderManifest::new_with_protocol(
        "gemini",
        "Gemini",
        ProviderProtocol::GeminiGenerateContent,
        "https://generativelanguage.googleapis.com/v1beta",
        "gemini-2.5-pro",
        "PANDORA_GEMINI_API_KEY",
    )
    .unwrap();
    let call = ToolCall::new("call-1", "workspace.read", json!({"path": "README.md"})).unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::system("Follow policy.").unwrap(),
            ChatMessage::user("Read README.").unwrap(),
            ChatMessage::assistant_tool_calls(&[call]).unwrap(),
            ChatMessage::tool_result("call-1", "fixture").unwrap(),
        ],
    )
    .unwrap()
    .with_tools(vec![
        ToolSchema::new(
            "workspace.read",
            "Read a workspace file",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        )
        .unwrap(),
    ])
    .unwrap();

    let body = serde_json::to_value(GeminiRequest::from_request(&request).unwrap()).unwrap();

    assert_eq!(
        body["systemInstruction"],
        json!({"parts":[{"text":"Follow policy."}]})
    );
    assert_eq!(
        body["contents"][0],
        json!({
            "role":"user",
            "parts":[{"text":"Read README."}]
        })
    );
    assert_eq!(
        body["contents"][1],
        json!({
            "role":"model",
            "parts":[{"functionCall":{
                "name":"workspace.read",
                "args":{"path":"README.md"}
            }}]
        })
    );
    assert_eq!(
        body["contents"][2],
        json!({
            "role":"user",
            "parts":[{"functionResponse":{
                "name":"workspace.read",
                "response":{"content":"fixture"}
            }}]
        })
    );
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["name"],
        "workspace.read"
    );
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 1024);
}

#[test]
fn gemini_response_normalizes_text_tools_and_usage() {
    let response = parse_gemini_response(
        br#"{
                "candidates":[{"content":{"parts":[
                    {"text":"Checking."},
                    {"functionCall":{"name":"workspace.read","args":{"path":"README.md"}}}
                ]}}],
                "usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}
            }"#,
    )
    .unwrap();

    assert_eq!(response.text(), "Checking.");
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].id(), "gemini-call-1");
    assert_eq!(response.tool_calls()[0].name(), "workspace.read");
    assert_eq!(response.usage().prompt_tokens(), 7);
    assert_eq!(response.usage().completion_tokens(), 3);
}

#[test]
fn empty_tool_results_use_a_bounded_placeholder() {
    let message = ChatMessage::tool_result("call-1", "").unwrap();

    assert_eq!(message.role(), MessageRole::Tool);
    assert_eq!(message.content(), "[empty tool result]");
}

#[test]
fn provider_debug_output_does_not_include_the_api_key() {
    let provider = HttpProvider::new(manifest(), "sk-live-secret").unwrap();

    assert!(!format!("{provider:?}").contains("sk-live-secret"));
}

#[test]
fn loopback_provider_completes_without_leaking_the_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..bytes_read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request);
        let request = request.to_ascii_lowercase();
        assert!(request.contains("authorization: bearer sk-live-secret"));
        assert!(request.contains("\"model\":\"model-a\""));
        let body = br#"{"choices":[{"message":{"content":"ready"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });
    let provider_manifest = ProviderManifest::new(
        "openai-compatible",
        "OpenAI-compatible",
        format!("http://{address}/v1"),
        "model-a",
        "PANDORA_PROVIDER_KEY",
    )
    .unwrap();
    let request = ModelRequest::new(
        provider_manifest.id().clone(),
        provider_manifest.default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();
    let provider = HttpProvider::new(provider_manifest, "sk-live-secret").unwrap();

    let response = provider.complete(request).unwrap();

    server.join().unwrap();
    assert_eq!(response.text(), "ready");
    assert_eq!(response.usage().total_tokens(), 3);
}

#[test]
fn anthropic_provider_uses_native_endpoint_headers_and_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..bytes_read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap();
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let request_line = headers.lines().next().unwrap();
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(request_line.contains("post /v1/messages http/1.1"));
        assert!(headers.contains("x-api-key: anthropic-secret"));
        assert!(headers.contains("anthropic-version: 2023-06-01"));
        assert!(!headers.contains("authorization: bearer"));
        assert!(body.contains("\"model\":\"claude-sonnet-4-20250514\""));
        let response = br#"{"content":[{"type":"text","text":"ready"}],"usage":{"input_tokens":2,"output_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .unwrap();
        stream.write_all(response).unwrap();
    });
    let manifest = ProviderManifest::new_with_protocol(
        "anthropic",
        "Anthropic",
        ProviderProtocol::AnthropicMessages,
        format!("http://{address}/v1"),
        "claude-sonnet-4-20250514",
        "PANDORA_ANTHROPIC_API_KEY",
    )
    .unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();
    let provider = HttpProvider::new(manifest, "anthropic-secret").unwrap();

    let response = provider.complete(request).unwrap();

    server.join().unwrap();
    assert_eq!(response.text(), "ready");
    assert_eq!(response.usage().total_tokens(), 3);
}

#[test]
fn gemini_provider_uses_native_endpoint_headers_and_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..bytes_read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap();
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let request_line = headers.lines().next().unwrap();
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(
            request_line.contains("post /v1beta/models/gemini-2.5-pro:generatecontent http/1.1")
        );
        assert!(headers.contains("x-goog-api-key: gemini-secret"));
        assert!(!headers.contains("authorization: bearer"));
        assert!(body.contains("\"maxOutputTokens\":1024"));
        let response = br#"{
                "candidates":[{"content":{"parts":[{"text":"ready"}]}}],
                "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}
            }"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .unwrap();
        stream.write_all(response).unwrap();
    });
    let manifest = ProviderManifest::new_with_protocol(
        "gemini",
        "Gemini",
        ProviderProtocol::GeminiGenerateContent,
        format!("http://{address}/v1beta"),
        "gemini-2.5-pro",
        "PANDORA_GEMINI_API_KEY",
    )
    .unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();
    let provider = HttpProvider::new(manifest, "gemini-secret").unwrap();

    let response = provider.complete(request).unwrap();

    server.join().unwrap();
    assert_eq!(response.text(), "ready");
    assert_eq!(response.usage().total_tokens(), 3);
}

#[test]
fn gemini_endpoint_strips_models_prefix_and_handles_direct_url() {
    let manifest = ProviderManifest::new_with_protocol(
        "gemini",
        "Gemini",
        ProviderProtocol::GeminiGenerateContent,
        "https://generativelanguage.googleapis.com/v1beta",
        "models/gemini-3.8-flash",
        "PANDORA_GEMINI_API_KEY",
    )
    .unwrap();
    let provider = HttpProvider::new(manifest, "test-key").unwrap();
    let endpoint = provider.endpoint(&ModelId::new("models/gemini-3.8-flash").unwrap());
    assert_eq!(
        endpoint,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
    );

    let direct_manifest = ProviderManifest::new_with_protocol(
        "gemini-direct",
        "Gemini Direct",
        ProviderProtocol::GeminiGenerateContent,
        "https://custom-gateway.local/v1/models/my-model:generateContent",
        "my-model",
        "PANDORA_GEMINI_API_KEY",
    )
    .unwrap();
    let direct_provider = HttpProvider::new(direct_manifest, "test-key").unwrap();
    assert_eq!(
        direct_provider.endpoint(&ModelId::new("my-model").unwrap()),
        "https://custom-gateway.local/v1/models/my-model:generateContent"
    );
}

#[test]
fn gemini_request_uses_authorization_bearer_for_access_tokens() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..bytes_read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(headers.contains("authorization: bearer ya29.mock-google-token"));
        assert!(!headers.contains("x-goog-api-key:"));
        let response = br#"{
                "candidates":[{"content":{"parts":[{"text":"authorized"}]}}],
                "usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}
            }"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .unwrap();
        stream.write_all(response).unwrap();
    });
    let manifest = ProviderManifest::new_with_protocol(
        "gemini-adc",
        "Gemini ADC",
        ProviderProtocol::GeminiGenerateContent,
        format!("http://{address}/v1beta"),
        "gemini-3.8-flash",
        "PANDORA_GOOGLE_ACCESS_TOKEN",
    )
    .unwrap();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![ChatMessage::user("ping").unwrap()],
    )
    .unwrap();
    let provider = HttpProvider::new(manifest, "ya29.mock-google-token").unwrap();
    let response = provider.complete(request).unwrap();
    server.join().unwrap();
    assert_eq!(response.text(), "authorized");
}

#[test]
fn gemini_response_filters_thoughts_and_handles_unknown_parts() {
    let payload = br#"{
            "candidates":[{
                "content":{
                    "parts":[
                        {"text":"Thinking step by step...","thought":true},
                        {"text":"Here is the verified answer."},
                        {"unknownPart":{"type":"inline-annotation","data":[1,2,3]}}
                    ]
                }
            }],
            "usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":8}
        }"#;
    let response = parse_gemini_response(payload).unwrap();
    assert_eq!(response.text(), "Here is the verified answer.");
    assert_eq!(response.usage().prompt_tokens(), 10);
    assert_eq!(response.usage().completion_tokens(), 8);
}

#[test]
fn gemini_response_falls_back_to_thoughts_when_text_empty() {
    let payload = br#"{
            "candidates":[{
                "content":{
                    "parts":[
                        {"text":"Only internal reasoning was emitted before cutoff.","thought":true}
                    ]
                }
            }],
            "usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":6}
        }"#;
    let response = parse_gemini_response(payload).unwrap();
    assert_eq!(
        response.text(),
        "Only internal reasoning was emitted before cutoff."
    );
}
