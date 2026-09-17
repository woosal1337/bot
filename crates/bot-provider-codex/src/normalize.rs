use std::collections::HashMap;

use bot_core::{
    AgentEvent, ApprovalKind, EventId, ProviderEventKind, SessionId, ToolCall, ToolCallKind,
    ToolCallState, TurnId, TurnOutcome, Usage,
};
use serde_json::Value;

use crate::{CodexEvent, ServerNotification, ServerRequest};

pub struct CodexEventNormalizer {
    session_id: SessionId,
    next_event_id: u64,
    tool_calls: HashMap<String, ToolCall>,
}

impl CodexEventNormalizer {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            next_event_id: 1,
            tool_calls: HashMap::new(),
        }
    }

    pub fn normalize(&mut self, event: &CodexEvent) -> Vec<AgentEvent> {
        match event {
            CodexEvent::Notification(notification) => self.normalize_notification(notification),
            CodexEvent::Request(request) => self.normalize_request(request),
            CodexEvent::UnmatchedResponse(response) => vec![self.warning(format!(
                "Codex returned a response for unknown request {}.",
                response.id
            ))],
            CodexEvent::ConnectionClosed(message) => vec![AgentEvent::Error {
                session_id: self.session_id.clone(),
                message: message.clone(),
            }],
        }
    }

    fn normalize_notification(&mut self, notification: &ServerNotification) -> Vec<AgentEvent> {
        let event = match notification.method.as_str() {
            "turn/started" => self.turn_started(&notification.params),
            "turn/completed" => self.turn_completed(&notification.params),
            "item/agentMessage/delta" | "item/plan/delta" => {
                self.text_delta(&notification.params, false)
            }
            "item/reasoning/textDelta" => return Vec::new(),
            "item/reasoning/summaryTextDelta" => self.text_delta(&notification.params, true),
            "item/reasoning/summaryPartAdded" => self.summary_part_added(&notification.params),
            "item/autoApprovalReview/started" => {
                self.auto_review_status(&notification.params, false)
            }
            "item/autoApprovalReview/completed" => {
                self.auto_review_status(&notification.params, true)
            }
            "autoApprovalReview/strictReviewRequired" => self.reasoning_status(
                &notification.params,
                "Codex started a strict safety review.\n",
            ),
            "item/started" => {
                return self.item_lifecycle(
                    &notification.method,
                    &notification.params,
                    ToolCallState::Running,
                );
            }
            "item/completed" => {
                return self.item_lifecycle(
                    &notification.method,
                    &notification.params,
                    completed_tool_state(&notification.params),
                );
            }
            "item/commandExecution/outputDelta" => self.tool_output(&notification.params),
            "item/commandExecution/terminalInteraction" => {
                self.terminal_interaction(&notification.params)
            }
            "item/fileChange/outputDelta" => self.tool_output(&notification.params),
            "item/fileChange/patchUpdated" => self.file_change_updated(&notification.params),
            "item/mcpToolCall/progress" => self.tool_progress(&notification.params),
            "thread/tokenUsage/updated" => self.usage_changed(&notification.params),
            "warning" | "guardianWarning" | "configWarning" | "deprecationNotice" => {
                self.message(&notification.params, false)
            }
            "model/rerouted" => self.model_rerouted(&notification.params),
            "model/safetyBuffering/updated" => self.model_safety_status(&notification.params),
            "model/verification" => {
                self.reasoning_status(&notification.params, "Codex is verifying model access.\n")
            }
            "mcpServer/oauthLogin/completed" => self.mcp_oauth_completed(&notification.params),
            "error" => self.message(&notification.params, true),
            _ => {
                return vec![self.provider_event(
                    ProviderEventKind::Notification,
                    &notification.method,
                    &notification.params,
                )];
            }
        };
        event.into_iter().collect()
    }

    fn normalize_request(&mut self, request: &ServerRequest) -> Vec<AgentEvent> {
        let kind = match request.method.as_str() {
            "item/commandExecution/requestApproval" => ApprovalKind::Command,
            "item/fileChange/requestApproval" => ApprovalKind::FileChange,
            "item/permissions/requestApproval" => ApprovalKind::Permission,
            "item/tool/requestUserInput" => ApprovalKind::Question,
            _ => {
                return vec![self.provider_event(
                    ProviderEventKind::Request,
                    &request.method,
                    &request.params,
                )];
            }
        };
        let Some(turn_id) = parse_turn_id(&request.params) else {
            return vec![self.warning(format!(
                "Codex sent an incomplete {} request.",
                request.method
            ))];
        };
        let title = approval_title(kind, &request.params);
        let Some(id) = self.event_id() else {
            return vec![self.warning("The event ID sequence is exhausted.".to_owned())];
        };
        vec![AgentEvent::ApprovalRequested {
            id,
            session_id: self.session_id.clone(),
            turn_id,
            title,
            kind,
        }]
    }

    fn provider_event(&self, kind: ProviderEventKind, name: &str, params: &Value) -> AgentEvent {
        let payload = match serde_json::to_string(params) {
            Ok(payload) => payload,
            Err(error) => error.to_string(),
        };
        AgentEvent::ProviderEvent {
            session_id: self.session_id.clone(),
            kind,
            name: name.to_owned(),
            payload,
        }
    }

    fn turn_started(&self, params: &Value) -> Option<AgentEvent> {
        Some(AgentEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: parse_nested_turn_id(params)?,
        })
    }

    fn turn_completed(&self, params: &Value) -> Option<AgentEvent> {
        let turn = params.get("turn")?;
        let status = turn.get("status")?.as_str()?;
        let outcome = match status {
            "completed" => TurnOutcome::Completed,
            "failed" => TurnOutcome::Failed,
            "interrupted" => TurnOutcome::Interrupted,
            other => TurnOutcome::Unknown(other.to_owned()),
        };
        Some(AgentEvent::TurnCompleted {
            session_id: self.session_id.clone(),
            turn_id: parse_id(turn.get("id")?)?,
            outcome,
        })
    }

    fn text_delta(&mut self, params: &Value, reasoning: bool) -> Option<AgentEvent> {
        let id = self.event_id()?;
        let turn_id = parse_turn_id(params)?;
        let text = params.get("delta")?.as_str()?.to_owned();
        if reasoning {
            Some(AgentEvent::ReasoningSummaryDelta {
                id,
                session_id: self.session_id.clone(),
                turn_id,
                text,
            })
        } else {
            Some(AgentEvent::AgentTextDelta {
                id,
                session_id: self.session_id.clone(),
                turn_id,
                text,
            })
        }
    }

    fn summary_part_added(&mut self, params: &Value) -> Option<AgentEvent> {
        if params.get("summaryIndex")?.as_u64()? == 0 {
            return None;
        }
        self.reasoning_status(params, "\n\n")
    }

    fn reasoning_status(&mut self, params: &Value, text: &str) -> Option<AgentEvent> {
        Some(AgentEvent::ReasoningSummaryDelta {
            id: self.event_id()?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            text: text.to_owned(),
        })
    }

    fn auto_review_status(&mut self, params: &Value, completed: bool) -> Option<AgentEvent> {
        let subject = auto_review_subject(params.get("action"));
        let text = if completed {
            let status = match params.get("review")?.get("status")?.as_str()? {
                "approved" => "approved",
                "denied" => "denied",
                "timedOut" => "timed out",
                "aborted" => "aborted",
                "inProgress" => "in progress",
                status => status,
            };
            format!("Codex safety review for {subject}: {status}.\n")
        } else {
            format!("Codex is reviewing {subject}.\n")
        };
        self.reasoning_status(params, &text)
    }

    fn model_safety_status(&mut self, params: &Value) -> Option<AgentEvent> {
        if params.get("showBufferingUi").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        self.reasoning_status(params, "Codex is checking safe model routing.\n")
    }

    fn tool_changed(&mut self, params: &Value, state: ToolCallState) -> Option<AgentEvent> {
        let item = params.get("item")?;
        let item_id = item.get("id")?.as_str()?;
        let mut call = tool_call(item, state)?;
        if let Some(previous) = self.tool_calls.get(item_id) {
            if call.detail.is_none() {
                call.detail.clone_from(&previous.detail);
            }
            if call.output.is_none() {
                call.output.clone_from(&previous.output);
            }
        }
        self.tool_calls.insert(item_id.to_owned(), call.clone());
        Some(AgentEvent::ToolCallChanged {
            id: self.item_event_id(item)?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            call,
        })
    }

    fn item_lifecycle(
        &mut self,
        method: &str,
        params: &Value,
        state: ToolCallState,
    ) -> Vec<AgentEvent> {
        let Some(item) = params.get("item") else {
            return vec![self.provider_event(ProviderEventKind::Notification, method, params)];
        };
        if matches!(
            item.get("type").and_then(Value::as_str),
            Some("userMessage" | "agentMessage" | "plan" | "reasoning")
        ) {
            return Vec::new();
        }
        self.tool_changed(params, state).map_or_else(
            || vec![self.provider_event(ProviderEventKind::Notification, method, params)],
            |event| vec![event],
        )
    }

    fn tool_output(&mut self, params: &Value) -> Option<AgentEvent> {
        let item_id = params.get("itemId")?.as_str()?;
        let delta = params.get("delta")?.as_str()?;
        let call = self.tool_calls.get_mut(item_id)?;
        append_tail(&mut call.output, delta, 32_768);
        let call = call.clone();
        Some(AgentEvent::ToolCallChanged {
            id: self.raw_item_event_id(item_id)?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            call,
        })
    }

    fn file_change_updated(&mut self, params: &Value) -> Option<AgentEvent> {
        let item_id = params.get("itemId")?.as_str()?;
        let detail = file_change_detail(params.get("changes")?);
        let call = self.tool_calls.get_mut(item_id)?;
        call.detail = detail;
        let call = call.clone();
        Some(AgentEvent::ToolCallChanged {
            id: self.raw_item_event_id(item_id)?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            call,
        })
    }

    fn terminal_interaction(&mut self, params: &Value) -> Option<AgentEvent> {
        let item_id = params.get("itemId")?.as_str()?;
        let stdin = params.get("stdin")?.as_str()?;
        let call = self.tool_calls.get_mut(item_id)?;
        let text = stdin
            .lines()
            .map(|line| format!("> {line}\n"))
            .collect::<String>();
        append_tail(&mut call.output, &text, 32_768);
        let call = call.clone();
        Some(AgentEvent::ToolCallChanged {
            id: self.raw_item_event_id(item_id)?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            call,
        })
    }

    fn tool_progress(&mut self, params: &Value) -> Option<AgentEvent> {
        let item_id = params.get("itemId")?.as_str()?;
        let message = params.get("message")?.as_str()?.trim();
        let call = self.tool_calls.get_mut(item_id)?;
        if !message.is_empty() {
            call.detail = Some(message.to_owned());
        }
        let call = call.clone();
        Some(AgentEvent::ToolCallChanged {
            id: self.raw_item_event_id(item_id)?,
            session_id: self.session_id.clone(),
            turn_id: parse_turn_id(params)?,
            call,
        })
    }

    fn message(&self, params: &Value, error: bool) -> Option<AgentEvent> {
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| {
                params
                    .get("error")
                    .and_then(|value| value.get("message"))
                    .and_then(Value::as_str)
            })
            .or_else(|| params.get("summary").and_then(Value::as_str))?;
        let detail = params
            .get("details")
            .and_then(Value::as_str)
            .or_else(|| {
                params
                    .get("error")
                    .and_then(|value| value.get("additionalDetails"))
                    .and_then(Value::as_str)
            })
            .filter(|detail| !detail.trim().is_empty());
        let message = match detail {
            Some(detail) => format!("{message}: {detail}"),
            None => message.to_owned(),
        };
        if error && params.get("willRetry").and_then(Value::as_bool) == Some(true) {
            Some(self.warning(format!("{message} Codex will retry.")))
        } else if error {
            Some(AgentEvent::Error {
                session_id: self.session_id.clone(),
                message,
            })
        } else {
            Some(self.warning(message))
        }
    }

    fn model_rerouted(&self, params: &Value) -> Option<AgentEvent> {
        let from = params.get("fromModel")?.as_str()?;
        let to = params.get("toModel")?.as_str()?;
        Some(self.warning(format!("Codex changed the model from {from} to {to}.")))
    }

    fn mcp_oauth_completed(&self, params: &Value) -> Option<AgentEvent> {
        if params.get("success").and_then(Value::as_bool) != Some(false) {
            return None;
        }
        let server = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("MCP server");
        let error = params
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Login failed");
        Some(self.warning(format!("{server}: {error}")))
    }

    fn usage_changed(&self, params: &Value) -> Option<AgentEvent> {
        let usage = params.get("tokenUsage")?;
        let total = usage.get("total")?;
        let last = usage.get("last")?;
        Some(AgentEvent::UsageChanged {
            session_id: self.session_id.clone(),
            usage: Usage {
                input_tokens: total.get("inputTokens")?.as_u64()?,
                cached_input_tokens: total
                    .get("cachedInputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                output_tokens: total.get("outputTokens")?.as_u64()?,
                reasoning_output_tokens: total
                    .get("reasoningOutputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                total_tokens: total
                    .get("totalTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        total
                            .get("inputTokens")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                            .saturating_add(
                                total
                                    .get("outputTokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or_default(),
                            )
                    }),
                context_tokens: last
                    .get("totalTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        last.get("inputTokens")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                            .saturating_add(
                                last.get("outputTokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or_default(),
                            )
                    }),
                context_window: usage.get("modelContextWindow").and_then(Value::as_u64),
            },
        })
    }

    fn event_id(&mut self) -> Option<EventId> {
        let sequence = self.next_event_id;
        self.next_event_id = self.next_event_id.checked_add(1)?;
        EventId::new(format!("{}:codex:{sequence}", self.session_id)).ok()
    }

    fn item_event_id(&self, item: &Value) -> Option<EventId> {
        let item_id = item.get("id")?.as_str()?;
        self.raw_item_event_id(item_id)
    }

    fn raw_item_event_id(&self, item_id: &str) -> Option<EventId> {
        EventId::new(format!("{}:codex:item:{item_id}", self.session_id)).ok()
    }

    fn warning(&self, message: String) -> AgentEvent {
        AgentEvent::Warning {
            session_id: self.session_id.clone(),
            message,
        }
    }
}

fn parse_nested_turn_id(params: &Value) -> Option<TurnId> {
    parse_id(params.get("turn")?.get("id")?)
}

fn parse_turn_id(params: &Value) -> Option<TurnId> {
    parse_id(params.get("turnId")?)
}

fn parse_id(value: &Value) -> Option<TurnId> {
    TurnId::new(value.as_str()?.to_owned()).ok()
}

fn approval_title(kind: ApprovalKind, params: &Value) -> String {
    match kind {
        ApprovalKind::Command => command_text(params).unwrap_or_else(|| "Run command".to_owned()),
        ApprovalKind::FileChange => "Apply file changes".to_owned(),
        ApprovalKind::Permission => "Grant permissions".to_owned(),
        ApprovalKind::Question => "Answer agent questions".to_owned(),
        ApprovalKind::Tool => "Run tool".to_owned(),
    }
}

fn tool_call(item: &Value, fallback_state: ToolCallState) -> Option<ToolCall> {
    let kind = item.get("type")?.as_str()?;
    let state = tool_state(item, fallback_state);
    let duration_ms = item.get("durationMs").and_then(Value::as_u64);
    let output = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let call = match kind {
        "commandExecution" => {
            let command = command_text(item);
            ToolCall {
                kind: ToolCallKind::Command,
                title: command.clone().unwrap_or_else(|| "Run command".to_owned()),
                detail: command.map(|value| format!("$ {value}")),
                output,
                state,
                duration_ms,
            }
        }
        "fileChange" => ToolCall {
            kind: ToolCallKind::FileChange,
            title: file_change_title(item.get("changes")),
            detail: item.get("changes").and_then(file_change_detail),
            output: None,
            state,
            duration_ms,
        },
        "mcpToolCall" => ToolCall {
            kind: ToolCallKind::Mcp,
            title: format!(
                "Call {} · {}",
                item.get("server")?.as_str()?,
                item.get("tool")?.as_str()?
            ),
            detail: mcp_result_detail(item),
            output: None,
            state,
            duration_ms,
        },
        "dynamicToolCall" => ToolCall {
            kind: ToolCallKind::Other,
            title: format!("Run {}", qualified_tool_name(item)?),
            detail: item.get("arguments").and_then(json_detail),
            output: item.get("contentItems").and_then(output_content),
            state,
            duration_ms,
        },
        "functionCallOutput" => ToolCall {
            kind: ToolCallKind::Other,
            title: format!("Receive output from {}", qualified_tool_name(item)?),
            detail: None,
            output: item.get("output").and_then(output_content),
            state,
            duration_ms,
        },
        "hookPrompt" => {
            let fragments = item.get("fragments").and_then(Value::as_array)?;
            ToolCall {
                kind: ToolCallKind::Other,
                title: "Apply hook context".to_owned(),
                detail: Some(match fragments.len() {
                    1 => "1 context fragment".to_owned(),
                    count => format!("{count} context fragments"),
                }),
                output: hook_prompt_output(fragments),
                state,
                duration_ms,
            }
        }
        "webSearch" => ToolCall {
            kind: ToolCallKind::WebSearch,
            title: "Search the web".to_owned(),
            detail: item
                .get("query")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "imageGeneration" => ToolCall {
            kind: ToolCallKind::Image,
            title: "Generate image".to_owned(),
            detail: item
                .get("revisedPrompt")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "imageView" => ToolCall {
            kind: ToolCallKind::Image,
            title: "View image".to_owned(),
            detail: item.get("path").and_then(Value::as_str).map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "collabAgentToolCall" | "subAgentActivity" => ToolCall {
            kind: ToolCallKind::Collaboration,
            title: collaboration_title(item),
            detail: item
                .get("prompt")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "sleep" => ToolCall {
            kind: ToolCallKind::Wait,
            title: "Wait".to_owned(),
            detail: item
                .get("durationMs")
                .and_then(Value::as_u64)
                .map(|duration| format!("{duration} ms")),
            output: None,
            state,
            duration_ms,
        },
        "enteredReviewMode" => ToolCall {
            kind: ToolCallKind::Other,
            title: "Enter review mode".to_owned(),
            detail: item
                .get("review")
                .and_then(Value::as_str)
                .map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "exitedReviewMode" => ToolCall {
            kind: ToolCallKind::Other,
            title: "Finish review".to_owned(),
            detail: item
                .get("review")
                .and_then(Value::as_str)
                .map(str::to_owned),
            output: None,
            state,
            duration_ms,
        },
        "contextCompaction" => ToolCall {
            kind: ToolCallKind::Other,
            title: "Compact context".to_owned(),
            detail: None,
            output: None,
            state,
            duration_ms,
        },
        _ => return None,
    };
    Some(call)
}

fn command_text(value: &Value) -> Option<String> {
    let command = value.get("command")?;
    if let Some(command) = command.as_str() {
        return (!command.trim().is_empty()).then(|| command.to_owned());
    }
    let parts = command.as_array()?;
    let command = parts
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    (!command.trim().is_empty()).then_some(command)
}

pub fn historical_tool_call(item: &Value) -> Option<ToolCall> {
    tool_call(item, ToolCallState::Completed)
}

fn tool_state(item: &Value, fallback: ToolCallState) -> ToolCallState {
    if item.get("success").and_then(Value::as_bool) == Some(false) {
        return ToolCallState::Failed;
    }
    match item.get("status").and_then(Value::as_str) {
        Some("inProgress" | "pending") => ToolCallState::Running,
        Some("completed") => ToolCallState::Completed,
        Some("interrupted" | "cancelled") => ToolCallState::Cancelled,
        Some("failed" | "declined" | "error") => ToolCallState::Failed,
        _ => fallback,
    }
}

fn completed_tool_state(params: &Value) -> ToolCallState {
    let Some(item) = params.get("item") else {
        return ToolCallState::Completed;
    };
    tool_state(item, ToolCallState::Completed)
}

fn qualified_tool_name(item: &Value) -> Option<String> {
    let tool = item.get("tool").or_else(|| item.get("name"))?.as_str()?;
    Some(match item.get("namespace").and_then(Value::as_str) {
        Some(namespace) if !namespace.is_empty() => format!("{namespace} · {tool}"),
        _ => tool.to_owned(),
    })
}

fn json_detail(value: &Value) -> Option<String> {
    serde_json::to_string(value)
        .ok()
        .filter(|value| value != "null")
}

fn output_content(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().filter(|text| !text.is_empty()) {
        return Some(text.to_owned());
    }
    let lines = value
        .as_array()?
        .iter()
        .filter_map(output_content_item)
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn output_content_item(item: &Value) -> Option<String> {
    match item.get("type").and_then(Value::as_str)? {
        "inputText" | "input_text" => item.get("text")?.as_str().map(str::to_owned),
        "inputImage" | "input_image" => Some(media_output(
            "Image output",
            item.get("imageUrl").or_else(|| item.get("image_url")),
        )),
        "inputAudio" | "input_audio" => Some(media_output(
            "Audio output",
            item.get("audioUrl").or_else(|| item.get("audio_url")),
        )),
        "encrypted_content" => Some("Encrypted output".to_owned()),
        _ => None,
    }
}

fn media_output(label: &str, value: Option<&Value>) -> String {
    match value
        .and_then(Value::as_str)
        .filter(|url| !url.starts_with("data:"))
    {
        Some(url) => format!("{label}: {url}"),
        None => label.to_owned(),
    }
}

fn hook_prompt_output(fragments: &[Value]) -> Option<String> {
    let text = fragments
        .iter()
        .filter_map(|fragment| fragment.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn file_change_title(changes: Option<&Value>) -> String {
    let count = changes.and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    match count {
        0 => "Edit files".to_owned(),
        1 => "Edit 1 file".to_owned(),
        count => format!("Edit {count} files"),
    }
}

fn file_change_detail(changes: &Value) -> Option<String> {
    let paths = changes
        .as_array()?
        .iter()
        .filter_map(|change| change.get("path").and_then(Value::as_str))
        .take(3)
        .collect::<Vec<_>>();
    (!paths.is_empty()).then(|| paths.join(" · "))
}

fn mcp_result_detail(item: &Value) -> Option<String> {
    item.get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            item.get("result")
                .and_then(|result| result.get("structuredContent"))
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn collaboration_title(item: &Value) -> String {
    let tool = item
        .get("tool")
        .and_then(Value::as_str)
        .or_else(|| item.get("kind").and_then(Value::as_str))
        .unwrap_or("activity");
    match tool {
        "spawnAgent" => "Spawn agent".to_owned(),
        "sendInput" | "sendMessage" | "followupTask" => "Message agent".to_owned(),
        "resumeAgent" => "Resume agent".to_owned(),
        "wait" => "Wait for agents".to_owned(),
        "closeAgent" => "Close agent".to_owned(),
        "interruptAgent" => "Interrupt agent".to_owned(),
        "listAgents" => "List agents".to_owned(),
        _ => "Agent activity".to_owned(),
    }
}

fn auto_review_subject(action: Option<&Value>) -> &'static str {
    match action
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
    {
        Some("command") => "a command",
        Some("execve") => "a program",
        Some("writeStdin") => "terminal input",
        Some("applyPatch") => "file changes",
        Some("networkAccess") => "network access",
        Some("mcpToolCall") => "an MCP tool",
        Some("requestPermissions") => "permissions",
        _ => "an action",
    }
}

fn append_tail(output: &mut Option<String>, delta: &str, maximum: usize) {
    let output = output.get_or_insert_with(String::new);
    output.push_str(delta);
    if output.len() <= maximum {
        return;
    }
    let mut start = output.len().saturating_sub(maximum);
    while !output.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    output.drain(..start);
}

#[cfg(test)]
mod tests {
    use bot_core::{AgentEvent, ApprovalKind, ToolCallKind, ToolCallState, TurnOutcome};
    use serde_json::json;

    use super::*;
    use crate::{RequestId, ServerNotification, ServerRequest};

    fn normalizer() -> CodexEventNormalizer {
        CodexEventNormalizer::new(SessionId::new("session-1").expect("session ID"))
    }

    #[test]
    fn normalizes_agent_text_and_reasoning_deltas() {
        let mut normalizer = normalizer();
        let text = CodexEvent::Notification(ServerNotification {
            method: "item/agentMessage/delta".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-1", "delta": "Hello"}),
        });
        let reasoning = CodexEvent::Notification(ServerNotification {
            method: "item/reasoning/summaryTextDelta".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-2", "delta": "Checking"}),
        });
        assert!(matches!(
            &normalizer.normalize(&text)[0],
            AgentEvent::AgentTextDelta { text, .. } if text == "Hello"
        ));
        assert!(matches!(
            &normalizer.normalize(&reasoning)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. } if text == "Checking"
        ));
        let plan = CodexEvent::Notification(ServerNotification {
            method: "item/plan/delta".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-3", "delta": "Next step"}),
        });
        assert!(matches!(
            &normalizer.normalize(&plan)[0],
            AgentEvent::AgentTextDelta { text, .. } if text == "Next step"
        ));
        let first_part = CodexEvent::Notification(ServerNotification {
            method: "item/reasoning/summaryPartAdded".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-2", "summaryIndex": 0}),
        });
        let next_part = CodexEvent::Notification(ServerNotification {
            method: "item/reasoning/summaryPartAdded".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-2", "summaryIndex": 1}),
        });
        assert!(normalizer.normalize(&first_part).is_empty());
        assert!(matches!(
            &normalizer.normalize(&next_part)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. } if text == "\n\n"
        ));
        let hidden = CodexEvent::Notification(ServerNotification {
            method: "item/reasoning/textDelta".to_owned(),
            params: json!({
                "turnId": "turn-1",
                "itemId": "item-2",
                "contentIndex": 0,
                "delta": "hidden"
            }),
        });
        assert!(normalizer.normalize(&hidden).is_empty());
    }

    #[test]
    fn normalizes_turn_lifecycle() {
        let mut normalizer = normalizer();
        let started = CodexEvent::Notification(ServerNotification {
            method: "turn/started".to_owned(),
            params: json!({"threadId": "thread-1", "turn": {"id": "turn-1", "status": "inProgress", "items": []}}),
        });
        let completed = CodexEvent::Notification(ServerNotification {
            method: "turn/completed".to_owned(),
            params: json!({"threadId": "thread-1", "turn": {"id": "turn-1", "status": "completed", "items": []}}),
        });
        assert!(matches!(
            &normalizer.normalize(&started)[0],
            AgentEvent::TurnStarted { turn_id, .. } if turn_id.as_str() == "turn-1"
        ));
        assert!(matches!(
            &normalizer.normalize(&completed)[0],
            AgentEvent::TurnCompleted {
                outcome: TurnOutcome::Completed,
                ..
            }
        ));
    }

    #[test]
    fn normalizes_tool_lifecycle() {
        let mut normalizer = normalizer();
        let started = CodexEvent::Notification(ServerNotification {
            method: "item/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {"id": "item-1", "type": "mcpToolCall", "server": "files", "tool": "read", "status": "inProgress"}
            }),
        });
        let completed = CodexEvent::Notification(ServerNotification {
            method: "item/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {"id": "item-1", "type": "mcpToolCall", "server": "files", "tool": "read", "status": "completed"}
            }),
        });
        let started = normalizer.normalize(&started);
        let completed = normalizer.normalize(&completed);
        assert!(matches!(
            &started[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.title == "Call files · read"
                    && call.kind == ToolCallKind::Mcp
                    && call.state == ToolCallState::Running
        ));
        assert!(matches!(
            (&started[0], &completed[0]),
            (
                AgentEvent::ToolCallChanged { id: started_id, .. },
                AgentEvent::ToolCallChanged { id: completed_id, call, .. }
            ) if started_id == completed_id && call.state == ToolCallState::Completed
        ));
    }

    #[test]
    fn accounts_for_every_current_codex_item_type() {
        let streamed = ["userMessage", "agentMessage", "plan", "reasoning"];
        let mut normalizer = normalizer();
        let mut seen = Vec::new();
        for line in include_str!("../tests/fixtures/item-lifecycle.jsonl").lines() {
            let crate::IncomingMessage::Notification(notification) =
                crate::decode_line(line).expect("item notification")
            else {
                panic!("notification");
            };
            let item_type = notification.params["item"]["type"]
                .as_str()
                .expect("item type")
                .to_owned();
            seen.push(item_type.clone());
            let events = normalizer.normalize(&CodexEvent::Notification(notification));
            if streamed.contains(&item_type.as_str()) {
                assert!(events.is_empty(), "{item_type}");
            } else {
                assert!(
                    matches!(events.as_slice(), [AgentEvent::ToolCallChanged { .. }]),
                    "{item_type}: {events:?}"
                );
            }
        }
        assert_eq!(
            seen,
            [
                "userMessage",
                "hookPrompt",
                "agentMessage",
                "functionCallOutput",
                "plan",
                "reasoning",
                "commandExecution",
                "fileChange",
                "mcpToolCall",
                "dynamicToolCall",
                "collabAgentToolCall",
                "subAgentActivity",
                "webSearch",
                "imageView",
                "sleep",
                "imageGeneration",
                "enteredReviewMode",
                "exitedReviewMode",
                "contextCompaction",
            ]
        );
    }

    #[test]
    fn renders_dynamic_hook_and_function_output_items() {
        let mut normalizer = normalizer();
        let events = include_str!("../tests/fixtures/item-lifecycle.jsonl")
            .lines()
            .filter_map(|line| {
                let crate::IncomingMessage::Notification(notification) =
                    crate::decode_line(line).expect("item notification")
                else {
                    panic!("notification");
                };
                normalizer
                    .normalize(&CodexEvent::Notification(notification))
                    .into_iter()
                    .next()
            })
            .filter_map(|event| match event {
                AgentEvent::ToolCallChanged { call, .. } => Some(call),
                _ => None,
            })
            .collect::<Vec<_>>();
        let hook = events
            .iter()
            .find(|call| call.title == "Apply hook context")
            .expect("hook item");
        assert_eq!(hook.detail.as_deref(), Some("1 context fragment"));
        assert_eq!(hook.output.as_deref(), Some("Use the project policy."));
        let function = events
            .iter()
            .find(|call| call.title == "Receive output from records · lookup")
            .expect("function output item");
        assert_eq!(function.output.as_deref(), Some("record alpha"));
        let dynamic = events
            .iter()
            .find(|call| call.title == "Run records · lookup")
            .expect("dynamic tool item");
        assert_eq!(dynamic.detail.as_deref(), Some(r#"{"key":"alpha"}"#));
        assert_eq!(dynamic.output.as_deref(), Some("record alpha"));
    }

    #[test]
    fn preserves_unknown_item_lifecycle_events() {
        let mut normalizer = normalizer();
        let event = CodexEvent::Notification(ServerNotification {
            method: "item/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {"id": "future-1", "type": "futureItem", "future": true}
            }),
        });
        assert!(matches!(
            normalizer.normalize(&event).as_slice(),
            [AgentEvent::ProviderEvent {
                kind: ProviderEventKind::Notification,
                name,
                payload,
                ..
            }] if name == "item/completed" && payload.contains("futureItem")
        ));
    }

    #[test]
    fn streams_command_output_and_completion_metadata() {
        let mut normalizer = normalizer();
        let started = CodexEvent::Notification(ServerNotification {
            method: "item/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1000,
                "item": {
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": "cargo test",
                    "status": "inProgress"
                }
            }),
        });
        let output = CodexEvent::Notification(ServerNotification {
            method: "item/commandExecution/outputDelta".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "delta": "running 60 tests\n"
            }),
        });
        let interaction = CodexEvent::Notification(ServerNotification {
            method: "item/commandExecution/terminalInteraction".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "processId": "process-1",
                "stdin": "yes\n"
            }),
        });
        let completed = CodexEvent::Notification(ServerNotification {
            method: "item/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 2820,
                "item": {
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": "cargo test",
                    "status": "completed",
                    "aggregatedOutput": "running 60 tests\ntest result: ok",
                    "durationMs": 1820
                }
            }),
        });
        let started = normalizer.normalize(&started);
        let output = normalizer.normalize(&output);
        let interaction = normalizer.normalize(&interaction);
        let completed = normalizer.normalize(&completed);
        assert!(matches!(
            &started[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.kind == ToolCallKind::Command
                    && call.title == "cargo test"
                    && call.detail.as_deref() == Some("$ cargo test")
                    && call.state == ToolCallState::Running
        ));
        assert!(matches!(
            &output[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.output.as_deref() == Some("running 60 tests\n")
        ));
        assert!(matches!(
            &interaction[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.output.as_deref() == Some("running 60 tests\n> yes\n")
        ));
        assert!(matches!(
            &completed[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.output.as_deref() == Some("running 60 tests\ntest result: ok")
                    && call.duration_ms == Some(1820)
                    && call.state == ToolCallState::Completed
        ));
    }

    #[test]
    fn normalizes_array_commands_without_losing_arguments() {
        let mut normalizer = normalizer();
        let event = CodexEvent::Notification(ServerNotification {
            method: "item/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": ["git", "status", "--short"],
                    "status": "inProgress"
                }
            }),
        });
        let events = normalizer.normalize(&event);
        assert!(matches!(
            &events[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.title == "git status --short"
                    && call.detail.as_deref() == Some("$ git status --short")
        ));
    }

    #[test]
    fn describes_file_and_web_activity() {
        let mut normalizer = normalizer();
        let file = CodexEvent::Notification(ServerNotification {
            method: "item/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1000,
                "item": {
                    "id": "file-1",
                    "type": "fileChange",
                    "changes": [
                        {"path": "src/app.rs", "kind": {"type": "update"}, "diff": ""},
                        {"path": "src/render.rs", "kind": {"type": "update"}, "diff": ""}
                    ],
                    "status": "inProgress"
                }
            }),
        });
        let web = CodexEvent::Notification(ServerNotification {
            method: "item/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1000,
                "item": {
                    "id": "web-1",
                    "type": "webSearch",
                    "query": "Ratatui animation tick",
                    "status": "inProgress"
                }
            }),
        });
        let file = normalizer.normalize(&file);
        let web = normalizer.normalize(&web);
        assert!(matches!(
            &file[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.title == "Edit 2 files"
                    && call.detail.as_deref() == Some("src/app.rs · src/render.rs")
        ));
        assert!(matches!(
            &web[0],
            AgentEvent::ToolCallChanged { call, .. }
                if call.kind == ToolCallKind::WebSearch
                    && call.detail.as_deref() == Some("Ratatui animation tick")
        ));
    }

    #[test]
    fn normalizes_approval_requests() {
        let mut normalizer = normalizer();
        let event = CodexEvent::Request(ServerRequest {
            id: RequestId::String("approval-1".to_owned()),
            method: "item/commandExecution/requestApproval".to_owned(),
            params: json!({"turnId": "turn-1", "itemId": "item-1", "command": "cargo test"}),
        });
        assert!(matches!(
            &normalizer.normalize(&event)[0],
            AgentEvent::ApprovalRequested { title, kind: ApprovalKind::Command, .. } if title == "cargo test"
        ));
    }

    #[test]
    fn normalizes_usage_totals() {
        let mut normalizer = normalizer();
        let event = CodexEvent::Notification(ServerNotification {
            method: "thread/tokenUsage/updated".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "tokenUsage": {
                    "total": {
                        "totalTokens": 160,
                        "inputTokens": 120,
                        "cachedInputTokens": 20,
                        "outputTokens": 40,
                        "reasoningOutputTokens": 10
                    },
                    "last": {"totalTokens": 16, "inputTokens": 12, "outputTokens": 4},
                    "modelContextWindow": 200000
                }
            }),
        });
        assert!(matches!(
            &normalizer.normalize(&event)[0],
            AgentEvent::UsageChanged { usage, .. }
                if usage.input_tokens == 120
                    && usage.cached_input_tokens == 20
                    && usage.output_tokens == 40
                    && usage.reasoning_output_tokens == 10
                    && usage.total_tokens == 160
                    && usage.context_tokens == 16
                    && usage.context_window == Some(200000)
        ));
    }

    #[test]
    fn preserves_connection_failure_as_an_agent_error() {
        let mut normalizer = normalizer();
        let event = CodexEvent::ConnectionClosed("Codex exited".to_owned());
        assert!(matches!(
            &normalizer.normalize(&event)[0],
            AgentEvent::Error { message, .. } if message == "Codex exited"
        ));
    }

    #[test]
    fn normalizes_structured_errors_and_provider_warnings() {
        let mut normalizer = normalizer();
        let error = CodexEvent::Notification(ServerNotification {
            method: "error".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": false,
                "error": {
                    "message": "Request failed",
                    "additionalDetails": "Try again"
                }
            }),
        });
        let warning = CodexEvent::Notification(ServerNotification {
            method: "configWarning".to_owned(),
            params: json!({
                "summary": "Invalid setting",
                "details": "Using the default"
            }),
        });
        let rerouted = CodexEvent::Notification(ServerNotification {
            method: "model/rerouted".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "fromModel": "gpt-old",
                "toModel": "gpt-new",
                "reason": "availability"
            }),
        });
        assert!(matches!(
            &normalizer.normalize(&error)[0],
            AgentEvent::Error { message, .. } if message == "Request failed: Try again"
        ));
        assert!(matches!(
            &normalizer.normalize(&warning)[0],
            AgentEvent::Warning { message, .. } if message == "Invalid setting: Using the default"
        ));
        assert!(matches!(
            &normalizer.normalize(&rerouted)[0],
            AgentEvent::Warning { message, .. }
                if message == "Codex changed the model from gpt-old to gpt-new."
        ));
        let retry = CodexEvent::Notification(ServerNotification {
            method: "error".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": true,
                "error": {"message": "Connection failed"}
            }),
        });
        assert!(matches!(
            &normalizer.normalize(&retry)[0],
            AgentEvent::Warning { message, .. }
                if message == "Connection failed Codex will retry."
        ));
        let oauth = CodexEvent::Notification(ServerNotification {
            method: "mcpServer/oauthLogin/completed".to_owned(),
            params: json!({
                "name": "demo",
                "success": false,
                "error": "Login expired"
            }),
        });
        assert!(matches!(
            &normalizer.normalize(&oauth)[0],
            AgentEvent::Warning { message, .. } if message == "demo: Login expired"
        ));
    }

    #[test]
    fn normalizes_codex_safety_status_without_hidden_reasoning() {
        let mut normalizer = normalizer();
        let started = CodexEvent::Notification(ServerNotification {
            method: "item/autoApprovalReview/started".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "reviewId": "review-1",
                "action": {"type": "networkAccess"},
                "review": {"status": "inProgress"},
                "startedAtMs": 1000
            }),
        });
        let completed = CodexEvent::Notification(ServerNotification {
            method: "item/autoApprovalReview/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "reviewId": "review-1",
                "action": {"type": "networkAccess"},
                "review": {"status": "approved", "rationale": "private"},
                "startedAtMs": 1000,
                "completedAtMs": 1200,
                "decisionSource": "agent"
            }),
        });
        let buffering = CodexEvent::Notification(ServerNotification {
            method: "model/safetyBuffering/updated".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "model": "gpt-5",
                "reasons": ["private"],
                "showBufferingUi": true,
                "useCases": []
            }),
        });
        let verification = CodexEvent::Notification(ServerNotification {
            method: "model/verification".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "verifications": ["trustedAccessForCyber"]
            }),
        });
        assert!(matches!(
            &normalizer.normalize(&started)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. }
                if text == "Codex is reviewing network access.\n"
        ));
        assert!(matches!(
            &normalizer.normalize(&completed)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. }
                if text == "Codex safety review for network access: approved.\n"
        ));
        assert!(matches!(
            &normalizer.normalize(&buffering)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. }
                if text == "Codex is checking safe model routing.\n"
        ));
        assert!(matches!(
            &normalizer.normalize(&verification)[0],
            AgentEvent::ReasoningSummaryDelta { text, .. }
                if text == "Codex is verifying model access.\n"
        ));
    }

    #[test]
    fn preserves_unknown_provider_events_as_opaque_json() {
        let mut normalizer = normalizer();
        let notification = CodexEvent::Notification(ServerNotification {
            method: "future/notification".to_owned(),
            params: json!({"future": true, "count": 2}),
        });
        let request = CodexEvent::Request(ServerRequest {
            id: RequestId::Number(7),
            method: "future/request".to_owned(),
            params: json!({"choice": "next"}),
        });

        let notification_events = normalizer.normalize(&notification);
        let AgentEvent::ProviderEvent {
            kind: ProviderEventKind::Notification,
            name,
            payload,
            ..
        } = &notification_events[0]
        else {
            panic!("expected provider notification");
        };
        assert_eq!(name, "future/notification");
        assert_eq!(
            serde_json::from_str::<Value>(payload).unwrap(),
            json!({"future": true, "count": 2})
        );
        assert!(matches!(
            &normalizer.normalize(&request)[0],
            AgentEvent::ProviderEvent {
                kind: ProviderEventKind::Request,
                name,
                payload,
                ..
            } if name == "future/request" && payload == r#"{"choice":"next"}"#
        ));
    }
}
