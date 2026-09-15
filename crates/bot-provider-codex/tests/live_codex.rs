use std::{env, fs};

use bot_provider_codex::{
    CodexClient, CodexEvent, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalResponse, DynamicToolCallOutputContentItem,
    DynamicToolCallParams, DynamicToolCallResponse, DynamicToolSpec, ITEM_TOOL_CALL_METHOD,
    InputModality, McpServerElicitationAction, McpServerElicitationRequest,
    McpServerElicitationRequestParams, McpServerElicitationRequestResponse,
    McpServerStatusListParams, ModelListParams, ReasoningSummary, ThreadCompactStartParams,
    ThreadDeleteParams, ThreadForkParams, ThreadListParams, ThreadRevertParams, ThreadSearchParams,
    ThreadSetNameParams, ThreadStartParams, ThreadTurnsListParams, TurnStartParams, UserInput,
};
use serde_json::json;
use tokio::sync::broadcast::Receiver;
use tokio::time::{Duration, timeout};

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn reads_the_live_account_and_model_catalog() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    assert!(!client.server_info().user_agent.is_empty());
    let account = client.account().await.expect("account");
    assert!(account.account.is_some() || account.requires_openai_auth);
    if account.account.is_some() || !account.requires_openai_auth {
        let limits = client.account_rate_limits().await.expect("rate limits");
        assert!(
            limits.rate_limits.primary.is_some()
                || limits.rate_limits.secondary.is_some()
                || limits
                    .rate_limits_by_limit_id
                    .as_ref()
                    .is_some_and(|items| !items.is_empty())
        );
    }
    let models = client
        .models(&ModelListParams::default())
        .await
        .expect("models");
    assert!(!models.data.is_empty());
    assert!(
        models
            .data
            .iter()
            .all(|model| !model.id.is_empty() && !model.model.is_empty())
    );
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI with an MCP server"]
async fn reloads_and_reads_the_live_mcp_catalog() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    client.reload_mcp_servers().await.expect("MCP reload");
    let servers = client
        .mcp_server_statuses(&McpServerStatusListParams::default())
        .await
        .expect("MCP server status");
    assert!(!servers.data.is_empty());
    assert!(servers.data.iter().all(|server| !server.name.is_empty()));
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn sends_and_receives_a_live_text_turn() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread.thread.id,
            input: vec![UserInput::Text {
                text: "Reply with exactly BOT_LIVE_OK. Do not use tools.".to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-message-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_id = turn.turn.id;
    let output = timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &turn_id),
    )
    .await
    .expect("live turn timeout");
    client.shutdown().await.expect("shutdown");
    assert_eq!(output.1, "completed");
    assert_eq!(output.0.trim(), "BOT_LIVE_OK");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn executes_a_live_dynamic_tool_call() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ephemeral: Some(true),
            dynamic_tools: Some(vec![DynamicToolSpec::Function {
                name: "lookup".to_owned(),
                description: "Return the test record for the supplied key.".to_owned(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"],
                    "additionalProperties": false
                }),
                defer_loading: Some(false),
            }]),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let thread_id = thread.thread.id;
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Call lookup with key alpha exactly once. After it returns, reply with exactly BOT_DYNAMIC_TOOL_OK."
                    .to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-dynamic-tool-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_id = turn.turn.id;
    let result = timeout(Duration::from_secs(60), async {
        let mut called = false;
        let mut completed = false;
        let mut output = String::new();
        loop {
            match events.recv().await.expect("Codex event") {
                CodexEvent::Request(request)
                    if request.method == ITEM_TOOL_CALL_METHOD
                        && request.params.get("turnId").and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    let params: DynamicToolCallParams =
                        serde_json::from_value(request.params).expect("dynamic tool call");
                    assert_eq!(params.thread_id, thread_id);
                    assert!(!params.call_id.is_empty());
                    assert_eq!(params.tool, "lookup");
                    assert_eq!(params.namespace, None);
                    assert_eq!(params.arguments, json!({"key": "alpha"}));
                    client
                        .respond(
                            request.id,
                            &DynamicToolCallResponse {
                                success: true,
                                content_items: vec![DynamicToolCallOutputContentItem::Text {
                                    text: "record alpha".to_owned(),
                                }],
                            },
                        )
                        .await
                        .expect("dynamic tool response");
                    called = true;
                }
                CodexEvent::Notification(notification)
                    if notification.method == "item/completed"
                        && notification.params["turnId"] == turn_id
                        && notification.params["item"]["type"] == "dynamicToolCall" =>
                {
                    assert_eq!(notification.params["item"]["success"], true);
                    assert_eq!(
                        notification.params["item"]["contentItems"][0]["text"],
                        "record alpha"
                    );
                    completed = true;
                }
                CodexEvent::Notification(notification)
                    if notification.method == "item/agentMessage/delta"
                        && notification.params.get("turnId").and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    if let Some(delta) = notification.params["delta"].as_str() {
                        output.push_str(delta);
                    }
                }
                CodexEvent::Notification(notification)
                    if notification.method == "turn/completed"
                        && notification.params["turn"]["id"] == turn_id =>
                {
                    break (
                        called,
                        completed,
                        output,
                        notification.params["turn"]["status"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                CodexEvent::ConnectionClosed(message) => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("live dynamic tool timeout");
    client.shutdown().await.expect("shutdown");
    assert!(result.0);
    assert!(result.1);
    assert_eq!(result.2.trim(), "BOT_DYNAMIC_TOOL_OK");
    assert_eq!(result.3, "completed");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn compacts_a_live_thread() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let thread_id = thread.thread.id;
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Reply with exactly BOT_COMPACT_READY. Do not use tools.".to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-compact-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &turn.turn.id),
    )
    .await
    .expect("live turn timeout");
    timeout(
        Duration::from_secs(60),
        client.compact_thread(&ThreadCompactStartParams { thread_id }),
    )
    .await
    .expect("live compaction timeout")
    .expect("live compaction");
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn rewinds_a_live_thread() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let thread_id = thread.thread.id;
    let mut events = client.subscribe();
    let first = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Reply with exactly BOT_REWIND_FIRST. Do not use tools.".to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd.clone()),
            client_user_message_id: Some("bot-live-rewind-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("first turn");
    timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &first.turn.id),
    )
    .await
    .expect("first live turn timeout");
    let second = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Reply with exactly BOT_REWIND_SECOND. Do not use tools.".to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-rewind-2".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("second turn");
    timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &second.turn.id),
    )
    .await
    .expect("second live turn timeout");
    let turns = client
        .list_thread_turns(&ThreadTurnsListParams {
            thread_id: thread_id.clone(),
            cursor: None,
            limit: Some(100),
            sort_direction: Some("asc".to_owned()),
            items_view: Some("full".to_owned()),
        })
        .await
        .expect("thread turns");
    let listed_two_turns = turns.data.len() == 2;
    let listed_second_turn = turns
        .data
        .get(1)
        .is_some_and(|turn| turn.id == second.turn.id);
    client
        .revert_thread(&ThreadRevertParams {
            thread_id: thread_id.clone(),
            before_turn_id: second.turn.id,
        })
        .await
        .expect("thread rewind");
    let remaining = client
        .list_thread_turns(&ThreadTurnsListParams {
            thread_id: thread_id.clone(),
            cursor: None,
            limit: Some(100),
            sort_direction: Some("asc".to_owned()),
            items_view: Some("full".to_owned()),
        })
        .await
        .expect("remaining thread turns");
    client
        .delete_thread(&ThreadDeleteParams { thread_id })
        .await
        .expect("test thread delete");
    client.shutdown().await.expect("shutdown");
    assert!(listed_two_turns);
    assert!(listed_second_turn);
    assert_eq!(remaining.data.len(), 1);
    assert_eq!(remaining.data[0].id, first.turn.id);
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn renames_forks_and_deletes_a_live_thread() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let test_name = "Bot live session management";
    delete_threads_named(&client, test_name).await;
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let thread_id = thread.thread.id;
    client
        .set_thread_name(&ThreadSetNameParams {
            thread_id: thread_id.clone(),
            name: test_name.to_owned(),
        })
        .await
        .expect("thread name");
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Reply with exactly BOT_SESSION_SEARCH_READY. Remember BOT_LIVE_SEARCH_7A39. Do not use tools."
                    .to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-session-search-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_result = timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &turn.turn.id),
    )
    .await
    .expect("live turn timeout");
    let search = client
        .search_threads(&ThreadSearchParams {
            cursor: None,
            limit: Some(20),
            sort_key: Some("updated_at".to_owned()),
            sort_direction: Some("desc".to_owned()),
            source_kinds: None,
            archived: Some(false),
            search_term: "BOT_LIVE_SEARCH_7A39".to_owned(),
        })
        .await
        .expect("thread search");
    let search_found = search
        .data
        .iter()
        .any(|result| result.thread.id == thread_id);
    let forked = client
        .fork_thread(&ThreadForkParams {
            thread_id: thread_id.clone(),
            cwd: None,
        })
        .await
        .expect("thread fork");
    assert_ne!(forked.thread.id, thread_id);
    client
        .delete_thread(&ThreadDeleteParams {
            thread_id: forked.thread.id,
        })
        .await
        .expect("fork delete");
    client
        .delete_thread(&ThreadDeleteParams { thread_id })
        .await
        .expect("thread delete");
    client.shutdown().await.expect("shutdown");
    assert_eq!(turn_result.1, "completed");
    assert_eq!(turn_result.0.trim(), "BOT_SESSION_SEARCH_READY");
    assert!(search_found);
}

async fn delete_threads_named(client: &CodexClient, name: &str) {
    let threads = client
        .list_threads(&ThreadListParams {
            limit: Some(100),
            archived: Some(false),
            ..ThreadListParams::default()
        })
        .await
        .expect("test thread cleanup list");
    for thread in threads
        .data
        .into_iter()
        .filter(|thread| thread.name.as_deref() == Some(name))
    {
        client
            .delete_thread(&ThreadDeleteParams {
                thread_id: thread.id,
            })
            .await
            .expect("test thread cleanup delete");
    }
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn sends_and_receives_a_live_image_turn() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let models = client
        .models(&ModelListParams::default())
        .await
        .expect("models");
    let model = models
        .data
        .into_iter()
        .find(|model| model.input_modalities.contains(&InputModality::Image))
        .expect("image model")
        .model;
    let thread = client
        .start_thread(&ThreadStartParams {
            model: Some(model.clone()),
            cwd: Some(cwd.clone()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let directory = tempfile::tempdir().expect("temporary directory");
    let image_path = directory.path().join("image.png");
    fs::write(&image_path, TINY_PNG_BYTES).expect("image fixture");
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread.thread.id,
            input: vec![
                UserInput::LocalImage {
                    path: image_path,
                    detail: None,
                },
                UserInput::Text {
                    text: "Reply with exactly BOT_IMAGE_OK. Do not use tools.".to_owned(),
                    text_elements: Vec::new(),
                },
            ],
            model: Some(model),
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-image-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_id = turn.turn.id;
    let output = timeout(
        Duration::from_secs(45),
        collect_text_turn(&mut events, &turn_id),
    )
    .await
    .expect("live turn timeout");
    client.shutdown().await.expect("shutdown");
    assert_eq!(output.1, "completed");
    assert_eq!(output.0.trim(), "BOT_IMAGE_OK");
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn resolves_a_live_command_approval() {
    let client = CodexClient::start("codex").await.expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            approval_policy: Some("untrusted".to_owned()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread.thread.id,
            input: vec![UserInput::Text {
                text: "Run `/bin/sh -lc 'printf BOT_APPROVAL_OK'` exactly once. After it finishes, reply with exactly APPROVAL_DONE."
                    .to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-approval-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_id = turn.turn.id;
    let result = timeout(Duration::from_secs(45), async {
        let mut approved = false;
        let mut output = String::new();
        loop {
            match events.recv().await.expect("Codex event") {
                CodexEvent::Request(request)
                    if request.method == "item/commandExecution/requestApproval"
                        && request.params.get("turnId").and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    let command = request
                        .params
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    assert!(command.contains("printf BOT_APPROVAL_OK"));
                    client
                        .respond(
                            request.id,
                            &CommandExecutionRequestApprovalResponse {
                                decision: CommandExecutionApprovalDecision::Accept,
                            },
                        )
                        .await
                        .expect("approval response");
                    approved = true;
                }
                CodexEvent::Notification(notification)
                    if notification.method == "item/agentMessage/delta"
                        && notification.params.get("turnId").and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    if let Some(delta) = notification
                        .params
                        .get("delta")
                        .and_then(|value| value.as_str())
                    {
                        output.push_str(delta);
                    }
                }
                CodexEvent::Notification(notification)
                    if notification.method == "turn/completed"
                        && notification
                            .params
                            .get("turn")
                            .and_then(|turn| turn.get("id"))
                            .and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    break (approved, output);
                }
                CodexEvent::ConnectionClosed(message) => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("live approval timeout");
    client.shutdown().await.expect("shutdown");
    assert!(result.0);
    assert!(result.1.trim_end().ends_with("APPROVAL_DONE"));
}

#[tokio::test]
#[ignore = "requires an installed and authenticated Codex CLI"]
async fn resolves_a_live_mcp_elicitation() {
    let fixture = format!(
        "{}/tests/fixtures/mcp_elicitation_server.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let fixture = serde_json::to_string(&fixture).expect("fixture path");
    let overrides = [
        "mcp_servers.bot_elicitation_fixture.command=\"python3\"".to_owned(),
        format!("mcp_servers.bot_elicitation_fixture.args=[{fixture}]"),
        "mcp_servers.bot_elicitation_fixture.enabled=true".to_owned(),
    ];
    let client = CodexClient::start_with_config_overrides("codex", &overrides)
        .await
        .expect("Codex app-server");
    let cwd = env::current_dir()
        .expect("workspace")
        .to_string_lossy()
        .into_owned();
    let thread = client
        .start_thread(&ThreadStartParams {
            cwd: Some(cwd.clone()),
            ephemeral: Some(true),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread");
    let thread_id = thread.thread.id;
    let mut events = client.subscribe();
    let turn = client
        .start_turn(&TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Call the bot_elicitation_fixture request_value MCP tool exactly once. When it requests a value, wait for the user response. Then reply with exactly BOT_MCP_ELICITATION_DONE."
                    .to_owned(),
                text_elements: Vec::new(),
            }],
            model: None,
            effort: None,
            summary: Some(ReasoningSummary::Auto),
            cwd: Some(cwd),
            client_user_message_id: Some("bot-live-mcp-elicitation-1".to_owned()),
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            collaboration_mode: None,
        })
        .await
        .expect("turn");
    let turn_id = turn.turn.id;
    let result = timeout(Duration::from_secs(60), async {
        let mut tool_approved = false;
        let mut elicited = false;
        let mut output = String::new();
        loop {
            match events.recv().await.expect("Codex event") {
                CodexEvent::Request(request)
                    if request.method == "mcpServer/elicitation/request" =>
                {
                    let params: McpServerElicitationRequestParams =
                        serde_json::from_value(request.params).expect("MCP elicitation");
                    assert_eq!(params.thread_id, thread_id);
                    assert_eq!(params.turn_id.as_deref(), Some(turn_id.as_str()));
                    assert_eq!(params.server_name, "bot_elicitation_fixture");
                    let McpServerElicitationRequest::Form {
                        meta,
                        message,
                        requested_schema,
                    } = params.request
                    else {
                        panic!("form elicitation");
                    };
                    let approval_kind = meta
                        .as_ref()
                        .and_then(|value| value.get("codex_approval_kind"))
                        .and_then(|value| value.as_str());
                    let content = if approval_kind == Some("mcp_tool_call") {
                        let meta = meta.expect("Codex approval metadata");
                        assert_eq!(
                            meta["tool_description"],
                            "Request the Bot MCP verification value."
                        );
                        assert_eq!(
                            message,
                            "Allow the bot_elicitation_fixture MCP server to run tool \"request_value\"?"
                        );
                        assert_eq!(requested_schema, json!({"type": "object", "properties": {}}));
                        tool_approved = true;
                        Some(json!({}))
                    } else {
                        assert_eq!(meta, Some(json!({"trace": "bot-live-mcp"})));
                        assert_eq!(message, "Enter the Bot MCP verification value.");
                        assert_eq!(requested_schema["required"], json!(["value"]));
                        elicited = true;
                        Some(json!({"value": "BOT_MCP_VALUE"}))
                    };
                    client
                        .respond(
                            request.id,
                            &McpServerElicitationRequestResponse {
                                action: McpServerElicitationAction::Accept,
                                content,
                                meta: None,
                                extra: Default::default(),
                            },
                        )
                        .await
                        .expect("MCP elicitation response");
                }
                CodexEvent::Notification(notification)
                    if notification.method == "item/agentMessage/delta"
                        && notification.params.get("turnId").and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    if let Some(delta) = notification
                        .params
                        .get("delta")
                        .and_then(|value| value.as_str())
                    {
                        output.push_str(delta);
                    }
                }
                CodexEvent::Notification(notification)
                    if notification.method == "turn/completed"
                        && notification
                            .params
                            .get("turn")
                            .and_then(|turn| turn.get("id"))
                            .and_then(|id| id.as_str())
                            == Some(turn_id.as_str()) =>
                {
                    break (tool_approved, elicited, output);
                }
                CodexEvent::ConnectionClosed(message) => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("live MCP elicitation timeout");
    client.shutdown().await.expect("shutdown");
    assert!(result.0);
    assert!(result.1);
    assert!(result.2.trim_end().ends_with("BOT_MCP_ELICITATION_DONE"));
}

async fn collect_text_turn(events: &mut Receiver<CodexEvent>, turn_id: &str) -> (String, String) {
    let mut output = String::new();
    loop {
        match events.recv().await.expect("Codex event") {
            CodexEvent::Notification(notification)
                if notification.method == "item/agentMessage/delta"
                    && notification.params.get("turnId").and_then(|id| id.as_str())
                        == Some(turn_id) =>
            {
                if let Some(delta) = notification
                    .params
                    .get("delta")
                    .and_then(|value| value.as_str())
                {
                    output.push_str(delta);
                }
            }
            CodexEvent::Notification(notification)
                if notification.method == "turn/completed"
                    && notification
                        .params
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .and_then(|id| id.as_str())
                        == Some(turn_id) =>
            {
                let status = notification
                    .params
                    .get("turn")
                    .and_then(|turn| turn.get("status"))
                    .and_then(|status| status.as_str())
                    .unwrap_or_default()
                    .to_owned();
                break (output, status);
            }
            CodexEvent::ConnectionClosed(message) => panic!("{message}"),
            _ => {}
        }
    }
}

const TINY_PNG_BYTES: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2, 0, 0, 5, 0, 1,
    122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
