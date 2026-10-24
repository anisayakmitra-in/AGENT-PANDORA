use super::{load_config, parse_options, require_config_file, session_error, session_store};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::SocietyEngine;
use pandora_types::AgentPersona;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("peer requires 'list', 'roster', 'register', 'inspect', 'delete', 'send', 'inbox', 'read', 'reply', 'thread', or 'a2a'"))?;

    match subcommand.as_str() {
        "list" | "roster" => roster(&args[1..]),
        "register" => register(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "delete" => delete(&args[1..]),
        "send" => send(&args[1..]),
        "inbox" => inbox(&args[1..]),
        "read" => read(&args[1..]),
        "reply" => reply(&args[1..]),
        "thread" => thread(&args[1..]),
        "a2a" => a2a(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown peer command '{unknown}'"))),
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn roster(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let personas = SocietyEngine::list_personas(&store).map_err(session_error)?;

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<15} {:<15} {:<40} {:<15}",
        "HANDLE", "NAME", "ROLE", "MODEL"
    ));
    lines.push("-".repeat(85));
    for p in &personas {
        let model_str = p.model.as_deref().unwrap_or("<default>");
        lines.push(format!(
            "@{:<14} {:<15} {:<40} {:<15}",
            p.handle, p.display_name, p.role, model_str
        ));
    }

    Ok(success(
        "peer list",
        json!({
            "count": personas.len(),
            "personas": personas,
        }),
        lines.join("\n"),
    ))
}

fn register(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "handle",
            "role",
            "display",
            "model",
            "prompt",
            "avatar",
            "capabilities",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let handle = parsed
        .value("handle")
        .ok_or_else(|| CliError::usage("peer register requires --handle <name>"))?;
    let role = parsed
        .value("role")
        .ok_or_else(|| CliError::usage("peer register requires --role <role>"))?;

    let display = parsed
        .value("display")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let clean = handle.trim_start_matches('@');
            let mut chars = clean.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => clean.to_string(),
            }
        });

    let model = parsed.value("model").map(|s| s.to_string());
    let prompt = parsed
        .value("prompt")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("You are the {display} agent persona specializing in {role}."));
    let avatar = parsed
        .value("avatar")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("persona-seed-{}", handle.trim_start_matches('@')));

    let capabilities = parsed
        .value("capabilities")
        .map(|s| {
            s.split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let now = now_epoch_secs();
    let persona = AgentPersona::new(
        handle,
        display,
        role,
        prompt,
        model,
        avatar,
        capabilities,
        now,
    );

    let saved = SocietyEngine::register_persona(&store, persona).map_err(session_error)?;

    Ok(success(
        "peer register",
        json!(saved),
        format!(
            "Registered persona @{} ({})",
            saved.handle, saved.display_name
        ),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "handle"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let handle = parsed
        .positionals
        .first()
        .map(String::as_str)
        .or_else(|| parsed.value("handle"))
        .ok_or_else(|| CliError::usage("peer inspect requires <handle> or --handle <name>"))?;

    let persona = SocietyEngine::get_persona(&store, handle)
        .map_err(session_error)?
        .ok_or_else(|| CliError::usage(format!("persona '{handle}' not found")))?;

    let display_text = format!(
        "Handle:        @{}\nDisplay Name:  {}\nRole:          {}\nModel:         {}\nAvatar Seed:   {}\nCapabilities:  {}\nCreated:       {}\nSystem Prompt:\n{}",
        persona.handle,
        persona.display_name,
        persona.role,
        persona.model.as_deref().unwrap_or("<default>"),
        persona.avatar_seed,
        persona.capabilities.join(", "),
        persona.created_at,
        persona.system_prompt,
    );

    Ok(success("peer inspect", json!(persona), display_text))
}

fn delete(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "handle"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let handle = parsed
        .positionals
        .first()
        .map(String::as_str)
        .or_else(|| parsed.value("handle"))
        .ok_or_else(|| CliError::usage("peer delete requires <handle> or --handle <name>"))?;

    let deleted = SocietyEngine::delete_persona(&store, handle).map_err(session_error)?;
    if !deleted {
        return Err(CliError::usage(format!("persona '{handle}' was not found")));
    }

    Ok(success(
        "peer delete",
        json!({"handle": handle, "deleted": true}),
        format!("Deleted persona @{handle}"),
    ))
}

fn send(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "from",
            "to",
            "body",
            "message",
            "subject",
            "thread",
            "payload",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let from = parsed
        .value("from")
        .ok_or_else(|| CliError::usage("peer send requires --from <sender_handle>"))?;
    let to = parsed
        .value("to")
        .ok_or_else(|| CliError::usage("peer send requires --to <recipient_handle>"))?;
    let body = parsed
        .value("body")
        .or_else(|| parsed.value("message"))
        .ok_or_else(|| CliError::usage("peer send requires --body <message_text>"))?;
    let subject = parsed.value("subject").unwrap_or("Peer direct message");
    let thread = parsed.value("thread");

    let payload: Option<serde_json::Value> = if let Some(raw) = parsed.value("payload") {
        serde_json::from_str(raw)
            .map(Some)
            .map_err(|e| CliError::usage(format!("invalid JSON payload: {e}")))?
    } else {
        None
    };

    let receipt =
        SocietyEngine::send_peer_message(&store, from, to, subject, body, payload, thread)
            .map_err(session_error)?;

    Ok(success(
        "peer send",
        json!(receipt),
        format!(
            "Dispatched DM {} to @{} (Thread: {})",
            receipt.message_id, receipt.to_handle, receipt.thread_id
        ),
    ))
}

fn inbox(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "handle", "unread"],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let handle = parsed.value("handle");
    let unread_only = parsed.has_flag("unread");

    let messages = SocietyEngine::list_inbox(&store, handle, unread_only).map_err(session_error)?;

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<15} {:<12} {:<12} {:<10} {:<30}",
        "MESSAGE ID", "FROM", "TO", "STATUS", "SUBJECT"
    ));
    lines.push("-".repeat(80));
    for m in &messages {
        lines.push(format!(
            "{:<15} @{:<11} @{:<11} {:<10} {:<30}",
            m.message_id,
            m.from_handle,
            m.to_handle,
            m.status.as_str(),
            m.subject
        ));
    }

    Ok(success(
        "peer inbox",
        json!({
            "count": messages.len(),
            "messages": messages,
        }),
        lines.join("\n"),
    ))
}

fn read(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let message_id = parsed
        .positionals
        .first()
        .map(String::as_str)
        .or_else(|| parsed.value("id"))
        .ok_or_else(|| CliError::usage("peer read requires <message_id>"))?;

    let message = SocietyEngine::read_message(&store, message_id).map_err(session_error)?;

    let display_text = format!(
        "Message ID:  {}\nThread ID:   {}\nFrom:        @{}\nTo:          @{}\nStatus:      {}\nSubject:     {}\nTimestamp:   {}\nBody:\n{}",
        message.message_id,
        message.thread_id,
        message.from_handle,
        message.to_handle,
        message.status.as_str(),
        message.subject,
        message.created_at,
        message.body,
    );

    Ok(success("peer read", json!(message), display_text))
}

fn reply(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "id",
            "from",
            "body",
            "message",
            "payload",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let message_id = parsed
        .positionals
        .first()
        .map(String::as_str)
        .or_else(|| parsed.value("id"))
        .ok_or_else(|| CliError::usage("peer reply requires <message_id>"))?;

    let from = parsed
        .value("from")
        .ok_or_else(|| CliError::usage("peer reply requires --from <sender_handle>"))?;
    let body = parsed
        .value("body")
        .or_else(|| parsed.value("message"))
        .ok_or_else(|| CliError::usage("peer reply requires --body <message_text>"))?;

    let payload: Option<serde_json::Value> = if let Some(raw) = parsed.value("payload") {
        serde_json::from_str(raw)
            .map(Some)
            .map_err(|e| CliError::usage(format!("invalid JSON payload: {e}")))?
    } else {
        None
    };

    let receipt = SocietyEngine::reply_message(&store, message_id, from, body, payload)
        .map_err(session_error)?;

    Ok(success(
        "peer reply",
        json!(receipt),
        format!(
            "Sent reply {} to @{} (Thread: {})",
            receipt.message_id, receipt.to_handle, receipt.thread_id
        ),
    ))
}

fn thread(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let thread_id = parsed
        .positionals
        .first()
        .map(String::as_str)
        .or_else(|| parsed.value("id"))
        .ok_or_else(|| CliError::usage("peer thread requires <thread_id>"))?;

    let th = SocietyEngine::get_thread(&store, thread_id).map_err(session_error)?;

    let mut lines = Vec::new();
    lines.push(format!("Thread: {}", th.thread_id));
    lines.push(format!("Subject: {}", th.subject));
    lines.push(format!("Participants: {}", th.participants.join(", ")));
    lines.push(format!("Messages: {}", th.message_count));
    lines.push("-".repeat(80));

    for msg in &th.messages {
        lines.push(format!(
            "[{}] @{} -> @{}: {}",
            msg.message_id, msg.from_handle, msg.to_handle, msg.subject
        ));
        lines.push(format!("  {}", msg.body));
        lines.push(String::new());
    }

    Ok(success("peer thread", json!(th), lines.join("\n")))
}

fn a2a(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args.first().ok_or_else(|| {
        CliError::usage("peer a2a requires 'card', 'rpc', 'delegate', or 'status'")
    })?;

    match action.as_str() {
        "card" => a2a_card(&args[1..]),
        "rpc" => a2a_rpc(&args[1..]),
        "delegate" => a2a_delegate(&args[1..]),
        "status" => a2a_status(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown peer a2a action '{unknown}'"
        ))),
    }
}

fn a2a_card(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "node-id"])?;
    let node_id = parsed.value("node-id").unwrap_or("pandora-primary");
    let card = pandora_runtime::A2aEngine::local_agent_card(node_id);

    let val = serde_json::to_value(&card)
        .map_err(|_| CliError::internal("could not serialize agent card", json!({})))?;

    let text = format!(
        "Agent ID:       {}\nName:           {}\nVersion:        {}\nDescription:    {}\nAuth Mode:      {}\nEndpoints:      {}\nCapabilities:   {}\nModels:         {}",
        card.id,
        card.name,
        card.version,
        card.description,
        card.auth_mode,
        card.endpoints.join(", "),
        card.capabilities.join(", "),
        card.supported_models.join(", "),
    );

    Ok(success("peer a2a card", val, text))
}

fn a2a_rpc(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "request"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let raw_req = parsed
        .value("request")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| {
            CliError::usage(
                "peer a2a rpc requires --request '<json>' or a JSON positional argument",
            )
        })?;

    let rpc_req: pandora_types::A2aRpcRequest = serde_json::from_str(raw_req)
        .map_err(|e| CliError::usage(format!("invalid A2A JSON-RPC 2.0 request: {e}")))?;

    let rpc_res =
        pandora_runtime::A2aEngine::handle_rpc(&rpc_req, &store).map_err(session_error)?;

    let val = serde_json::to_value(&rpc_res)
        .map_err(|_| CliError::internal("could not serialize RPC response", json!({})))?;

    let text = serde_json::to_string_pretty(&val).unwrap_or_default();
    Ok(success("peer a2a rpc", val, text))
}

fn a2a_delegate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "endpoint",
            "from",
            "to",
            "intent",
            "payload",
            "timeout",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let endpoint = parsed.value("endpoint").unwrap_or("local");
    let from = parsed.value("from").unwrap_or("operator");
    let to = parsed
        .value("to")
        .ok_or_else(|| CliError::usage("peer a2a delegate requires --to <target_agent>"))?;
    let intent = parsed
        .value("intent")
        .ok_or_else(|| CliError::usage("peer a2a delegate requires --intent <task_description>"))?;

    let payload: Option<serde_json::Value> = if let Some(raw) = parsed.value("payload") {
        serde_json::from_str(raw)
            .map(Some)
            .map_err(|e| CliError::usage(format!("invalid JSON payload: {e}")))?
    } else {
        None
    };

    let timeout_seconds = parsed
        .value("timeout")
        .map(|v| v.parse::<u64>())
        .transpose()
        .map_err(|_| CliError::usage("timeout must be an integer (seconds)"))?
        .unwrap_or(300);

    let response = pandora_runtime::A2aEngine::delegate(
        endpoint,
        from,
        to,
        intent,
        payload,
        timeout_seconds,
        &store,
    )
    .map_err(|e| CliError::internal(e, json!({})))?;

    let val = serde_json::to_value(&response)
        .map_err(|_| CliError::internal("could not serialize task response", json!({})))?;

    let text = format!(
        "Delegated Task ID: {}\nStatus:            {}\nDigest:            {}\nOutput:            {}",
        response.task_id,
        response.status.as_str(),
        response.evidence_digest.as_deref().unwrap_or("<none>"),
        response.output.as_deref().unwrap_or("<none>"),
    );

    Ok(success("peer a2a delegate", val, text))
}

fn a2a_status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "task-id"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let task_id = parsed
        .value("task-id")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| {
            CliError::usage("peer a2a status requires --task-id <id> or positional task ID")
        })?;

    let rpc_req = pandora_types::A2aRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: serde_json::json!(1),
        method: "a2a.query".to_string(),
        params: serde_json::json!({"task_id": task_id}),
    };

    let rpc_res =
        pandora_runtime::A2aEngine::handle_rpc(&rpc_req, &store).map_err(session_error)?;

    let val = serde_json::to_value(&rpc_res)
        .map_err(|_| CliError::internal("could not serialize RPC response", json!({})))?;

    Ok(success(
        "peer a2a status",
        val,
        format!("Task '{task_id}' status retrieved successfully"),
    ))
}
