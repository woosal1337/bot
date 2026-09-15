use std::path::PathBuf;

use bot_provider_codex::{
    AccountLoginCompletedNotification, AccountRateLimitsResponse, AccountReadParams,
    AccountUpdatedNotification, CancelLoginAccountParams, ClientInfo,
    CommandExecutionApprovalDecision, CommandExecutionRequestApprovalResponse,
    ConfigValueWriteParams, DynamicToolCallOutputContentItem, DynamicToolCallParams,
    DynamicToolCallResponse, DynamicToolNamespaceTool, DynamicToolSpec, FileChangeApprovalDecision,
    FileChangeRequestApprovalResponse, HookEventName, HookHandlerMetadata, HookSource,
    HookTrustStatus, HooksListParams, HooksListResponse, IncomingMessage, InitializeParams,
    InitializeResponse, InputModality, LoginAccountParams, LoginAccountResponse,
    MCP_SERVER_RELOAD_METHOD, McpAuthStatus, McpServerConnectionStatus, McpServerElicitationAction,
    McpServerElicitationRequest, McpServerElicitationRequestParams,
    McpServerElicitationRequestResponse, McpServerRefreshResponse, McpServerStatusDetail,
    McpServerStatusListParams, McpServerStatusListResponse, MergeStrategy, ModelListResponse,
    PermissionGrantScope, PermissionsRequestApprovalResponse, PluginInstallParams,
    PluginListParams, PluginListResponse, PluginReconcileParams, PluginSource,
    PluginUninstallParams, ProtocolError, ReasoningSummary, RequestId, SkillScope,
    SkillsConfigWriteParams, SkillsListParams, SkillsListResponse, ThreadCompactStartParams,
    ThreadDeleteParams, ThreadForkParams, ThreadListParams, ThreadResumeParams, ThreadRevertParams,
    ThreadSearchParams, ThreadSetNameParams, ThreadStartParams, ThreadStartResponse,
    ThreadTurnsListParams, ToolRequestUserInputParams, ToolRequestUserInputResponse,
    TurnDiffUpdatedNotification, TurnInterruptParams, TurnStartParams, TurnSteerParams, UserInput,
    decode_line, encode_error_response, encode_notification, encode_request, encode_response,
};
use serde_json::{Value, json};

#[test]
fn collaboration_modes_keep_native_instructions_and_selected_settings() {
    use bot_provider_codex::{CollaborationMode, CollaborationModeKind, CollaborationModeSettings};
    for (mode, expected) in [
        (CollaborationModeKind::Plan, "plan"),
        (CollaborationModeKind::Default, "default"),
    ] {
        let value = serde_json::to_value(CollaborationMode {
            mode,
            settings: CollaborationModeSettings {
                model: "test-model".into(),
                reasoning_effort: Some("low".into()),
                developer_instructions: None,
            },
        })
        .unwrap();
        assert_eq!(
            value,
            json!({"mode": expected, "settings": {"model": "test-model", "reasoning_effort": "low", "developer_instructions": null}})
        );
    }
}

#[test]
fn encodes_an_initialize_request_as_jsonl() {
    let params = InitializeParams {
        client_info: ClientInfo {
            name: "bot".to_owned(),
            title: Some("Bot".to_owned()),
            version: "0.1.0".to_owned(),
        },
        capabilities: None,
    };
    let line = encode_request(RequestId::Number(1), "initialize", &params).expect("request");
    let value: Value = serde_json::from_str(&line).expect("JSON line");
    assert_eq!(
        value,
        json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "bot",
                    "title": "Bot",
                    "version": "0.1.0"
                }
            }
        })
    );
    assert!(line.ends_with('\n'));
}

#[test]
fn encodes_an_initialized_notification() {
    let line = encode_notification("initialized", &json!({})).expect("notification");
    let value: Value = serde_json::from_str(&line).expect("JSON line");
    assert_eq!(value, json!({"method": "initialized", "params": {}}));
}

#[test]
fn encodes_native_compaction_requests() {
    let value = serde_json::to_value(ThreadCompactStartParams {
        thread_id: "thread-1".to_owned(),
    })
    .expect("compact request");
    assert_eq!(value, json!({"threadId": "thread-1"}));
}

#[test]
fn encodes_dynamic_tools_on_thread_start() {
    let value = serde_json::to_value(ThreadStartParams {
        dynamic_tools: Some(vec![
            DynamicToolSpec::Function {
                name: "lookup".to_owned(),
                description: "Look up a value".to_owned(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"]
                }),
                defer_loading: Some(false),
            },
            DynamicToolSpec::Namespace {
                name: "records".to_owned(),
                description: "Work with records".to_owned(),
                tools: vec![DynamicToolNamespaceTool::Function {
                    name: "read".to_owned(),
                    description: "Read a record".to_owned(),
                    input_schema: json!({"type": "object"}),
                    defer_loading: None,
                }],
            },
        ]),
        ..ThreadStartParams::default()
    })
    .expect("thread start");
    assert_eq!(
        value,
        json!({
            "dynamicTools": [
                {
                    "type": "function",
                    "name": "lookup",
                    "description": "Look up a value",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"key": {"type": "string"}},
                        "required": ["key"]
                    },
                    "deferLoading": false
                },
                {
                    "type": "namespace",
                    "name": "records",
                    "description": "Work with records",
                    "tools": [{
                        "type": "function",
                        "name": "read",
                        "description": "Read a record",
                        "inputSchema": {"type": "object"}
                    }]
                }
            ]
        })
    );
}

#[test]
fn decodes_and_encodes_dynamic_tool_calls() {
    let params: DynamicToolCallParams = serde_json::from_value(json!({
        "threadId": "thread-1",
        "turnId": "turn-1",
        "callId": "call-1",
        "tool": "read",
        "namespace": "records",
        "arguments": {"key": "alpha"},
        "futureField": {"kept": true}
    }))
    .expect("dynamic tool call");
    assert_eq!(params.tool, "read");
    assert_eq!(params.namespace.as_deref(), Some("records"));
    assert_eq!(params.arguments, json!({"key": "alpha"}));
    assert_eq!(params.extra["futureField"], json!({"kept": true}));

    let response = serde_json::to_value(DynamicToolCallResponse {
        success: true,
        content_items: vec![
            DynamicToolCallOutputContentItem::Text {
                text: "done".to_owned(),
            },
            DynamicToolCallOutputContentItem::Image {
                image_url: "data:image/png;base64,AA==".to_owned(),
            },
            DynamicToolCallOutputContentItem::Audio {
                audio_url: "data:audio/wav;base64,AA==".to_owned(),
            },
        ],
    })
    .expect("dynamic tool response");
    assert_eq!(
        response,
        json!({
            "success": true,
            "contentItems": [
                {"type": "inputText", "text": "done"},
                {"type": "inputImage", "imageUrl": "data:image/png;base64,AA=="},
                {"type": "inputAudio", "audioUrl": "data:audio/wav;base64,AA=="}
            ]
        })
    );
}

#[test]
fn encodes_native_session_management_requests() {
    let rename = serde_json::to_value(ThreadSetNameParams {
        thread_id: "thread-1".to_owned(),
        name: "Focused work".to_owned(),
    })
    .expect("rename request");
    let delete = serde_json::to_value(ThreadDeleteParams {
        thread_id: "thread-1".to_owned(),
    })
    .expect("delete request");
    let fork = serde_json::to_value(ThreadForkParams {
        thread_id: "thread-1".to_owned(),
        cwd: Some("/tmp/project".to_owned()),
    })
    .expect("fork request");
    assert_eq!(
        rename,
        json!({"threadId": "thread-1", "name": "Focused work"})
    );
    assert_eq!(delete, json!({"threadId": "thread-1"}));
    assert_eq!(fork, json!({"threadId": "thread-1", "cwd": "/tmp/project"}));
}

#[test]
fn encodes_server_request_responses() {
    let command = encode_response(
        RequestId::Number(11),
        &CommandExecutionRequestApprovalResponse {
            decision: CommandExecutionApprovalDecision::Accept,
        },
    )
    .expect("command response");
    let file_change = encode_response(
        RequestId::String("approval-12".to_owned()),
        &FileChangeRequestApprovalResponse {
            decision: FileChangeApprovalDecision::Decline,
        },
    )
    .expect("file response");
    let permissions = encode_response(
        RequestId::Number(13),
        &PermissionsRequestApprovalResponse {
            permissions: json!({}),
            scope: PermissionGrantScope::Turn,
            strict_auto_review: None,
        },
    )
    .expect("permission response");
    assert_eq!(
        serde_json::from_str::<Value>(&command).expect("command JSON"),
        json!({"id": 11, "result": {"decision": "accept"}})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&file_change).expect("file JSON"),
        json!({"id": "approval-12", "result": {"decision": "decline"}})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&permissions).expect("permission JSON"),
        json!({"id": 13, "result": {"permissions": {}, "scope": "turn"}})
    );
}

#[test]
fn encodes_server_request_errors() {
    let line = encode_error_response(
        RequestId::Number(14),
        &bot_provider_codex::RemoteError {
            code: -32601,
            message: "Unsupported method".to_owned(),
            data: None,
        },
    )
    .expect("error response");
    assert_eq!(
        serde_json::from_str::<Value>(&line).expect("error JSON"),
        json!({"id": 14, "error": {"code": -32601, "message": "Unsupported method"}})
    );
}

#[test]
fn decodes_a_typed_initialize_response() {
    let message = decode_line(include_str!("fixtures/initialize-response.jsonl")).expect("message");
    let IncomingMessage::Response(response) = message else {
        panic!("response");
    };
    let value = response.outcome.expect("result");
    let initialized: InitializeResponse = serde_json::from_value(value).expect("initialize result");
    assert_eq!(initialized.platform_os, "macos");
    assert_eq!(initialized.codex_home.to_string_lossy(), "/tmp/codex");
}

#[test]
fn preserves_unknown_model_fields() {
    let message = decode_line(include_str!("fixtures/model-list-response.jsonl")).expect("message");
    let IncomingMessage::Response(response) = message else {
        panic!("response");
    };
    let models: ModelListResponse =
        serde_json::from_value(response.outcome.expect("result")).expect("model list");
    assert_eq!(models.data.len(), 1);
    assert_eq!(models.data[0].input_modalities[1], InputModality::Image);
    assert_eq!(models.data[0].extra["futureField"], json!({"kept": true}));
}

#[test]
fn decodes_the_current_turn_diff_notification() {
    let message =
        decode_line(include_str!("fixtures/turn-diff-updated.jsonl")).expect("notification");
    let IncomingMessage::Notification(notification) = message else {
        panic!("notification");
    };
    assert_eq!(notification.method, "turn/diff/updated");
    let update: TurnDiffUpdatedNotification =
        serde_json::from_value(notification.params).expect("turn diff");
    assert_eq!(update.thread_id, "thread-1");
    assert_eq!(update.turn_id, "turn-1");
    assert!(update.diff.contains("-old\n+new"));
    assert_eq!(update.extra["futureField"], json!({"kept": true}));
}

#[test]
fn decodes_current_request_user_input_variants() {
    let requests = include_str!("fixtures/request-user-input.jsonl")
        .lines()
        .map(|line| {
            let IncomingMessage::Request(request) = decode_line(line).expect("request") else {
                panic!("server request");
            };
            assert_eq!(request.method, "item/tool/requestUserInput");
            serde_json::from_value::<ToolRequestUserInputParams>(request.params)
                .expect("request user input")
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].is_blocking);
    assert!(requests[0].questions[0].is_other);
    assert!(!requests[0].questions[0].is_secret);
    assert_eq!(
        requests[0].questions[0].options.as_ref().map(Vec::len),
        Some(2)
    );
    assert!(!requests[1].is_blocking);
    assert!(requests[1].questions[0].is_secret);
    assert!(requests[1].questions[0].options.is_none());
    assert_eq!(requests[1].extra["futureField"], json!({"kept": true}));
}

#[test]
fn request_user_input_defaults_legacy_requests_to_blocking() {
    let request: ToolRequestUserInputParams = serde_json::from_value(json!({
        "threadId": "thread-1",
        "turnId": "turn-1",
        "itemId": "question-1",
        "questions": []
    }))
    .expect("legacy request user input");
    assert!(request.is_blocking);
}

#[test]
fn encodes_current_request_user_input_response() {
    let response: ToolRequestUserInputResponse = serde_json::from_value(json!({
        "answers": {
            "environment": {"answers": ["Production", "user_note: blue"]},
            "token": {"answers": ["secret-value"]}
        }
    }))
    .expect("request user input response");
    assert_eq!(
        serde_json::to_value(response).expect("response"),
        json!({
            "answers": {
                "environment": {"answers": ["Production", "user_note: blue"]},
                "token": {"answers": ["secret-value"]}
            }
        })
    );
}

#[test]
fn decodes_all_current_mcp_elicitation_modes() {
    let requests = include_str!("fixtures/mcp-elicitation-requests.jsonl")
        .lines()
        .map(|line| {
            let IncomingMessage::Request(request) = decode_line(line).expect("request") else {
                panic!("server request");
            };
            serde_json::from_value::<McpServerElicitationRequestParams>(request.params)
                .expect("MCP elicitation")
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 5);
    let McpServerElicitationRequest::Form {
        meta,
        message,
        requested_schema,
    } = &requests[0].request
    else {
        panic!("form");
    };
    assert_eq!(meta.as_ref().expect("meta")["trace"], "form-1");
    assert_eq!(message, "Enter a value");
    assert_eq!(requested_schema["properties"]["value"]["type"], "string");
    assert_eq!(requests[0].extra["futureField"], json!({"kept": true}));
    assert!(matches!(
        requests[1].request,
        McpServerElicitationRequest::OpenAiForm { .. }
    ));
    assert!(matches!(
        requests[2].request,
        McpServerElicitationRequest::OpenAiElicitationForm { .. }
    ));
    assert!(matches!(
        requests[3].request,
        McpServerElicitationRequest::UserVerification { .. }
    ));
    assert!(matches!(
        requests[4].request,
        McpServerElicitationRequest::Url { .. }
    ));
}

#[test]
fn encodes_the_current_mcp_elicitation_response() {
    let response = McpServerElicitationRequestResponse {
        action: McpServerElicitationAction::Accept,
        content: Some(json!({"value": "done"})),
        meta: Some(json!({"source": "bot"})),
        extra: Default::default(),
    };
    assert_eq!(
        serde_json::to_value(response).expect("response"),
        json!({
            "action": "accept",
            "content": {"value": "done"},
            "_meta": {"source": "bot"}
        })
    );
}

#[test]
fn encodes_native_mcp_inventory_requests() {
    let params = McpServerStatusListParams {
        cursor: Some("page-2".to_owned()),
        detail: Some(McpServerStatusDetail::Full),
        limit: Some(100),
        thread_id: Some("thread-1".to_owned()),
    };
    let value = serde_json::to_value(params).expect("MCP list params");
    assert_eq!(
        value,
        json!({
            "cursor": "page-2",
            "detail": "full",
            "limit": 100,
            "threadId": "thread-1"
        })
    );
}

#[test]
fn encodes_native_mcp_reload_requests() {
    let line = encode_request(RequestId::Number(21), MCP_SERVER_RELOAD_METHOD, &())
        .expect("MCP reload request");
    let value: Value = serde_json::from_str(&line).expect("MCP reload JSON");
    assert_eq!(
        value,
        json!({"id": 21, "method": "config/mcpServer/reload", "params": null})
    );

    let response: McpServerRefreshResponse =
        serde_json::from_value(json!({})).expect("MCP reload response");
    assert_eq!(response, McpServerRefreshResponse::default());
}

#[test]
fn decodes_native_mcp_inventory_responses() {
    let response: McpServerStatusListResponse = serde_json::from_value(json!({
        "data": [{
            "name": "docs",
            "authStatus": "oAuth",
            "pluginId": "docs-plugin",
            "runtimeStatus": "connected",
            "serverInfo": {"name": "Docs", "title": "Documentation", "version": "1.0.0"},
            "tools": {
                "read": {
                    "name": "read",
                    "title": "Read docs",
                    "description": "Reads documentation",
                    "inputSchema": {"type": "object"}
                }
            },
            "resources": [],
            "resourceTemplates": [],
            "toolsError": null
        }],
        "nextCursor": null
    }))
    .expect("MCP list response");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].auth_status, McpAuthStatus::OAuth);
    assert_eq!(
        response.data[0].runtime_status,
        Some(McpServerConnectionStatus::Connected)
    );
    assert_eq!(
        response.data[0].tools["read"].title.as_deref(),
        Some("Read docs")
    );
}

#[test]
fn encodes_native_plugin_inventory_requests() {
    let params = PluginListParams {
        cwds: Some(vec![PathBuf::from("/work/bot")]),
        force_refetch: false,
    };
    let value = serde_json::to_value(params).expect("plugin list params");
    assert_eq!(
        value,
        json!({
            "cwds": ["/work/bot"],
            "forceRefetch": false
        })
    );
}

#[test]
fn decodes_native_plugin_inventory_responses() {
    let response: PluginListResponse = serde_json::from_value(json!({
        "marketplaces": [{
            "name": "Fixture marketplace",
            "path": "/work/bot/.codex/marketplace.json",
            "plugins": [{
                "id": "docs-plugin",
                "name": "docs-plugin",
                "enabled": true,
                "installed": true,
                "localVersion": "1.2.3",
                "version": null,
                "interface": {
                    "displayName": "Docs plugin",
                    "shortDescription": "Reads project documentation"
                },
                "source": {
                    "type": "local",
                    "path": "/work/bot/.codex/plugins/docs"
                }
            }]
        }]
    }))
    .expect("plugin list response");
    assert_eq!(response.marketplaces.len(), 1);
    assert!(response.marketplaces[0].plugins[0].installed);
    assert_eq!(
        response.marketplaces[0].plugins[0]
            .interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref()),
        Some("Docs plugin")
    );
    assert!(matches!(
        &response.marketplaces[0].plugins[0].source,
        PluginSource::Local { path } if path == &PathBuf::from("/work/bot/.codex/plugins/docs")
    ));
}

#[test]
fn encodes_native_hooks_requests() {
    let request = serde_json::to_value(HooksListParams {
        cwds: vec![PathBuf::from("/work/bot")],
    })
    .expect("hooks list params");
    assert_eq!(request, json!({"cwds": ["/work/bot"]}));
}

#[test]
fn decodes_native_hooks_inventory_responses() {
    let response: HooksListResponse = serde_json::from_value(json!({
        "data": [{
            "cwd": "/work/bot",
            "hooks": [{
                "key": "project/pre-tool",
                "eventName": "preToolUse",
                "matcher": "shell",
                "handlerType": "command",
                "command": "/work/bot/.codex/hooks/check",
                "timeoutSec": 10,
                "async": false,
                "source": "project",
                "sourcePath": "/work/bot/.codex/config.toml",
                "pluginId": null,
                "enabled": true,
                "isManaged": false,
                "trustStatus": "trusted",
                "currentHash": "sha256:fixture",
                "displayOrder": 0,
                "statusMessage": null,
                "additionalContextLimit": null
            }, {
                "key": "plugin/tool",
                "eventName": "postToolUse",
                "matcher": null,
                "handlerType": "mcpTool",
                "server": "docs",
                "tool": "record",
                "timeoutSec": 15,
                "source": "plugin",
                "sourcePath": "/work/bot/.codex/plugins/docs/plugin.json",
                "pluginId": "docs-plugin@fixture-marketplace",
                "enabled": false,
                "isManaged": false,
                "trustStatus": "untrusted",
                "currentHash": "sha256:plugin",
                "displayOrder": 1,
                "statusMessage": "Disabled until trusted",
                "additionalContextLimit": 2500
            }],
            "errors": [],
            "warnings": []
        }]
    }))
    .expect("hooks list response");
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data[0].hooks[0].event_name,
        HookEventName::PreToolUse
    );
    assert_eq!(response.data[0].hooks[0].source, HookSource::Project);
    assert_eq!(
        response.data[0].hooks[0].trust_status,
        HookTrustStatus::Trusted
    );
    assert!(matches!(
        response.data[0].hooks[1].handler,
        HookHandlerMetadata::McpTool { .. }
    ));
}

#[test]
fn encodes_native_skill_requests() {
    let list = serde_json::to_value(SkillsListParams {
        cwds: vec![PathBuf::from("/work/bot")],
        force_reload: true,
    })
    .expect("skills list params");
    assert_eq!(
        list,
        json!({
            "cwds": ["/work/bot"],
            "forceReload": true
        })
    );

    let write = serde_json::to_value(SkillsConfigWriteParams {
        path: None,
        name: Some("repo-skill".to_owned()),
        enabled: false,
    })
    .expect("skill config params");
    assert_eq!(write, json!({"name": "repo-skill", "enabled": false}));
}

#[test]
fn decodes_native_skill_inventory_responses() {
    let response: SkillsListResponse = serde_json::from_value(json!({
        "data": [{
            "cwd": "/work/bot",
            "skills": [{
                "name": "repo-skill",
                "description": "Checks the repository",
                "enabled": true,
                "path": "/work/bot/.agents/skills/repo-skill/SKILL.md",
                "scope": "repo",
                "interface": {
                    "displayName": "Repository check",
                    "shortDescription": "Check this repository",
                    "defaultPrompt": null
                },
                "pluginId": null,
                "shortDescription": null,
                "dependencies": {"tools": []}
            }],
            "errors": []
        }]
    }))
    .expect("skills list response");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].skills[0].scope, SkillScope::Repo);
    assert_eq!(
        response.data[0].skills[0]
            .interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref()),
        Some("Repository check")
    );
}

#[test]
fn encodes_native_skill_input() {
    assert_eq!(
        serde_json::to_value(UserInput::Skill {
            name: "repo-skill".to_owned(),
            path: PathBuf::from("/work/.agents/skills/repo-skill/SKILL.md"),
        })
        .unwrap(),
        json!({
            "type": "skill",
            "name": "repo-skill",
            "path": "/work/.agents/skills/repo-skill/SKILL.md"
        })
    );
}

#[test]
fn encodes_native_plugin_actions() {
    assert_eq!(
        serde_json::to_value(PluginInstallParams {
            marketplace_path: Some(PathBuf::from("/work/marketplace")),
            remote_marketplace_name: None,
            install_attempt_id: None,
            plugin_name: "docs".to_owned(),
        })
        .unwrap(),
        json!({
            "marketplacePath": "/work/marketplace",
            "pluginName": "docs"
        })
    );
    assert_eq!(
        serde_json::to_value(PluginUninstallParams {
            plugin_id: "docs@fixture".to_owned(),
        })
        .unwrap(),
        json!({"pluginId": "docs@fixture"})
    );
    assert_eq!(
        serde_json::to_value(PluginReconcileParams {
            reason: Some("Bot plugin reload".to_owned()),
        })
        .unwrap(),
        json!({"reason": "Bot plugin reload"})
    );
    assert_eq!(
        serde_json::to_value(ConfigValueWriteParams {
            key_path: "plugins.\"docs@fixture\".enabled".to_owned(),
            value: Value::Bool(false),
            merge_strategy: MergeStrategy::Upsert,
            file_path: None,
            expected_version: None,
        })
        .unwrap(),
        json!({
            "keyPath": "plugins.\"docs@fixture\".enabled",
            "value": false,
            "mergeStrategy": "upsert"
        })
    );
}

#[test]
fn separates_notifications_and_server_requests() {
    let notification = decode_line(include_str!("fixtures/notification.jsonl")).expect("message");
    let request = decode_line(include_str!("fixtures/server-request.jsonl")).expect("message");
    let IncomingMessage::Notification(notification) = notification else {
        panic!("notification");
    };
    let IncomingMessage::Request(request) = request else {
        panic!("request");
    };
    assert_eq!(notification.method, "thread/started");
    assert_eq!(notification.params["futureField"], true);
    assert_eq!(request.id, RequestId::String("approval-1".to_owned()));
    assert_eq!(request.method, "item/commandExecution/requestApproval");
}

#[test]
fn decodes_the_command_execution_lifecycle_fixture() {
    let messages = include_str!("fixtures/command-execution.jsonl")
        .lines()
        .map(decode_line)
        .collect::<Result<Vec<_>, _>>()
        .expect("command fixture");
    assert_eq!(messages.len(), 3);
    let IncomingMessage::Notification(started) = &messages[0] else {
        panic!("started notification");
    };
    let IncomingMessage::Notification(output) = &messages[1] else {
        panic!("output notification");
    };
    let IncomingMessage::Notification(completed) = &messages[2] else {
        panic!("completed notification");
    };
    assert_eq!(
        started.params["item"]["command"],
        "find . -maxdepth 1 -type f"
    );
    assert_eq!(output.params["delta"], "./.DS_Store\n");
    assert_eq!(completed.params["item"]["exitCode"], 0);
    assert_eq!(completed.params["item"]["cwd"], "/workspace");
}

#[test]
fn decodes_remote_errors() {
    let message = decode_line(
        r#"{"id":7,"error":{"code":-32602,"message":"Invalid params","data":{"field":"cwd"}}}"#,
    )
    .expect("message");
    let IncomingMessage::Response(response) = message else {
        panic!("response");
    };
    let error = response.outcome.expect_err("remote error");
    assert_eq!(error.code, -32602);
    assert_eq!(error.data.expect("data")["field"], "cwd");
}

#[test]
fn rejects_malformed_envelopes() {
    assert!(matches!(
        decode_line("[]"),
        Err(ProtocolError::ExpectedObject)
    ));
    assert!(matches!(
        decode_line(r#"{"id":1}"#),
        Err(ProtocolError::InvalidResponse)
    ));
    assert!(matches!(
        decode_line(r#"{"method":3,"params":{}}"#),
        Err(ProtocolError::InvalidField("method"))
    ));
}

#[test]
fn omits_the_account_refresh_flag_by_default() {
    let params = serde_json::to_value(AccountReadParams::default()).expect("params");
    assert_eq!(params, json!({}));
}

#[test]
fn encodes_supported_account_login_requests() {
    let browser = serde_json::to_value(LoginAccountParams::chatgpt()).expect("browser login");
    let device = serde_json::to_value(LoginAccountParams::ChatgptDeviceCode).expect("device login");
    let cancel = serde_json::to_value(CancelLoginAccountParams {
        login_id: "login-1".to_owned(),
    })
    .expect("cancel login");
    assert_eq!(browser, json!({"type": "chatgpt"}));
    assert_eq!(device, json!({"type": "chatgptDeviceCode"}));
    assert_eq!(cancel, json!({"loginId": "login-1"}));
}

#[test]
fn decodes_account_login_responses_and_notifications() {
    let responses = include_str!("fixtures/account-login-responses.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<LoginAccountResponse>(line).expect("login response"))
        .collect::<Vec<_>>();
    assert_eq!(responses[0].kind, "chatgpt");
    assert_eq!(responses[0].login_id.as_deref(), Some("login-browser"));
    assert_eq!(
        responses[0].auth_url.as_deref(),
        Some("https://auth.openai.com/browser")
    );
    assert_eq!(responses[1].kind, "chatgptDeviceCode");
    assert_eq!(responses[1].user_code.as_deref(), Some("ABCD-EFGH"));
    assert_eq!(responses[1].extra["futureField"], true);

    let completed: AccountLoginCompletedNotification =
        serde_json::from_str(include_str!("fixtures/account-login-completed.json"))
            .expect("login notification");
    assert!(completed.success);
    assert_eq!(completed.login_id.as_deref(), Some("login-browser"));
    assert_eq!(completed.extra["futureField"], 4);

    let updated: AccountUpdatedNotification = serde_json::from_value(json!({
        "authMode": "chatgpt",
        "planType": "pro",
        "futureField": "kept"
    }))
    .expect("account notification");
    assert_eq!(updated.auth_mode.as_deref(), Some("chatgpt"));
    assert_eq!(updated.extra["futureField"], "kept");
}

#[test]
fn decodes_account_rate_limits_and_preserves_unknown_fields() {
    let limits: AccountRateLimitsResponse =
        serde_json::from_str(include_str!("fixtures/account-rate-limits-response.json"))
            .expect("rate limits");
    assert_eq!(
        limits
            .rate_limits_by_limit_id
            .as_ref()
            .and_then(|items| items.get("codex"))
            .and_then(|snapshot| snapshot.primary.as_ref())
            .map(|window| window.used_percent),
        Some(37)
    );
    assert_eq!(
        limits
            .rate_limit_reset_credits
            .as_ref()
            .map(|credits| credits.available_count),
        Some(2)
    );
    assert_eq!(limits.rate_limits.extra["futureSnapshotField"], true);
    assert_eq!(limits.extra["futureResponseField"], "kept");
}

#[test]
fn encodes_a_text_turn_with_selected_settings() {
    let params = TurnStartParams {
        thread_id: "thread-1".to_owned(),
        input: vec![UserInput::Text {
            text: "Explain this workspace.".to_owned(),
            text_elements: Vec::new(),
        }],
        model: Some("gpt-5.4".to_owned()),
        effort: Some("high".to_owned()),
        summary: Some(ReasoningSummary::Auto),
        cwd: Some("/work/bot".to_owned()),
        client_user_message_id: Some("bot-message-1".to_owned()),
        approval_policy: Some("on-request".to_owned()),
        approvals_reviewer: Some("auto_review".to_owned()),
        sandbox_policy: Some(json!({"type": "workspaceWrite"})),
        collaboration_mode: None,
    };
    let line = encode_request(RequestId::Number(8), "turn/start", &params).expect("request");
    let value: Value = serde_json::from_str(&line).expect("JSON line");
    assert_eq!(
        value,
        json!({
            "id": 8,
            "method": "turn/start",
            "params": {
                "threadId": "thread-1",
                "input": [{"type": "text", "text": "Explain this workspace."}],
                "model": "gpt-5.4",
                "effort": "high",
                "summary": "auto",
                "cwd": "/work/bot",
                "clientUserMessageId": "bot-message-1",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "auto_review",
                "sandboxPolicy": {"type": "workspaceWrite"}
            }
        })
    );
}

#[test]
fn encodes_a_turn_interrupt() {
    let params = TurnInterruptParams {
        thread_id: "thread-1".to_owned(),
        turn_id: "turn-1".to_owned(),
    };
    let line = encode_request(RequestId::Number(9), "turn/interrupt", &params).expect("request");
    let value: Value = serde_json::from_str(&line).expect("JSON line");
    assert_eq!(
        value,
        json!({
            "id": 9,
            "method": "turn/interrupt",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        })
    );
}

#[test]
fn encodes_a_turn_steer_request() {
    let line = encode_request(
        RequestId::Number(10),
        "turn/steer",
        &TurnSteerParams {
            thread_id: "thread-1".to_owned(),
            expected_turn_id: "turn-1".to_owned(),
            input: vec![UserInput::Text {
                text: "Use the smaller test set.".to_owned(),
                text_elements: Vec::new(),
            }],
            client_user_message_id: Some("interjection-1".to_owned()),
        },
    )
    .expect("steer request");
    assert_eq!(
        serde_json::from_str::<Value>(&line).expect("steer JSON"),
        json!({
            "id": 10,
            "method": "turn/steer",
            "params": {
                "threadId": "thread-1",
                "expectedTurnId": "turn-1",
                "input": [{"type": "text", "text": "Use the smaller test set."}],
                "clientUserMessageId": "interjection-1"
            }
        })
    );
}

#[test]
fn encodes_thread_list_and_resume_requests() {
    let list = encode_request(
        RequestId::Number(10),
        "thread/list",
        &ThreadListParams {
            limit: Some(50),
            cwd: Some("/work/bot".to_owned()),
            search_term: Some("provider".to_owned()),
            sort_key: Some("updated_at".to_owned()),
            sort_direction: Some("desc".to_owned()),
            ..ThreadListParams::default()
        },
    )
    .expect("list request");
    let resume = encode_request(
        RequestId::Number(11),
        "thread/resume",
        &ThreadResumeParams {
            thread_id: "thread-1".to_owned(),
            model: Some("gpt-5.4".to_owned()),
            cwd: Some("/work/bot".to_owned()),
            approval_policy: Some("on-request".to_owned()),
            approvals_reviewer: None,
            sandbox: Some("workspace-write".to_owned()),
        },
    )
    .expect("resume request");
    assert_eq!(
        serde_json::from_str::<Value>(&list).expect("list JSON"),
        json!({
            "id": 10,
            "method": "thread/list",
            "params": {
                "limit": 50,
                "cwd": "/work/bot",
                "searchTerm": "provider",
                "sortKey": "updated_at",
                "sortDirection": "desc"
            }
        })
    );
    assert_eq!(
        serde_json::from_str::<Value>(&resume).expect("resume JSON"),
        json!({
            "id": 11,
            "method": "thread/resume",
            "params": {
                "threadId": "thread-1",
                "model": "gpt-5.4",
                "cwd": "/work/bot",
                "approvalPolicy": "on-request",
                "approvalsReviewer": null,
                "sandbox": "workspace-write"
            }
        })
    );
}

#[test]
fn encodes_a_thread_search_request() {
    let line = encode_request(
        RequestId::Number(12),
        "thread/search",
        &ThreadSearchParams {
            cursor: None,
            limit: Some(20),
            sort_key: Some("updated_at".to_owned()),
            sort_direction: Some("desc".to_owned()),
            source_kinds: None,
            archived: Some(false),
            search_term: "provider authentication".to_owned(),
        },
    )
    .expect("search request");
    assert_eq!(
        serde_json::from_str::<Value>(&line).expect("search JSON"),
        json!({
            "id": 12,
            "method": "thread/search",
            "params": {
                "limit": 20,
                "sortKey": "updated_at",
                "sortDirection": "desc",
                "archived": false,
                "searchTerm": "provider authentication"
            }
        })
    );
}

#[test]
fn encodes_thread_turn_listing_and_revert_requests() {
    let list = encode_request(
        RequestId::Number(12),
        "thread/turns/list",
        &ThreadTurnsListParams {
            thread_id: "thread-1".to_owned(),
            cursor: Some("page-2".to_owned()),
            limit: Some(100),
            sort_direction: Some("asc".to_owned()),
            items_view: Some("full".to_owned()),
        },
    )
    .expect("turn list request");
    assert_eq!(
        serde_json::from_str::<Value>(&list).expect("turn list JSON"),
        json!({
            "id": 12,
            "method": "thread/turns/list",
            "params": {
                "threadId": "thread-1",
                "cursor": "page-2",
                "limit": 100,
                "sortDirection": "asc",
                "itemsView": "full"
            }
        })
    );

    let revert = encode_request(
        RequestId::Number(13),
        "thread/revert",
        &ThreadRevertParams {
            thread_id: "thread-1".to_owned(),
            before_turn_id: "turn-2".to_owned(),
        },
    )
    .expect("revert request");
    assert_eq!(
        serde_json::from_str::<Value>(&revert).expect("revert JSON"),
        json!({
            "id": 13,
            "method": "thread/revert",
            "params": {
                "threadId": "thread-1",
                "beforeTurnId": "turn-2"
            }
        })
    );
}

#[test]
fn encodes_a_local_image_before_text() {
    let params = TurnStartParams {
        thread_id: "thread-1".to_owned(),
        input: vec![
            UserInput::LocalImage {
                path: PathBuf::from("/tmp/bot-attachments/image.png"),
                detail: None,
            },
            UserInput::Text {
                text: "Describe this image.".to_owned(),
                text_elements: Vec::new(),
            },
        ],
        model: None,
        effort: None,
        summary: None,
        cwd: None,
        client_user_message_id: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_policy: None,
        collaboration_mode: None,
    };
    let line = encode_request(RequestId::Number(9), "turn/start", &params).expect("request");
    let value: Value = serde_json::from_str(&line).expect("JSON line");
    assert_eq!(
        value["params"]["input"],
        json!([
            {
                "type": "localImage",
                "path": "/tmp/bot-attachments/image.png"
            },
            {
                "type": "text",
                "text": "Describe this image."
            }
        ])
    );
    assert_eq!(value["params"]["approvalPolicy"], Value::Null);
    assert_eq!(value["params"]["approvalsReviewer"], Value::Null);
    assert_eq!(value["params"]["sandboxPolicy"], Value::Null);
}

#[test]
fn decodes_thread_start_and_preserves_unknown_fields() {
    let message =
        decode_line(include_str!("fixtures/thread-start-response.jsonl")).expect("message");
    let IncomingMessage::Response(response) = message else {
        panic!("response");
    };
    let thread: ThreadStartResponse =
        serde_json::from_value(response.outcome.expect("result")).expect("thread start");
    assert_eq!(thread.thread.id, "019a-thread");
    assert_eq!(thread.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(thread.thread.extra["futureThreadField"], true);
    assert_eq!(thread.extra["futureResponseField"], 8);
}
