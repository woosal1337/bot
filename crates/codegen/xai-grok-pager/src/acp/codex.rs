// Modified by the Bot project on 2026-09-17: native Codex tools, permissions, extensions, sessions, and turn receipts.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use agent_client_protocol as acp;
use agent_client_protocol::Client as _;
use anyhow::{Context, Result};
use base64::Engine as _;
use bot_core::{
    AgentEvent, PROVIDER_USAGE_UPDATED_METHOD, ProviderId as CoreProviderId, ProviderUsage,
    ProviderUsageUpdate, ToolCallKind, ToolCallState, TurnOutcome, UsageLimit, UsageLimitWindow,
};
use bot_provider::{CommandKind, CommandOwnership};
use bot_provider_codex::{
    ACCOUNT_LOGIN_COMPLETED_NOTIFICATION, ACCOUNT_RATE_LIMITS_UPDATED_NOTIFICATION,
    AccountLoginCompletedNotification, AccountRateLimitsResponse, AccountReadResponse,
    CancelLoginAccountParams, CodexClient, CodexEvent, CodexEventNormalizer, CollaborationMode,
    CollaborationModeKind, CollaborationModeSettings, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalResponse, ConfigValueWriteParams,
    DynamicToolCallOutputContentItem, DynamicToolCallParams, DynamicToolCallResponse,
    DynamicToolSpec, FileChangeApprovalDecision, FileChangeRequestApprovalResponse,
    HookEventName as CodexHookEventName, HookHandlerMetadata as CodexHookHandlerMetadata,
    HookMetadata as CodexHookMetadata, HookSource as CodexHookSource,
    HookTrustStatus as CodexHookTrustStatus, HooksListParams, ITEM_TOOL_CALL_METHOD,
    LoginAccountParams, McpAuthStatus, McpServerConnectionStatus, McpServerElicitationAction,
    McpServerElicitationRequest, McpServerElicitationRequestParams,
    McpServerElicitationRequestResponse, McpServerStatus, McpServerStatusDetail,
    McpServerStatusListParams, MergeStrategy, Model, ModelListParams, PermissionGrantScope,
    PermissionsRequestApprovalResponse, PluginInstallParams, PluginListParams, PluginMarketplace,
    PluginReconcileParams, PluginSource, PluginUninstallParams, ReasoningSummary, RequestId,
    ServerRequest, SkillMetadata as CodexSkillMetadata, SkillScope as CodexSkillScope,
    SkillsConfigWriteParams, SkillsListParams, Thread, ThreadCompactStartParams,
    ThreadDeleteParams, ThreadForkParams, ThreadListParams, ThreadResumeParams, ThreadRevertParams,
    ThreadSearchParams, ThreadSearchResult, ThreadSetNameParams, ThreadStartParams,
    ThreadTurnsListParams, ToolRequestUserInputAnswer, ToolRequestUserInputParams,
    ToolRequestUserInputResponse, Turn, TurnDiffUpdatedNotification, TurnInterruptParams,
    TurnStartParams, TurnSteerParams, UserInput, historical_tool_call, parse_unified_diff,
    split_turn_diff,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use xai_acp_lib::{AcpGatewayReceiver, AcpGatewaySender, acp_channels};
use xai_grok_shell::extensions::notification::{
    SessionNotification as XaiSessionNotification, SessionUpdate as XaiSessionUpdate,
};
use xai_grok_tools::implementations::grok_build::ask_user_question::{
    AskUserQuestionExtRequest, AskUserQuestionExtResponse, AskUserQuestionMode, Question,
    QuestionAnnotation, QuestionMetadata, QuestionOption,
};
use xai_grok_tools::mcp_elicitation::{
    McpElicitCompletePayload, McpElicitExtRequest, McpElicitExtResponse, McpElicitModeFields,
};
use xai_grok_tools::types::output::{BashOutput, ToolOutput};

use super::{AgentEndpoint, AgentLocation, ConnectFlags};
use crate::app::account::{
    ProviderAccountStatus, ProviderCredits, ProviderRateLimit, ProviderRateLimitWindow,
};

const CODEX_CACHED_AUTH_METHOD: &str = "codex.cached";
const CODEX_CHATGPT_AUTH_METHOD: &str = "codex.chatgpt";
const CODEX_DEVICE_AUTH_METHOD: &str = "codex.chatgpt.device";
const CODEX_DYNAMIC_TOOL_CALL_METHOD: &str = "bot/codex/dynamicToolCall";
const CODEX_DYNAMIC_TOOLS_META_KEY: &str = "bot/codexDynamicTools";
const AUTH_CANCEL_METHOD: &str = "x.ai/auth/cancel";
const AUTH_GET_URL_METHOD: &str = "x.ai/auth/get_url";
const AUTH_LOGOUT_METHOD: &str = "x.ai/auth/logout";
const ACCOUNT_STATUS_METHOD: &str = "x.ai/account/read";
const COMPACT_CONVERSATION_METHOD: &str = "x.ai/compact_conversation";
const COMMANDS_LIST_METHOD: &str = "x.ai/commands/list";
const INTERJECT_METHOD: &str = "x.ai/interject";
const SESSION_LIST_METHOD: &str = "x.ai/session/list";
const HOOKS_ACTION_METHOD: &str = "x.ai/hooks/action";
const HOOKS_LIST_METHOD: &str = "x.ai/hooks/list";
const MCP_LIST_METHOD: &str = "x.ai/mcp/list";
const PLUGINS_ACTION_METHOD: &str = "x.ai/plugins/action";
const PLUGINS_LIST_METHOD: &str = "x.ai/plugins/list";
const SESSION_DELETE_METHOD: &str = "x.ai/session/delete";
const SESSION_FORK_METHOD: &str = "x.ai/session/fork";
const SESSION_RENAME_METHOD: &str = "x.ai/session/rename";
const SESSION_SEARCH_METHOD: &str = "x.ai/session/search";
const REWIND_EXECUTE_METHOD: &str = "x.ai/rewind/execute";
const REWIND_POINTS_METHOD: &str = "x.ai/rewind/points";
const SKILLS_LIST_METHOD: &str = "x.ai/skills/list";
const SKILLS_TOGGLE_METHOD: &str = "x.ai/skills/toggle";
const CODEX_NON_BLOCKING_QUESTION_TIMEOUT_MS: u64 = 120_000;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionListRequest {
    cwd: Option<String>,
    limit: Option<u32>,
    query: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexCompactRequest {
    session_id: String,
    user_context: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionRenameRequest {
    session_id: String,
    title: String,
    #[serde(default)]
    reset_to_auto: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionDeleteRequest {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionForkRequest {
    source_session_id: String,
    new_cwd: String,
    #[serde(default)]
    new_session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionSearchRequest {
    query: String,
    cwd: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexInterjectRequest {
    session_id: String,
    text: String,
    interjection_id: String,
    content: Option<Vec<acp::ContentBlock>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexRewindRequest {
    session_id: String,
    target_prompt_index: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexRewindPointsRequest {
    session_id: String,
}

struct CodexRewindPoint {
    turn_id: String,
    prompt_index: usize,
    prompt_preview: String,
    prompt_text: Option<String>,
    created_at: String,
    has_file_changes: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexHooksListRequest {
    session_id: Option<String>,
    cwd: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSkillsListRequest {
    session_id: Option<String>,
    cwd: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexCommandsListRequest {
    session_id: Option<String>,
    cwd: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSkillsToggleRequest {
    session_id: Option<String>,
    cwd: Option<String>,
    name: String,
    enabled: bool,
}

#[derive(Clone)]
struct CodexSession {
    thread_id: String,
    cwd: PathBuf,
    model: String,
    effort: Option<String>,
    model_efforts: HashMap<String, String>,
    permission_mode: CodexPermissionMode,
    turn_state: CodexTurnState,
    mode: CollaborationModeKind,
}

#[derive(Clone)]
struct CodexPendingLogin {
    login_id: String,
    auth_url: String,
    mode: &'static str,
    request_seq: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CodexTurnState {
    Idle,
    Starting {
        cancel_requested: bool,
        stop_background_terminals: bool,
    },
    Active {
        turn_id: String,
    },
}

impl CodexTurnState {
    fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

fn codex_mode_id(mode: CollaborationModeKind) -> &'static str {
    match mode {
        CollaborationModeKind::Default => "default",
        CollaborationModeKind::Plan => "plan",
    }
}

fn codex_mode_state(mode: CollaborationModeKind) -> acp::SessionModeState {
    acp::SessionModeState::new(
        codex_mode_id(mode),
        vec![
            acp::SessionMode::new("default", "Default"),
            acp::SessionMode::new("plan", "Plan"),
        ],
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexPermissionMode {
    Default,
    Ask,
    Auto,
    ReadOnly,
    AlwaysApprove,
}

impl CodexPermissionMode {
    fn from_canonical(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            "read-only" => Some(Self::ReadOnly),
            "always-approve" => Some(Self::AlwaysApprove),
            _ => None,
        }
    }

    fn from_wire(params: &Value) -> Option<Self> {
        match params.get("permission_mode").and_then(Value::as_str) {
            Some(value) => Self::from_canonical(value),
            None if params.get("yolo_mode").and_then(Value::as_bool) == Some(true) => {
                Some(Self::AlwaysApprove)
            }
            None if params.get("auto_mode").and_then(Value::as_bool) == Some(true) => {
                Some(Self::Auto)
            }
            None => Some(Self::Ask),
        }
    }

    fn approval_policy(self) -> Option<String> {
        match self {
            Self::Default => None,
            Self::Ask | Self::Auto | Self::ReadOnly => Some("on-request".to_owned()),
            Self::AlwaysApprove => Some("never".to_owned()),
        }
    }

    fn approvals_reviewer(self) -> Option<String> {
        match self {
            Self::Auto => Some("auto_review".to_owned()),
            Self::Ask | Self::ReadOnly | Self::AlwaysApprove => Some("user".to_owned()),
            Self::Default => None,
        }
    }

    fn sandbox(self) -> Option<String> {
        match self {
            Self::Default => None,
            Self::Ask | Self::Auto => Some("workspace-write".to_owned()),
            Self::ReadOnly => Some("read-only".to_owned()),
            Self::AlwaysApprove => Some("danger-full-access".to_owned()),
        }
    }

    fn sandbox_policy(self) -> Option<Value> {
        match self {
            Self::Default => None,
            Self::Ask | Self::Auto => Some(json!({"type": "workspaceWrite"})),
            Self::ReadOnly => Some(json!({"type": "readOnly"})),
            Self::AlwaysApprove => Some(json!({"type": "dangerFullAccess"})),
        }
    }
}

struct CodexAcpAgent {
    client: Rc<CodexClient>,
    gateway: AcpGatewaySender<acp::AgentSide>,
    account: RefCell<AccountReadResponse>,
    models: Vec<Model>,
    default_model: String,
    sessions: RefCell<HashMap<acp::SessionId, CodexSession>>,
    pending_elicitations: RefCell<HashMap<(String, String), String>>,
    pending_login: RefCell<Option<CodexPendingLogin>>,
    default_permission_mode: RefCell<CodexPermissionMode>,
    provider_usage: RefCell<ProviderUsage>,
}

pub(crate) async fn spawn_codex(
    executable: PathBuf,
    cancel: &CancellationToken,
    flags: &ConnectFlags,
) -> Result<AgentEndpoint> {
    let (client_channel, agent_channel) = acp_channels();
    let worker_cancel = cancel.child_token();
    let thread_cancel = worker_cancel.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let permission_mode = flags
        .default_permission_mode
        .as_deref()
        .and_then(CodexPermissionMode::from_canonical)
        .unwrap_or({
            if flags.default_yolo_mode {
                CodexPermissionMode::AlwaysApprove
            } else if flags.default_auto_mode {
                CodexPermissionMode::Auto
            } else {
                CodexPermissionMode::Ask
            }
        });
    let handle = super::spawn::spawn_runtime_thread("codex-agent-worker", move |rt| {
        let local = tokio::task::LocalSet::new();
        let result = local.block_on(&rt, async move {
            let setup =
                CodexAcpAgent::start(executable, agent_channel.tx.clone(), permission_mode).await;
            let agent = match setup {
                Ok(agent) => {
                    let _ = ready_tx.send(Ok(()));
                    Rc::new(agent)
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    let _ = ready_tx.send(Err(message));
                    return Err(error);
                }
            };
            let receiver =
                AcpGatewayReceiver::new(agent_channel.rx, agent.clone()).with_tracing(true);
            tokio::task::spawn_local(receiver.run());
            thread_cancel.cancelled().await;
            agent.client.close().await.context("failed to stop Codex")?;
            Ok(())
        });
        drop(local);
        super::spawn::shutdown_worker_runtime(rt);
        result
    })
    .await?;
    match tokio::time::timeout(std::time::Duration::from_secs(20), ready_rx).await {
        Ok(Ok(Ok(()))) => Ok(AgentEndpoint {
            tx: client_channel.tx,
            rx: client_channel.rx,
            cancel: worker_cancel,
            location: AgentLocation::Thread(handle),
        }),
        Ok(Ok(Err(message))) => {
            worker_cancel.cancel();
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
            anyhow::bail!(message)
        }
        Ok(Err(_)) => {
            worker_cancel.cancel();
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
            anyhow::bail!("Codex adapter stopped during startup")
        }
        Err(_) => {
            worker_cancel.cancel();
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
            anyhow::bail!("Codex adapter startup timed out after 20 seconds")
        }
    }
}

impl CodexAcpAgent {
    async fn start(
        executable: PathBuf,
        client_tx: xai_acp_lib::AcpClientTx,
        permission_mode: CodexPermissionMode,
    ) -> Result<Self> {
        let client = Rc::new(
            CodexClient::start(&executable)
                .await
                .with_context(|| format!("failed to start {}", executable.display()))?,
        );
        let account = client
            .account()
            .await
            .context("failed to read Codex account")?;
        let models = load_models(&client).await?;
        let default_model = models
            .iter()
            .find(|model| model.is_default)
            .or_else(|| models.first())
            .map(|model| model.model.clone())
            .context("Codex returned no selectable models")?;
        Ok(Self {
            client,
            gateway: AcpGatewaySender::new(client_tx),
            account: RefCell::new(account),
            models,
            default_model,
            sessions: RefCell::new(HashMap::new()),
            pending_elicitations: RefCell::new(HashMap::new()),
            pending_login: RefCell::new(None),
            default_permission_mode: RefCell::new(permission_mode),
            provider_usage: RefCell::new(ProviderUsage::unavailable(CoreProviderId::Codex)),
        })
    }

    fn is_signed_in(&self) -> bool {
        let account = self.account.borrow();
        account.account.is_some() || !account.requires_openai_auth
    }

    fn auth_methods(&self) -> Vec<acp::AuthMethod> {
        let mut methods = Vec::new();
        if self.is_signed_in() {
            methods.push(acp::AuthMethod::Agent(acp::AuthMethodAgent::new(
                CODEX_CACHED_AUTH_METHOD,
                "Current Codex account",
            )));
        }
        methods.extend(codex_interactive_auth_methods(browser_login_available()));
        methods
    }

    async fn cancel_pending_login(&self, request_seq: Option<u64>) -> Result<bool, acp::Error> {
        let pending = {
            let mut current = self.pending_login.borrow_mut();
            if current
                .as_ref()
                .is_some_and(|login| request_seq.is_none() || login.request_seq == request_seq)
            {
                current.take()
            } else {
                None
            }
        };
        let Some(pending) = pending else {
            return Ok(false);
        };
        self.client
            .cancel_account_login(&CancelLoginAccountParams {
                login_id: pending.login_id,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        Ok(true)
    }

    async fn authenticate_interactively(
        &self,
        params: LoginAccountParams,
        request_seq: Option<u64>,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        self.cancel_pending_login(None).await?;
        let mut events = self.client.subscribe();
        let response = self
            .client
            .start_account_login(&params)
            .await
            .map_err(acp::Error::into_internal_error)?;
        let login_id = response
            .login_id
            .ok_or_else(|| acp::Error::internal_error().data("Codex returned no login ID"))?;
        let (auth_url, mode) = match params {
            LoginAccountParams::Chatgpt { .. } => (
                response.auth_url.ok_or_else(|| {
                    acp::Error::internal_error().data("Codex returned no browser login URL")
                })?,
                "command",
            ),
            LoginAccountParams::ChatgptDeviceCode => {
                let verification_url = response.verification_url.ok_or_else(|| {
                    acp::Error::internal_error().data("Codex returned no device verification URL")
                })?;
                let user_code = response.user_code.ok_or_else(|| {
                    acp::Error::internal_error().data("Codex returned no device code")
                })?;
                (device_auth_url(&verification_url, &user_code), "device")
            }
        };
        self.pending_login.replace(Some(CodexPendingLogin {
            login_id: login_id.clone(),
            auth_url: auth_url.clone(),
            mode,
            request_seq,
        }));
        #[cfg(not(test))]
        crate::app::link_opener::open_url(&auth_url);
        loop {
            match events.recv().await {
                Ok(CodexEvent::Notification(notification))
                    if notification.method == ACCOUNT_LOGIN_COMPLETED_NOTIFICATION =>
                {
                    let completed: AccountLoginCompletedNotification =
                        serde_json::from_value(notification.params)
                            .map_err(acp::Error::into_internal_error)?;
                    if completed
                        .login_id
                        .as_deref()
                        .is_some_and(|completed_id| completed_id != login_id)
                    {
                        continue;
                    }
                    if self
                        .pending_login
                        .borrow()
                        .as_ref()
                        .is_some_and(|pending| pending.login_id == login_id)
                    {
                        self.pending_login.take();
                    }
                    if !completed.success {
                        return Err(acp::Error::auth_required().data(
                            completed
                                .error
                                .unwrap_or_else(|| "Codex login did not complete".to_owned()),
                        ));
                    }
                    let account = self
                        .client
                        .account()
                        .await
                        .map_err(acp::Error::into_internal_error)?;
                    self.account.replace(account);
                    self.refresh_all_provider_usage().await;
                    return Ok(acp::AuthenticateResponse::new());
                }
                Ok(CodexEvent::ConnectionClosed(message)) => {
                    return Err(acp::Error::internal_error().data(message));
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(
                        acp::Error::internal_error().data("Codex closed before login completed")
                    );
                }
            }
        }
    }

    async fn account_status(&self) -> Result<ProviderAccountStatus, acp::Error> {
        let account = self
            .client
            .account()
            .await
            .map_err(acp::Error::into_internal_error)?;
        let signed_in = account.account.is_some() || !account.requires_openai_auth;
        let auth_method = account
            .account
            .as_ref()
            .map(|current| codex_display_name(&current.kind));
        let email = account
            .account
            .as_ref()
            .and_then(|current| current.email.clone());
        let plan = account
            .account
            .as_ref()
            .and_then(|current| current.plan_type.as_deref())
            .map(codex_display_name);
        self.account.replace(account);
        let (rate_limits, ordinary_usage_allowed, reset_credits, rate_limits_error) = if signed_in {
            match self.client.account_rate_limits().await {
                Ok(response) => {
                    let snapshots = match response.rate_limits_by_limit_id {
                        Some(items) if !items.is_empty() => items.into_values().collect(),
                        _ => vec![response.rate_limits],
                    };
                    (
                        snapshots.into_iter().map(map_rate_limit).collect(),
                        response.ordinary_usage_allowed,
                        response
                            .rate_limit_reset_credits
                            .map(|credits| credits.available_count),
                        None,
                    )
                }
                Err(error) => {
                    tracing::debug!(error = %error, "Codex rate-limit read failed");
                    (
                        Vec::new(),
                        None,
                        None,
                        Some("Codex did not return rate-limit data.".to_owned()),
                    )
                }
            }
        } else {
            (Vec::new(), None, None, None)
        };
        Ok(ProviderAccountStatus {
            provider: "Codex".to_owned(),
            signed_in,
            auth_method,
            email,
            plan,
            ordinary_usage_allowed,
            reset_credits,
            rate_limits,
            rate_limits_error,
        })
    }

    fn model_state(&self, current_model: &str, effort: Option<&str>) -> acp::SessionModelState {
        let available = self
            .models
            .iter()
            .map(|model| {
                model_info(
                    model,
                    (model.model == current_model).then_some(effort).flatten(),
                )
            })
            .collect();
        acp::SessionModelState::new(current_model.to_owned(), available)
    }

    async fn notify(&self, session_id: &acp::SessionId, update: acp::SessionUpdate) {
        let _ = self
            .gateway
            .session_notification(acp::SessionNotification::new(session_id.clone(), update))
            .await;
    }

    async fn notify_provider_usage(&self, session_id: &acp::SessionId) {
        let update = ProviderUsageUpdate {
            session_id: session_id.0.to_string(),
            usage: self.provider_usage.borrow().clone(),
        };
        let Ok(raw) = serde_json::value::to_raw_value(&update) else {
            return;
        };
        let _ = self
            .gateway
            .ext_notification(acp::ExtNotification::new(
                PROVIDER_USAGE_UPDATED_METHOD,
                raw.into(),
            ))
            .await;
    }

    async fn refresh_provider_usage(&self, session_id: &acp::SessionId) {
        let (rate_limits, account_usage) = tokio::join!(
            self.client.account_rate_limits(),
            self.client.account_usage()
        );
        {
            let mut usage = self.provider_usage.borrow_mut();
            match rate_limits {
                Ok(response) => usage.limits = map_usage_limits(response),
                Err(error) => tracing::debug!(error = %error, "Codex rate-limit refresh failed"),
            }
            match account_usage {
                Ok(response) => {
                    usage.lifetime_tokens =
                        response.summary.and_then(|summary| summary.lifetime_tokens);
                }
                Err(error) => tracing::debug!(error = %error, "Codex account-usage refresh failed"),
            }
        }
        self.notify_provider_usage(session_id).await;
    }

    async fn refresh_all_provider_usage(&self) {
        let session_ids = self.sessions.borrow().keys().cloned().collect::<Vec<_>>();
        let Some((first, rest)) = session_ids.split_first() else {
            return;
        };
        self.refresh_provider_usage(first).await;
        for session_id in rest {
            self.notify_provider_usage(session_id).await;
        }
    }

    async fn clear_provider_usage(&self) {
        self.provider_usage
            .replace(ProviderUsage::unavailable(CoreProviderId::Codex));
        let session_ids = self.sessions.borrow().keys().cloned().collect::<Vec<_>>();
        for session_id in &session_ids {
            self.notify_provider_usage(session_id).await;
        }
    }

    async fn stream_turn(
        &self,
        session_id: &acp::SessionId,
        thread_id: &str,
        turn_id: &str,
        mut events: broadcast::Receiver<CodexEvent>,
    ) -> Result<acp::StopReason, acp::Error> {
        let core_session_id = bot_core::SessionId::new(session_id.0.to_string())
            .map_err(acp::Error::into_internal_error)?;
        let mut normalizer = CodexEventNormalizer::new(core_session_id);
        let mut emitted_tools = HashSet::new();
        let mut item_file_paths = HashSet::new();
        let mut pending_turn_diff = None;
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    self.notify(
                        session_id,
                        text_update(
                            acp::SessionUpdate::AgentThoughtChunk,
                            format!("Codex stream skipped {count} buffered events."),
                        ),
                    )
                    .await;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(acp_error("Codex app-server closed its event stream"));
                }
            };
            if !event_matches(&event, thread_id, turn_id) {
                continue;
            }
            if let CodexEvent::Request(request) = &event {
                self.handle_request(session_id, request).await?;
                continue;
            }
            if let CodexEvent::ConnectionClosed(message) = &event {
                return Err(acp_error(message));
            }
            if let CodexEvent::Notification(notification) = &event {
                self.complete_mcp_oauth(session_id, notification).await?;
            }
            if let CodexEvent::Notification(notification) = &event
                && notification.method == ACCOUNT_RATE_LIMITS_UPDATED_NOTIFICATION
            {
                match serde_json::from_value::<AccountRateLimitsResponse>(
                    notification.params.clone(),
                ) {
                    Ok(response) => {
                        self.provider_usage.borrow_mut().limits = map_usage_limits(response);
                        self.notify_provider_usage(session_id).await;
                    }
                    Err(error) => {
                        tracing::debug!(error = %error, "Codex sent invalid rate-limit data");
                    }
                }
                continue;
            }
            if let CodexEvent::Notification(notification) = &event {
                if notification.method == "turn/diff/updated" {
                    match serde_json::from_value::<TurnDiffUpdatedNotification>(
                        notification.params.clone(),
                    ) {
                        Ok(update) => pending_turn_diff = Some(update),
                        Err(error) => {
                            self.notify(
                                session_id,
                                text_update(
                                    acp::SessionUpdate::AgentThoughtChunk,
                                    format!("Warning: Codex sent an invalid turn diff: {error}"),
                                ),
                            )
                            .await;
                        }
                    }
                    continue;
                }
                let file_changes = protocol_file_change_calls(session_id, notification);
                if !file_changes.is_empty() {
                    for (tool_id, call) in file_changes {
                        item_file_paths
                            .extend(call.locations.iter().map(|location| location.path.clone()));
                        let update = if emitted_tools.insert(tool_id) {
                            acp::SessionUpdate::ToolCall(call)
                        } else {
                            acp::SessionUpdate::ToolCallUpdate(call.into())
                        };
                        self.notify(session_id, update).await;
                    }
                    continue;
                }
            }
            if let CodexEvent::Notification(notification) = &event
                && let Some((tool_id, call)) =
                    protocol_command_completion_call(session_id, notification)
            {
                let update = if emitted_tools.insert(tool_id) {
                    acp::SessionUpdate::ToolCall(call)
                } else {
                    acp::SessionUpdate::ToolCallUpdate(call.into())
                };
                self.notify(session_id, update).await;
                continue;
            }
            if let CodexEvent::Notification(notification) = &event
                && let Some(update) = protocol_session_update(notification)
            {
                self.notify(session_id, update).await;
                continue;
            }
            for normalized in normalizer.normalize(&event) {
                match normalized {
                    AgentEvent::AgentTextDelta { text, .. } => {
                        self.notify(
                            session_id,
                            text_update(acp::SessionUpdate::AgentMessageChunk, text),
                        )
                        .await;
                    }
                    AgentEvent::ReasoningSummaryDelta { text, .. } => {
                        self.notify(
                            session_id,
                            text_update(acp::SessionUpdate::AgentThoughtChunk, text),
                        )
                        .await;
                    }
                    AgentEvent::ToolCallChanged { id, call, .. } => {
                        let tool_id = id.as_str().to_owned();
                        let update = if emitted_tools.insert(tool_id.clone()) {
                            acp::SessionUpdate::ToolCall(tool_call(tool_id, &call))
                        } else {
                            acp::SessionUpdate::ToolCallUpdate(tool_call_update(tool_id, &call))
                        };
                        self.notify(session_id, update).await;
                    }
                    AgentEvent::UsageChanged { usage, .. } => {
                        if let Some(size) = usage.context_window {
                            self.notify(
                                session_id,
                                acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(
                                    usage.input_tokens.saturating_add(usage.output_tokens),
                                    size,
                                )),
                            )
                            .await;
                        }
                    }
                    AgentEvent::Warning { message, .. } => {
                        self.notify(
                            session_id,
                            text_update(
                                acp::SessionUpdate::AgentThoughtChunk,
                                format!("Warning: {message}"),
                            ),
                        )
                        .await;
                    }
                    AgentEvent::Error { message, .. } => return Err(acp_error(&message)),
                    AgentEvent::TurnCompleted { outcome, .. } => {
                        if let Some(turn_diff) = pending_turn_diff.take() {
                            for (_, call) in
                                turn_diff_calls(session_id, &turn_diff, &item_file_paths)
                            {
                                self.notify(session_id, acp::SessionUpdate::ToolCall(call))
                                    .await;
                            }
                        }
                        return match outcome {
                            TurnOutcome::Completed => Ok(acp::StopReason::EndTurn),
                            TurnOutcome::Interrupted => Ok(acp::StopReason::Cancelled),
                            TurnOutcome::Failed => Err(acp_error("Codex turn failed")),
                            TurnOutcome::Unknown(status) => {
                                Err(acp_error(format!("Codex turn ended with status {status}")))
                            }
                        };
                    }
                    AgentEvent::TurnStarted { .. }
                    | AgentEvent::UserMessage { .. }
                    | AgentEvent::ApprovalRequested { .. }
                    | AgentEvent::ApprovalResolved { .. }
                    | AgentEvent::ProviderEvent { .. } => {}
                }
            }
        }
    }

    async fn handle_request(
        &self,
        session_id: &acp::SessionId,
        request: &ServerRequest,
    ) -> Result<(), acp::Error> {
        match request.method.as_str() {
            "item/commandExecution/requestApproval" => {
                let decision = self.request_permission(session_id, request).await?;
                self.client
                    .respond(
                        request.id.clone(),
                        &CommandExecutionRequestApprovalResponse {
                            decision: match decision {
                                PermissionDecision::AllowOnce => {
                                    CommandExecutionApprovalDecision::Accept
                                }
                                PermissionDecision::AllowAlways => {
                                    CommandExecutionApprovalDecision::AcceptForSession
                                }
                                PermissionDecision::Reject => {
                                    CommandExecutionApprovalDecision::Decline
                                }
                                PermissionDecision::Cancel => {
                                    CommandExecutionApprovalDecision::Cancel
                                }
                            },
                        },
                    )
                    .await
                    .map_err(acp::Error::into_internal_error)
            }
            "item/fileChange/requestApproval" => {
                let decision = self.request_permission(session_id, request).await?;
                self.client
                    .respond(
                        request.id.clone(),
                        &FileChangeRequestApprovalResponse {
                            decision: match decision {
                                PermissionDecision::AllowOnce => FileChangeApprovalDecision::Accept,
                                PermissionDecision::AllowAlways => {
                                    FileChangeApprovalDecision::AcceptForSession
                                }
                                PermissionDecision::Reject => FileChangeApprovalDecision::Decline,
                                PermissionDecision::Cancel => FileChangeApprovalDecision::Cancel,
                            },
                        },
                    )
                    .await
                    .map_err(acp::Error::into_internal_error)
            }
            "item/permissions/requestApproval" => {
                let decision = self.request_permission(session_id, request).await?;
                let permissions = if matches!(
                    decision,
                    PermissionDecision::AllowOnce | PermissionDecision::AllowAlways
                ) {
                    request
                        .params
                        .get("permissions")
                        .cloned()
                        .unwrap_or_else(|| json!({}))
                } else {
                    json!({})
                };
                self.client
                    .respond(
                        request.id.clone(),
                        &PermissionsRequestApprovalResponse {
                            permissions,
                            scope: if matches!(decision, PermissionDecision::AllowAlways) {
                                PermissionGrantScope::Session
                            } else {
                                PermissionGrantScope::Turn
                            },
                            strict_auto_review: None,
                        },
                    )
                    .await
                    .map_err(acp::Error::into_internal_error)
            }
            "item/tool/requestUserInput" => self.request_user_input(session_id, request).await,
            ITEM_TOOL_CALL_METHOD => self.request_dynamic_tool(request).await,
            "execCommandApproval" | "applyPatchApproval" => {
                let decision = self.request_permission(session_id, request).await?;
                self.client
                    .respond(request.id.clone(), &legacy_approval_response(decision))
                    .await
                    .map_err(acp::Error::into_internal_error)
            }
            "currentTime/read" => self
                .client
                .respond(
                    request.id.clone(),
                    &json!({ "currentTimeAt": Utc::now().timestamp() }),
                )
                .await
                .map_err(acp::Error::into_internal_error),
            "mcpServer/elicitation/request" => {
                self.request_mcp_elicitation(session_id, request).await
            }
            _ => self
                .client
                .respond_error(
                    request.id.clone(),
                    -32601,
                    format!("Unsupported Codex request method: {}", request.method),
                    None,
                )
                .await
                .map_err(acp::Error::into_internal_error),
        }
    }

    async fn request_permission(
        &self,
        session_id: &acp::SessionId,
        request: &ServerRequest,
    ) -> Result<PermissionDecision, acp::Error> {
        let item_id = request
            .params
            .get("itemId")
            .and_then(Value::as_str)
            .or_else(|| request.params.get("callId").and_then(Value::as_str))
            .unwrap_or("codex-approval");
        let title = request
            .params
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                request
                    .params
                    .get("command")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
            })
            .or_else(|| {
                request
                    .params
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Codex requests permission".to_owned());
        let kind =
            if request.method.contains("fileChange") || request.method == "applyPatchApproval" {
                acp::ToolKind::Edit
            } else {
                acp::ToolKind::Execute
            };
        let update = acp::ToolCallUpdate::new(
            codex_tool_id(session_id, item_id),
            acp::ToolCallUpdateFields::new()
                .title(title)
                .kind(kind)
                .status(acp::ToolCallStatus::Pending),
        );
        let options = vec![
            acp::PermissionOption::new(
                "allow_once",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                "allow_always",
                "Allow for this session",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new("reject", "Reject", acp::PermissionOptionKind::RejectOnce),
        ];
        let response = self
            .gateway
            .request_permission(acp::RequestPermissionRequest::new(
                session_id.clone(),
                update,
                options,
            ))
            .await?;
        Ok(match response.outcome {
            acp::RequestPermissionOutcome::Selected(selected) => {
                match selected.option_id.0.as_ref() {
                    "allow_once" => PermissionDecision::AllowOnce,
                    "allow_always" => PermissionDecision::AllowAlways,
                    _ => PermissionDecision::Reject,
                }
            }
            acp::RequestPermissionOutcome::Cancelled => PermissionDecision::Cancel,
            _ => PermissionDecision::Cancel,
        })
    }

    async fn request_user_input(
        &self,
        session_id: &acp::SessionId,
        request: &ServerRequest,
    ) -> Result<(), acp::Error> {
        let params: ToolRequestUserInputParams = serde_json::from_value(request.params.clone())
            .map_err(acp::Error::into_internal_error)?;
        let item_id = params.item_id.clone();
        let mut answer_ids = HashMap::new();
        let mut question_metadata = HashMap::new();
        let questions = params
            .questions
            .into_iter()
            .map(|value| {
                let display = if value.header.is_empty() || value.header == value.question {
                    value.question.clone()
                } else {
                    format!("{}: {}", value.header, value.question)
                };
                let id = value.id;
                let options = value.options.unwrap_or_default();
                question_metadata.insert(
                    id.clone(),
                    QuestionMetadata {
                        allow_freeform: options.is_empty() || value.is_other,
                        secret: value.is_secret,
                    },
                );
                answer_ids.insert(display.clone(), id.clone());
                let options = options
                    .into_iter()
                    .map(|option| QuestionOption {
                        label: option.label,
                        description: option.description,
                        preview: None,
                        id: None,
                    })
                    .collect();
                Question {
                    question: display,
                    options,
                    multi_select: Some(false),
                    id: Some(id),
                }
            })
            .collect();
        let payload = AskUserQuestionExtRequest {
            session_id: session_id.0.to_string(),
            tool_call_id: item_id,
            questions,
            mode: AskUserQuestionMode::Default,
            question_metadata,
            auto_resolution_ms: (!params.is_blocking)
                .then_some(CODEX_NON_BLOCKING_QUESTION_TIMEOUT_MS),
        };
        let raw =
            serde_json::value::to_raw_value(&payload).map_err(acp::Error::into_internal_error)?;
        let response = self
            .gateway
            .ext_method(acp::ExtRequest::new("x.ai/ask_user_question", raw.into()))
            .await?;
        let response: AskUserQuestionExtResponse =
            serde_json::from_str(response.0.get()).map_err(acp::Error::into_internal_error)?;
        let answers = codex_question_answers(response, &answer_ids);
        self.client
            .respond(
                request.id.clone(),
                &ToolRequestUserInputResponse { answers },
            )
            .await
            .map_err(acp::Error::into_internal_error)
    }

    async fn request_mcp_elicitation(
        &self,
        session_id: &acp::SessionId,
        request: &ServerRequest,
    ) -> Result<(), acp::Error> {
        let payload = mcp_elicitation_payload(session_id, request)
            .map_err(acp::Error::into_internal_error)?;
        let server_name = payload.server_name.clone();
        let pending_url = match &payload.mode {
            McpElicitModeFields::Url { elicitation_id, .. } => Some(elicitation_id.clone()),
            McpElicitModeFields::Form { .. } => None,
        };
        if let Some(elicitation_id) = pending_url.as_ref() {
            self.pending_elicitations.borrow_mut().insert(
                (session_id.0.to_string(), server_name.clone()),
                elicitation_id.clone(),
            );
        }
        let raw =
            serde_json::value::to_raw_value(&payload).map_err(acp::Error::into_internal_error)?;
        let response = self
            .gateway
            .ext_method(acp::ExtRequest::new("x.ai/mcp/elicit", raw.into()))
            .await;
        let response = match response {
            Ok(response) => serde_json::from_str::<McpElicitExtResponse>(response.0.get())
                .map_err(acp::Error::into_internal_error)?,
            Err(error) => {
                self.pending_elicitations
                    .borrow_mut()
                    .remove(&(session_id.0.to_string(), server_name));
                return Err(error);
            }
        };
        if !matches!(response, McpElicitExtResponse::Accept { .. }) {
            self.pending_elicitations
                .borrow_mut()
                .remove(&(session_id.0.to_string(), server_name));
        }
        let result = mcp_elicitation_result(response);
        self.client
            .respond(request.id.clone(), &result)
            .await
            .map_err(acp::Error::into_internal_error)
    }

    async fn request_dynamic_tool(&self, request: &ServerRequest) -> Result<(), acp::Error> {
        let params = match serde_json::from_value::<DynamicToolCallParams>(request.params.clone()) {
            Ok(params) => params,
            Err(error) => {
                return self
                    .client
                    .respond_error(
                        request.id.clone(),
                        -32602,
                        format!("Invalid Codex dynamic tool call: {error}"),
                        None,
                    )
                    .await
                    .map_err(acp::Error::into_internal_error);
            }
        };
        let raw =
            serde_json::value::to_raw_value(&params).map_err(acp::Error::into_internal_error)?;
        let response = match self
            .gateway
            .ext_method(acp::ExtRequest::new(
                CODEX_DYNAMIC_TOOL_CALL_METHOD,
                raw.into(),
            ))
            .await
        {
            Ok(response) => match serde_json::from_str::<DynamicToolCallResponse>(response.0.get())
            {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(error = %error, "Invalid dynamic tool response from ACP client");
                    dynamic_tool_failure("The Bot client returned an invalid tool response.")
                }
            },
            Err(error) => {
                tracing::warn!(error = ?error, "Dynamic tool is unavailable in the ACP client");
                dynamic_tool_failure("This dynamic tool is not available in the Bot client.")
            }
        };
        self.client
            .respond(request.id.clone(), &response)
            .await
            .map_err(acp::Error::into_internal_error)
    }

    async fn complete_mcp_oauth(
        &self,
        session_id: &acp::SessionId,
        notification: &bot_provider_codex::ServerNotification,
    ) -> Result<(), acp::Error> {
        if notification.method != "mcpServer/oauthLogin/completed" {
            return Ok(());
        }
        let Some(server_name) = notification.params.get("name").and_then(Value::as_str) else {
            return Ok(());
        };
        let key = (session_id.0.to_string(), server_name.to_owned());
        let Some(elicitation_id) = self.pending_elicitations.borrow_mut().remove(&key) else {
            return Ok(());
        };
        let payload = McpElicitCompletePayload {
            session_id: session_id.0.to_string(),
            elicitation_id,
            server_name: Some(server_name.to_owned()),
        };
        let raw =
            serde_json::value::to_raw_value(&payload).map_err(acp::Error::into_internal_error)?;
        self.gateway
            .ext_notification(acp::ExtNotification::new(
                "x.ai/mcp/elicit_complete",
                raw.into(),
            ))
            .await
    }

    async fn session_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSessionListRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let response = self
            .client
            .list_threads(&ThreadListParams {
                cursor: None,
                limit: request.limit,
                cwd: request.cwd,
                archived: Some(false),
                search_term: request.query,
                sort_key: Some("updated_at".to_owned()),
                sort_direction: Some("desc".to_owned()),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let value = codex_session_list_payload(response.data);
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn compact_conversation(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexCompactRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        if request
            .user_context
            .as_deref()
            .is_some_and(|context| !context.trim().is_empty())
        {
            return Err(acp::Error::invalid_params()
                .data("Codex does not support custom compaction instructions"));
        }
        let session = self.ext_session_id(&request.session_id)?;
        self.client
            .compact_thread(&ThreadCompactStartParams {
                thread_id: session.thread_id,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let raw =
            serde_json::value::to_raw_value(&json!({})).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn rename_session(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSessionRenameRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        if request.reset_to_auto {
            return Err(
                acp::Error::invalid_params().data("Codex does not support automatic title reset")
            );
        }
        let title = request.title.trim();
        if title.is_empty() {
            return Err(acp::Error::invalid_params().data("Session title must not be blank"));
        }
        self.client
            .set_thread_name(&ThreadSetNameParams {
                thread_id: request.session_id,
                name: title.to_owned(),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let raw = serde_json::value::to_raw_value(&json!({"success": true}))
            .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn delete_session(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSessionDeleteRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        self.client
            .delete_thread(&ThreadDeleteParams {
                thread_id: request.session_id.clone(),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        self.sessions
            .borrow_mut()
            .remove(&acp::SessionId::new(request.session_id));
        let raw = serde_json::value::to_raw_value(&json!({"success": true}))
            .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn fork_session(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSessionForkRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        if request.new_session_id.is_some() {
            return Err(acp::Error::invalid_params()
                .data("Codex does not support client-selected thread IDs"));
        }
        let response = self
            .client
            .fork_thread(&ThreadForkParams {
                thread_id: request.source_session_id,
                cwd: Some(request.new_cwd),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let raw = serde_json::value::to_raw_value(&json!({
            "newSessionId": response.thread.id,
        }))
        .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn search_sessions(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSessionSearchRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let query = request.query.trim();
        if query.is_empty() {
            return Err(acp::Error::invalid_params().data("Session search must not be blank"));
        }
        let response = self
            .client
            .search_threads(&ThreadSearchParams {
                cursor: None,
                limit: Some(request.limit.unwrap_or(20).clamp(1, 100)),
                sort_key: Some("updated_at".to_owned()),
                sort_direction: Some("desc".to_owned()),
                source_kinds: None,
                archived: Some(false),
                search_term: query.to_owned(),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let matches = response
            .data
            .into_iter()
            .filter(|matched| {
                request.cwd.as_ref().is_none_or(|cwd| {
                    matched
                        .thread
                        .cwd
                        .as_deref()
                        .is_some_and(|thread_cwd| thread_cwd == Path::new(cwd))
                })
            })
            .collect();
        let results = codex_session_search_payload(matches);
        let raw =
            serde_json::value::to_raw_value(&results).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn thread_turns(&self, thread_id: &str) -> Result<Vec<Turn>, acp::Error> {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        let mut turns = Vec::new();
        loop {
            let response = self
                .client
                .list_thread_turns(&ThreadTurnsListParams {
                    thread_id: thread_id.to_owned(),
                    cursor: cursor.clone(),
                    limit: Some(100),
                    sort_direction: Some("asc".to_owned()),
                    items_view: Some("full".to_owned()),
                })
                .await
                .map_err(acp::Error::into_internal_error)?;
            turns.extend(response.data);
            match response.next_cursor {
                Some(next) if seen_cursors.insert(next.clone()) => cursor = Some(next),
                Some(_) => {
                    return Err(
                        acp::Error::internal_error().data("Codex repeated a turn-history cursor")
                    );
                }
                None => return Ok(turns),
            }
        }
    }

    async fn rewind_points(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexRewindPointsRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session_id(&request.session_id)?;
        let turns = self.thread_turns(&session.thread_id).await?;
        let points = codex_rewind_points(&turns)
            .into_iter()
            .map(|point| {
                json!({
                    "promptIndex": point.prompt_index,
                    "createdAt": point.created_at,
                    "numFileSnapshots": 0,
                    "promptPreview": point.prompt_preview,
                    "hasFileChanges": point.has_file_changes,
                })
            })
            .collect::<Vec<_>>();
        let raw = serde_json::value::to_raw_value(&json!({"rewindPoints": points}))
            .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn rewind_execute(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexRewindRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session_id(&request.session_id)?;
        if session.turn_state.is_busy() {
            return Err(acp::Error::invalid_params().data("Stop the Codex turn before you rewind"));
        }
        let turns = self.thread_turns(&session.thread_id).await?;
        let point = codex_rewind_points(&turns)
            .into_iter()
            .find(|point| point.prompt_index == request.target_prompt_index)
            .ok_or_else(|| {
                acp::Error::invalid_params()
                    .data("The selected Codex rewind point is not available")
            })?;
        self.client
            .revert_thread(&ThreadRevertParams {
                thread_id: session.thread_id,
                before_turn_id: point.turn_id,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let raw = serde_json::value::to_raw_value(&json!({
            "success": true,
            "targetPromptIndex": point.prompt_index,
            "revertedFiles": [],
            "cleanFiles": [],
            "conflicts": [],
            "mode": "conversation_only",
            "promptText": point.prompt_text,
        }))
        .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn interject(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexInterjectRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session_id(&request.session_id)?;
        let turn_id = match session.turn_state {
            CodexTurnState::Active { turn_id } => turn_id,
            CodexTurnState::Starting { .. } => {
                return Err(acp::Error::invalid_params()
                    .data("Wait for the Codex turn to start before you interject"));
            }
            CodexTurnState::Idle => {
                return Err(
                    acp::Error::invalid_params().data("Start a Codex turn before you interject")
                );
            }
        };
        let image_dir = tempfile::tempdir().map_err(acp::Error::into_internal_error)?;
        let blocks = request.content.unwrap_or_else(|| {
            vec![acp::ContentBlock::Text(acp::TextContent::new(
                request.text.clone(),
            ))]
        });
        let input = prompt_input(&blocks, image_dir.path())?;
        let response = self
            .client
            .steer_turn(&TurnSteerParams {
                thread_id: session.thread_id,
                expected_turn_id: turn_id,
                input,
                client_user_message_id: Some(request.interjection_id.clone()),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let notification = json!({
            "sessionId": request.session_id,
            "text": request.text,
            "interjectionId": request.interjection_id,
        });
        let notification = serde_json::value::to_raw_value(&notification)
            .map_err(acp::Error::into_internal_error)?;
        self.gateway
            .ext_notification(acp::ExtNotification::new(
                "x.ai/session/interjection",
                notification.into(),
            ))
            .await?;
        let raw = serde_json::value::to_raw_value(&json!({"turnId": response.turn_id}))
            .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn mcp_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let params: Value = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session(&params)?;
        if params.get("cache").and_then(Value::as_bool) == Some(false) {
            self.client
                .reload_mcp_servers()
                .await
                .map_err(acp::Error::into_internal_error)?;
        }
        let mut cursor = None;
        let mut servers = Vec::new();
        loop {
            let response = self
                .client
                .mcp_server_statuses(&McpServerStatusListParams {
                    cursor,
                    detail: Some(McpServerStatusDetail::Full),
                    limit: Some(100),
                    thread_id: Some(session.thread_id.clone()),
                })
                .await
                .map_err(acp::Error::into_internal_error)?;
            servers.extend(response.data);
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let value = codex_mcp_list_payload(servers);
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn hooks_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexHooksListRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let cwd = self.extension_cwd(request.session_id.as_deref(), request.cwd.as_deref())?;
        let response = self
            .client
            .hooks(&HooksListParams {
                cwds: vec![cwd.clone()],
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let value = codex_hooks_list_payload(response);
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn hooks_action(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        use xai_hooks_plugins_types::{ActionOutcome, HooksAction, OutcomeStatus};

        let request: xai_hooks_plugins_types::HooksActionRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session_id(&request.session_id)?;
        let outcome = match request.action {
            HooksAction::Reload => {
                self.client
                    .hooks(&HooksListParams {
                        cwds: vec![session.cwd],
                    })
                    .await
                    .map_err(acp::Error::into_internal_error)?;
                ActionOutcome {
                    status: OutcomeStatus::Success,
                    message: "Reloaded Codex hooks.".to_owned(),
                    requires_reload: false,
                    requires_restart: false,
                }
            }
            _ => ActionOutcome {
                status: OutcomeStatus::Unsupported,
                message: "Codex manages hook changes through its native configuration.".to_owned(),
                requires_reload: false,
                requires_restart: false,
            },
        };
        let raw =
            serde_json::value::to_raw_value(&outcome).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn skills_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSkillsListRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let cwd = self.extension_cwd(request.session_id.as_deref(), request.cwd.as_deref())?;
        let (skills, errors) = self.load_codex_skills(&cwd, true).await?;
        let value = json!({"skills": skills, "errors": errors});
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn commands_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexCommandsListRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let cwd = self.extension_cwd(request.session_id.as_deref(), request.cwd.as_deref())?;
        let (skills, _) = self.load_codex_skills(&cwd, true).await?;
        let commands = codex_skill_commands(skills);
        let raw = serde_json::value::to_raw_value(&json!({ "commands": commands }))
            .map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn skills_toggle(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let request: CodexSkillsToggleRequest = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let cwd = self.extension_cwd(request.session_id.as_deref(), request.cwd.as_deref())?;
        let response = self
            .client
            .write_skill_config(&SkillsConfigWriteParams {
                path: None,
                name: Some(request.name),
                enabled: request.enabled,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let (skills, errors) = self.load_codex_skills(&cwd, true).await?;
        let value = json!({
            "skills": skills,
            "errors": errors,
            "effectiveEnabled": response.effective_enabled,
        });
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn load_codex_skills(
        &self,
        cwd: &Path,
        force_reload: bool,
    ) -> Result<
        (
            Vec<xai_grok_tools::implementations::skills::types::SkillInfo>,
            Vec<bot_provider_codex::SkillErrorInfo>,
        ),
        acp::Error,
    > {
        let response = self
            .client
            .skills(&SkillsListParams {
                cwds: vec![cwd.to_path_buf()],
                force_reload,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let mut skills = Vec::new();
        let mut errors = Vec::new();
        for entry in response.data {
            skills.extend(entry.skills.into_iter().map(codex_skill_info));
            errors.extend(entry.errors);
        }
        Ok((skills, errors))
    }

    fn extension_cwd(
        &self,
        session_id: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<PathBuf, acp::Error> {
        if let Some(session_id) = session_id {
            return self.ext_session_id(session_id).map(|session| session.cwd);
        }
        let base = std::env::current_dir().map_err(acp::Error::into_internal_error)?;
        let path = Path::new(cwd.unwrap_or("."));
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        })
    }

    async fn plugins_list(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        let params: Value = serde_json::from_str(params)
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session(&params)?;
        let response = self
            .client
            .plugins(&PluginListParams {
                cwds: Some(vec![session.cwd.clone()]),
                force_refetch: false,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let value = codex_plugins_list_payload(response.marketplaces, &session.cwd);
        let raw =
            serde_json::value::to_raw_value(&value).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn plugins_action(&self, params: &str) -> Result<acp::ExtResponse, acp::Error> {
        use xai_hooks_plugins_types::{ActionOutcome, OutcomeStatus, PluginsAction};

        let request: xai_hooks_plugins_types::PluginsActionRequest =
            serde_json::from_str(params)
                .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let session = self.ext_session_id(&request.session_id)?;
        let outcome = match request.action {
            PluginsAction::Reload => match self
                .client
                .reconcile_plugins(&PluginReconcileParams {
                    reason: Some("Bot plugin reload".to_owned()),
                })
                .await
            {
                Ok(value) => codex_reconcile_outcome(value),
                Err(error) => codex_plugin_error("reload plugins", error),
            },
            PluginsAction::Install { source } => {
                self.install_codex_plugin(&session, source.trim()).await
            }
            PluginsAction::Uninstall { plugin_id, .. } => {
                if plugin_id.trim().is_empty() {
                    ActionOutcome {
                        status: OutcomeStatus::ValidationError,
                        message: "Plugin ID is required.".to_owned(),
                        requires_reload: false,
                        requires_restart: false,
                    }
                } else {
                    match self
                        .client
                        .uninstall_plugin(&PluginUninstallParams {
                            plugin_id: plugin_id.clone(),
                        })
                        .await
                    {
                        Ok(_) => ActionOutcome {
                            status: OutcomeStatus::Success,
                            message: format!("Uninstalled Codex plugin {plugin_id}."),
                            requires_reload: true,
                            requires_restart: false,
                        },
                        Err(error) => codex_plugin_error("uninstall the plugin", error),
                    }
                }
            }
            PluginsAction::Enable { plugin_id } => {
                self.set_codex_plugin_enabled(plugin_id, true).await
            }
            PluginsAction::Disable { plugin_id } => {
                self.set_codex_plugin_enabled(plugin_id, false).await
            }
            PluginsAction::Update { .. } => ActionOutcome {
                status: OutcomeStatus::Unsupported,
                message: "Codex does not provide a per-plugin update action. Reload plugins to reconcile installed bundles.".to_owned(),
                requires_reload: false,
                requires_restart: false,
            },
            PluginsAction::Add { .. } | PluginsAction::Remove { .. } => ActionOutcome {
                status: OutcomeStatus::Unsupported,
                message: "Codex manages plugin sources through marketplaces. Direct plugin paths are not supported by this provider.".to_owned(),
                requires_reload: false,
                requires_restart: false,
            },
        };
        let raw =
            serde_json::value::to_raw_value(&outcome).map_err(acp::Error::into_internal_error)?;
        Ok(acp::ExtResponse::new(raw.into()))
    }

    async fn install_codex_plugin(
        &self,
        session: &CodexSession,
        source: &str,
    ) -> xai_hooks_plugins_types::ActionOutcome {
        use xai_hooks_plugins_types::{ActionOutcome, OutcomeStatus};

        if source.is_empty() {
            return ActionOutcome {
                status: OutcomeStatus::ValidationError,
                message: "Enter a Codex plugin ID in plugin@marketplace form.".to_owned(),
                requires_reload: false,
                requires_restart: false,
            };
        }
        let response = match self
            .client
            .plugins(&PluginListParams {
                cwds: Some(vec![session.cwd.clone()]),
                force_refetch: false,
            })
            .await
        {
            Ok(response) => response,
            Err(error) => return codex_plugin_error("load the plugin catalog", error),
        };
        let mut matches = Vec::new();
        for marketplace in response.marketplaces {
            for plugin in marketplace.plugins {
                if plugin.id == source || plugin.name == source {
                    matches.push((
                        plugin.id,
                        plugin.name,
                        plugin.installed,
                        marketplace.name.clone(),
                        marketplace.path.clone(),
                    ));
                }
            }
        }
        if matches.is_empty() {
            return ActionOutcome {
                status: OutcomeStatus::NotFound,
                message: format!(
                    "Codex plugin {source} was not found. Use plugin@marketplace from the Codex catalog."
                ),
                requires_reload: false,
                requires_restart: false,
            };
        }
        if matches.len() > 1 {
            return ActionOutcome {
                status: OutcomeStatus::ValidationError,
                message: format!(
                    "More than one Codex plugin is named {source}. Enter plugin@marketplace."
                ),
                requires_reload: false,
                requires_restart: false,
            };
        }
        let (plugin_id, plugin_name, installed, marketplace_name, marketplace_path) =
            matches.remove(0);
        if installed {
            return ActionOutcome {
                status: OutcomeStatus::Success,
                message: format!("Codex plugin {plugin_id} is already installed."),
                requires_reload: false,
                requires_restart: false,
            };
        }
        let remote_marketplace_name = marketplace_path
            .is_none()
            .then_some(marketplace_name.clone());
        match self
            .client
            .install_plugin(&PluginInstallParams {
                marketplace_path,
                remote_marketplace_name,
                install_attempt_id: None,
                plugin_name,
            })
            .await
        {
            Ok(value) => {
                let auth_count = value
                    .get("appsNeedingAuth")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                let message = if auth_count == 0 {
                    format!("Installed Codex plugin {plugin_id}.")
                } else {
                    format!(
                        "Installed Codex plugin {plugin_id}. {auth_count} app connection(s) need authentication."
                    )
                };
                ActionOutcome {
                    status: OutcomeStatus::Success,
                    message,
                    requires_reload: true,
                    requires_restart: false,
                }
            }
            Err(error) => codex_plugin_error("install the plugin", error),
        }
    }

    async fn set_codex_plugin_enabled(
        &self,
        plugin_id: String,
        enabled: bool,
    ) -> xai_hooks_plugins_types::ActionOutcome {
        use xai_hooks_plugins_types::{ActionOutcome, OutcomeStatus};

        if plugin_id.trim().is_empty() {
            return ActionOutcome {
                status: OutcomeStatus::ValidationError,
                message: "Plugin ID is required.".to_owned(),
                requires_reload: false,
                requires_restart: false,
            };
        }
        let escaped_id = plugin_id.replace('\\', "\\\\").replace('"', "\\\"");
        let write = self
            .client
            .write_config_value(&ConfigValueWriteParams {
                key_path: format!("plugins.\"{escaped_id}\".enabled"),
                value: Value::Bool(enabled),
                merge_strategy: MergeStrategy::Upsert,
                file_path: None,
                expected_version: None,
            })
            .await;
        if let Err(error) = write {
            return codex_plugin_error("change plugin state", error);
        }
        if let Err(error) = self
            .client
            .reconcile_plugins(&PluginReconcileParams {
                reason: Some(format!(
                    "Bot {} plugin {plugin_id}",
                    if enabled { "enabled" } else { "disabled" }
                )),
            })
            .await
        {
            return codex_plugin_error("reload plugin state", error);
        }
        ActionOutcome {
            status: OutcomeStatus::Success,
            message: format!(
                "{} Codex plugin {plugin_id}.",
                if enabled { "Enabled" } else { "Disabled" }
            ),
            requires_reload: false,
            requires_restart: false,
        }
    }

    fn ext_session(&self, params: &Value) -> Result<CodexSession, acp::Error> {
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| acp::Error::invalid_params().data("Missing Codex session ID"))?;
        self.ext_session_id(session_id)
    }

    fn ext_session_id(&self, session_id: &str) -> Result<CodexSession, acp::Error> {
        self.sessions
            .borrow()
            .get(&acp::SessionId::new(session_id.to_owned()))
            .cloned()
            .ok_or_else(|| acp::Error::resource_not_found(Some(session_id.to_owned())))
    }

    async fn interrupt_codex_turn(
        &self,
        thread_id: String,
        turn_id: String,
        stop_background_terminals: bool,
    ) -> Result<(), acp::Error> {
        self.client
            .interrupt_turn(&TurnInterruptParams {
                thread_id: thread_id.clone(),
                turn_id,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        if stop_background_terminals {
            self.client
                .clean_background_terminals(
                    &bot_provider_codex::ThreadBackgroundTerminalsCleanParams { thread_id },
                )
                .await
                .map_err(acp::Error::into_internal_error)?;
        }
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Agent for CodexAcpAgent {
    async fn initialize(
        &self,
        _args: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        let state = self.model_state(&self.default_model, None);
        let mut meta = Map::new();
        meta.insert("grokShell".to_owned(), Value::Bool(false));
        meta.insert(
            "modelState".to_owned(),
            serde_json::to_value(state).map_err(acp::Error::into_internal_error)?,
        );
        let auth_methods = self.auth_methods();
        let default_auth_method = auth_methods
            .first()
            .map(|method| method.id().0.to_string())
            .unwrap_or_else(|| CODEX_CHATGPT_AUTH_METHOD.to_owned());
        meta.insert(
            "defaultAuthMethodId".to_owned(),
            Value::String(default_auth_method),
        );
        Ok(acp::InitializeResponse::new(acp::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new()
                    .load_session(true)
                    .prompt_capabilities(acp::PromptCapabilities::new().image(true)),
            )
            .auth_methods(auth_methods)
            .agent_info(acp::Implementation::new("codex", env!("CARGO_PKG_VERSION")).title("Codex"))
            .meta(meta))
    }

    async fn authenticate(
        &self,
        args: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        let request_seq = args
            .meta
            .as_ref()
            .and_then(|meta| meta.get("request_seq"))
            .and_then(Value::as_u64);
        let force_interactive = args
            .meta
            .as_ref()
            .and_then(|meta| meta.get("force_interactive"))
            .and_then(Value::as_bool)
            == Some(true);
        match args.method_id.0.as_ref() {
            CODEX_CACHED_AUTH_METHOD if force_interactive => {
                self.authenticate_interactively(LoginAccountParams::chatgpt(), request_seq)
                    .await
            }
            CODEX_CACHED_AUTH_METHOD => {
                let account = self
                    .client
                    .account()
                    .await
                    .map_err(acp::Error::into_internal_error)?;
                let signed_in = account.account.is_some() || !account.requires_openai_auth;
                self.account.replace(account);
                if signed_in {
                    Ok(acp::AuthenticateResponse::new())
                } else {
                    Err(acp::Error::auth_required().data("Codex is signed out"))
                }
            }
            CODEX_CHATGPT_AUTH_METHOD => {
                self.authenticate_interactively(LoginAccountParams::chatgpt(), request_seq)
                    .await
            }
            CODEX_DEVICE_AUTH_METHOD => {
                self.authenticate_interactively(LoginAccountParams::ChatgptDeviceCode, request_seq)
                    .await
            }
            _ => Err(acp::Error::invalid_params().data("Unknown Codex auth method")),
        }
    }

    async fn new_session(
        &self,
        args: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        let dynamic_tools = codex_dynamic_tools(args.meta.as_ref())?;
        let cwd = args.cwd;
        let permission_mode = *self.default_permission_mode.borrow();
        let started = self
            .client
            .start_thread(&ThreadStartParams {
                model: Some(self.default_model.clone()),
                cwd: Some(cwd.to_string_lossy().into_owned()),
                approval_policy: permission_mode.approval_policy(),
                approvals_reviewer: permission_mode.approvals_reviewer(),
                sandbox: permission_mode.sandbox(),
                ephemeral: Some(false),
                dynamic_tools,
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let session_id = acp::SessionId::new(started.thread.id.clone());
        let model = started.model;
        let effort = started.reasoning_effort;
        self.sessions.borrow_mut().insert(
            session_id.clone(),
            CodexSession {
                thread_id: started.thread.id,
                cwd,
                model: model.clone(),
                effort: effort.clone(),
                model_efforts: effort
                    .clone()
                    .map(|value| (model.clone(), value))
                    .into_iter()
                    .collect(),
                permission_mode,
                turn_state: CodexTurnState::Idle,
                mode: CollaborationModeKind::Default,
            },
        );
        self.refresh_provider_usage(&session_id).await;
        Ok(acp::NewSessionResponse::new(session_id)
            .modes(codex_mode_state(CollaborationModeKind::Default))
            .models(self.model_state(&model, effort.as_deref())))
    }

    async fn load_session(
        &self,
        args: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        let permission_mode = *self.default_permission_mode.borrow();
        let resumed = self
            .client
            .resume_thread(&ThreadResumeParams {
                thread_id: args.session_id.0.to_string(),
                model: None,
                cwd: Some(args.cwd.to_string_lossy().into_owned()),
                approval_policy: permission_mode.approval_policy(),
                approvals_reviewer: permission_mode.approvals_reviewer(),
                sandbox: permission_mode.sandbox(),
            })
            .await
            .map_err(acp::Error::into_internal_error)?;
        let model = resumed.model;
        let effort = resumed.reasoning_effort;
        let thread_id = resumed.thread.id.clone();
        let turns = resumed.thread.turns;
        let cwd = if resumed.cwd.as_os_str().is_empty() {
            args.cwd
        } else {
            resumed.cwd
        };
        self.sessions.borrow_mut().insert(
            args.session_id.clone(),
            CodexSession {
                thread_id,
                cwd,
                model: model.clone(),
                effort: effort.clone(),
                model_efforts: effort
                    .clone()
                    .map(|value| (model.clone(), value))
                    .into_iter()
                    .collect(),
                permission_mode,
                turn_state: CodexTurnState::Idle,
                mode: CollaborationModeKind::Default,
            },
        );
        self.refresh_provider_usage(&args.session_id).await;
        for turn in &turns {
            for update in replay_updates(&args.session_id, std::slice::from_ref(turn)) {
                self.notify(&args.session_id, update).await;
            }
            if let Some(notification) = replay_turn_completion(&args.session_id, turn)
                .map_err(acp::Error::into_internal_error)?
            {
                let _ = self.gateway.ext_notification(notification).await;
            }
        }
        Ok(acp::LoadSessionResponse::new()
            .modes(codex_mode_state(CollaborationModeKind::Default))
            .models(self.model_state(&model, effort.as_deref())))
    }

    async fn prompt(&self, args: acp::PromptRequest) -> Result<acp::PromptResponse, acp::Error> {
        let session = self
            .sessions
            .borrow()
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| acp::Error::resource_not_found(Some(args.session_id.0.to_string())))?;
        let image_dir = tempfile::tempdir().map_err(acp::Error::into_internal_error)?;
        let input = prompt_input(&args.prompt, image_dir.path())?;
        let permission_mode = session.permission_mode;
        {
            let mut sessions = self.sessions.borrow_mut();
            let current = sessions.get_mut(&args.session_id).ok_or_else(|| {
                acp::Error::resource_not_found(Some(args.session_id.0.to_string()))
            })?;
            if current.turn_state.is_busy() {
                return Err(acp::Error::invalid_params().data("A Codex turn is already running"));
            }
            current.turn_state = CodexTurnState::Starting {
                cancel_requested: false,
                stop_background_terminals: false,
            };
        }
        let events = self.client.subscribe();
        let turn = self
            .client
            .start_turn(&TurnStartParams {
                thread_id: session.thread_id.clone(),
                input,
                model: Some(session.model.clone()),
                effort: session.effort.clone(),
                summary: Some(ReasoningSummary::Auto),
                cwd: Some(session.cwd.to_string_lossy().into_owned()),
                client_user_message_id: args.message_id.clone(),
                approval_policy: permission_mode.approval_policy(),
                approvals_reviewer: permission_mode.approvals_reviewer(),
                sandbox_policy: permission_mode.sandbox_policy(),
                collaboration_mode: Some(CollaborationMode {
                    mode: session.mode,
                    settings: CollaborationModeSettings {
                        model: session.model.clone(),
                        reasoning_effort: session.effort.clone(),
                        developer_instructions: None,
                    },
                }),
            })
            .await;
        let turn = match turn {
            Ok(turn) => turn,
            Err(error) => {
                if let Some(session) = self.sessions.borrow_mut().get_mut(&args.session_id) {
                    session.turn_state = CodexTurnState::Idle;
                }
                return Err(acp::Error::into_internal_error(error));
            }
        };
        let turn_id = turn.turn.id;
        let pending_cancel =
            if let Some(session) = self.sessions.borrow_mut().get_mut(&args.session_id) {
                let previous = std::mem::replace(
                    &mut session.turn_state,
                    CodexTurnState::Active {
                        turn_id: turn_id.clone(),
                    },
                );
                match previous {
                    CodexTurnState::Starting {
                        cancel_requested,
                        stop_background_terminals,
                    } => cancel_requested.then_some(stop_background_terminals),
                    CodexTurnState::Idle | CodexTurnState::Active { .. } => None,
                }
            } else {
                None
            };
        if let Some(stop_background_terminals) = pending_cancel
            && let Err(error) = self
                .interrupt_codex_turn(
                    session.thread_id.clone(),
                    turn_id.clone(),
                    stop_background_terminals,
                )
                .await
        {
            if let Some(session) = self.sessions.borrow_mut().get_mut(&args.session_id) {
                session.turn_state = CodexTurnState::Idle;
            }
            return Err(error);
        }
        if let Some(prompt_id) = args.meta.as_ref().and_then(|meta| meta.get("promptId")) {
            let payload = json!({
                "sessionId": args.session_id.0,
                "entries": [],
                "runningPromptId": prompt_id,
            });
            let raw = serde_json::value::to_raw_value(&payload)
                .map_err(acp::Error::into_internal_error)?;
            let result = self
                .gateway
                .ext_notification(acp::ExtNotification::new("x.ai/queue/changed", raw.into()))
                .await;
            if let Err(error) = result {
                if let Some(session) = self.sessions.borrow_mut().get_mut(&args.session_id) {
                    session.turn_state = CodexTurnState::Idle;
                }
                return Err(error);
            }
        }
        let stop_reason = self
            .stream_turn(&args.session_id, &session.thread_id, &turn_id, events)
            .await;
        if let Some(session) = self.sessions.borrow_mut().get_mut(&args.session_id) {
            session.turn_state = CodexTurnState::Idle;
        }
        self.refresh_provider_usage(&args.session_id).await;
        let mut response = acp::PromptResponse::new(stop_reason?);
        if let Some(message_id) = args.message_id {
            response = response.user_message_id(message_id);
        }
        Ok(response)
    }

    async fn cancel(&self, args: acp::CancelNotification) -> Result<(), acp::Error> {
        let stop_background_terminals = args
            .meta
            .as_ref()
            .and_then(|meta| meta.get("stopBackgroundTerminals"))
            .and_then(Value::as_bool)
            == Some(true);
        let active = {
            let mut sessions = self.sessions.borrow_mut();
            sessions
                .get_mut(&args.session_id)
                .and_then(|session| match &mut session.turn_state {
                    CodexTurnState::Starting {
                        cancel_requested,
                        stop_background_terminals: pending_stop,
                    } => {
                        *cancel_requested = true;
                        *pending_stop |= stop_background_terminals;
                        None
                    }
                    CodexTurnState::Active { turn_id } => {
                        Some((session.thread_id.clone(), turn_id.clone()))
                    }
                    CodexTurnState::Idle => None,
                })
        };
        if let Some((thread_id, turn_id)) = active {
            self.interrupt_codex_turn(thread_id, turn_id, stop_background_terminals)
                .await?;
        }
        Ok(())
    }

    async fn set_session_mode(
        &self,
        args: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        let mode = match args.mode_id.0.as_ref() {
            "default" => CollaborationModeKind::Default,
            "plan" => CollaborationModeKind::Plan,
            _ => {
                return Err(
                    acp::Error::invalid_params().data("Choose Default or Plan mode for Codex")
                );
            }
        };
        {
            let mut sessions = self.sessions.borrow_mut();
            let session = sessions.get_mut(&args.session_id).ok_or_else(|| {
                acp::Error::resource_not_found(Some(args.session_id.0.to_string()))
            })?;
            if session.turn_state.is_busy() {
                return Err(acp::Error::invalid_params()
                    .data("Wait for the current turn to finish, then change mode"));
            }
            session.mode = mode;
        }
        self.notify(
            &args.session_id,
            acp::SessionUpdate::CurrentModeUpdate(acp::CurrentModeUpdate::new(codex_mode_id(mode))),
        )
        .await;
        Ok(acp::SetSessionModeResponse::new())
    }

    async fn set_session_model(
        &self,
        args: acp::SetSessionModelRequest,
    ) -> Result<acp::SetSessionModelResponse, acp::Error> {
        let requested = args.model_id.0.to_string();
        let model = self
            .models
            .iter()
            .find(|model| model.model == requested)
            .ok_or_else(|| {
                acp::Error::invalid_params()
                    .data(format!("Codex model is not available: {requested}"))
            })?;
        let effort = args
            .meta
            .as_ref()
            .and_then(|meta| meta.get("reasoningEffort"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut sessions = self.sessions.borrow_mut();
        let session = sessions
            .get_mut(&args.session_id)
            .ok_or_else(|| acp::Error::resource_not_found(Some(args.session_id.0.to_string())))?;
        if session.turn_state.is_busy() {
            return Err(acp::Error::invalid_params()
                .data("Wait for the current turn to stop before you change the model"));
        }
        let effort = resolve_model_effort(
            model,
            effort.as_deref(),
            session.model_efforts.get(&requested).map(String::as_str),
        )?;
        if let Some(value) = &effort {
            session
                .model_efforts
                .insert(requested.clone(), value.clone());
        }
        session.model = requested;
        session.effort = effort;
        Ok(acp::SetSessionModelResponse::new())
    }

    async fn ext_method(&self, args: acp::ExtRequest) -> Result<acp::ExtResponse, acp::Error> {
        match args.method.as_ref() {
            ACCOUNT_STATUS_METHOD => raw_ext_response(&self.account_status().await?),
            AUTH_CANCEL_METHOD => {
                let params: CodexAuthCancelRequest = serde_json::from_str(args.params.get())
                    .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
                let cancelled = self.cancel_pending_login(params.request_seq).await?;
                raw_ext_response(&json!({"cancelled": cancelled}))
            }
            AUTH_GET_URL_METHOD => {
                let pending = self.pending_login.borrow().clone();
                raw_ext_response(&match pending {
                    Some(pending) => json!({
                        "auth_url": pending.auth_url,
                        "external_provider": true,
                        "mode": pending.mode,
                    }),
                    None => json!({
                        "auth_url": null,
                        "external_provider": true,
                        "mode": null,
                    }),
                })
            }
            AUTH_LOGOUT_METHOD => {
                self.cancel_pending_login(None).await?;
                self.client
                    .logout_account()
                    .await
                    .map_err(acp::Error::into_internal_error)?;
                let account = self
                    .client
                    .account()
                    .await
                    .map_err(acp::Error::into_internal_error)?;
                self.account.replace(account);
                self.clear_provider_usage().await;
                raw_ext_response(&json!({"ok": true}))
            }
            COMPACT_CONVERSATION_METHOD => self.compact_conversation(args.params.get()).await,
            COMMANDS_LIST_METHOD => self.commands_list(args.params.get()).await,
            INTERJECT_METHOD => self.interject(args.params.get()).await,
            HOOKS_ACTION_METHOD => self.hooks_action(args.params.get()).await,
            HOOKS_LIST_METHOD => self.hooks_list(args.params.get()).await,
            SESSION_LIST_METHOD => self.session_list(args.params.get()).await,
            MCP_LIST_METHOD => self.mcp_list(args.params.get()).await,
            PLUGINS_ACTION_METHOD => self.plugins_action(args.params.get()).await,
            PLUGINS_LIST_METHOD => self.plugins_list(args.params.get()).await,
            REWIND_EXECUTE_METHOD => self.rewind_execute(args.params.get()).await,
            REWIND_POINTS_METHOD => self.rewind_points(args.params.get()).await,
            SESSION_DELETE_METHOD => self.delete_session(args.params.get()).await,
            SESSION_FORK_METHOD => self.fork_session(args.params.get()).await,
            SESSION_RENAME_METHOD => self.rename_session(args.params.get()).await,
            SESSION_SEARCH_METHOD => self.search_sessions(args.params.get()).await,
            SKILLS_LIST_METHOD => self.skills_list(args.params.get()).await,
            SKILLS_TOGGLE_METHOD => self.skills_toggle(args.params.get()).await,
            _ => Err(acp::Error::method_not_found()),
        }
    }

    async fn ext_notification(&self, args: acp::ExtNotification) -> Result<(), acp::Error> {
        if args.method.as_ref() != "x.ai/yolo_mode_changed" {
            return Ok(());
        }
        let params: Value = serde_json::from_str(args.params.get())
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        let permission_mode = CodexPermissionMode::from_wire(&params)
            .ok_or_else(|| acp::Error::invalid_params().data("Unknown permission mode"))?;
        if let Some(session_id) = params.get("sessionId").and_then(Value::as_str) {
            let session_id = acp::SessionId::new(session_id.to_owned());
            let mut sessions = self.sessions.borrow_mut();
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| acp::Error::resource_not_found(Some(session_id.0.to_string())))?;
            session.permission_mode = permission_mode;
        } else {
            self.default_permission_mode.replace(permission_mode);
        }
        Ok(())
    }
}

#[derive(Default, Deserialize)]
struct CodexAuthCancelRequest {
    #[serde(default)]
    request_seq: Option<u64>,
}

fn codex_interactive_auth_method(id: &str, name: &str, mode: &str) -> acp::AuthMethod {
    let meta = json!({
        "external_provider": true,
        "auth_mode": mode,
    })
    .as_object()
    .cloned();
    acp::AuthMethod::Agent(acp::AuthMethodAgent::new(id.to_owned(), name.to_owned()).meta(meta))
}

fn codex_interactive_auth_methods(browser_available: bool) -> Vec<acp::AuthMethod> {
    let browser =
        codex_interactive_auth_method(CODEX_CHATGPT_AUTH_METHOD, "Sign in with ChatGPT", "browser");
    let device = codex_interactive_auth_method(
        CODEX_DEVICE_AUTH_METHOD,
        "Sign in with a device code",
        "device",
    );
    if browser_available {
        vec![browser, device]
    } else {
        vec![device, browser]
    }
}

fn browser_login_available() -> bool {
    std::env::var_os("GROK_TEST_OPEN_URL_FILE").is_some()
        || crate::app::link_opener::browser_open_likely_available()
}

fn device_auth_url(verification_url: &str, user_code: &str) -> String {
    let separator = if verification_url.contains('?') {
        '&'
    } else {
        '?'
    };
    let encoded_code =
        url::form_urlencoded::byte_serialize(user_code.as_bytes()).collect::<String>();
    format!("{verification_url}{separator}user_code={encoded_code}")
}

fn raw_ext_response<T: serde::Serialize>(value: &T) -> Result<acp::ExtResponse, acp::Error> {
    let raw = serde_json::value::to_raw_value(value).map_err(acp::Error::into_internal_error)?;
    Ok(acp::ExtResponse::new(raw.into()))
}

fn map_rate_limit(snapshot: bot_provider_codex::RateLimitSnapshot) -> ProviderRateLimit {
    ProviderRateLimit {
        name: snapshot
            .limit_name
            .or(snapshot.limit_id)
            .unwrap_or_else(|| "Codex".to_owned()),
        model: snapshot.normal_model_slug,
        primary: snapshot.primary.map(map_rate_limit_window),
        secondary: snapshot.secondary.map(map_rate_limit_window),
        credits: snapshot.credits.map(|credits| ProviderCredits {
            has_credits: credits.has_credits,
            unlimited: credits.unlimited,
            balance: credits.balance,
        }),
        limit_reached: snapshot.rate_limit_reached_type.or_else(|| {
            (snapshot.spend_control_reached == Some(true))
                .then_some("spend_control_reached".to_owned())
        }),
    }
}

fn map_rate_limit_window(window: bot_provider_codex::RateLimitWindow) -> ProviderRateLimitWindow {
    ProviderRateLimitWindow {
        used_percent: window.used_percent,
        window_minutes: window.window_duration_mins,
        resets_at: window.resets_at,
    }
}

fn map_usage_limits(response: AccountRateLimitsResponse) -> Vec<UsageLimit> {
    let snapshots = match response.rate_limits_by_limit_id {
        Some(items) if !items.is_empty() => items.into_values().collect(),
        _ => vec![response.rate_limits],
    };
    snapshots.into_iter().map(map_usage_limit).collect()
}

fn map_usage_limit(snapshot: bot_provider_codex::RateLimitSnapshot) -> UsageLimit {
    let mut windows = Vec::with_capacity(2);
    if let Some(window) = snapshot.primary {
        windows.push(map_usage_window("Primary", window));
    }
    if let Some(window) = snapshot.secondary {
        windows.push(map_usage_window("Secondary", window));
    }
    UsageLimit {
        id: snapshot.limit_id,
        name: snapshot.limit_name.unwrap_or_else(|| "Codex".to_owned()),
        model: snapshot.normal_model_slug,
        windows,
    }
}

fn map_usage_window(label: &str, window: bot_provider_codex::RateLimitWindow) -> UsageLimitWindow {
    UsageLimitWindow {
        label: label.to_owned(),
        used_percent: f64::from(window.used_percent),
        duration_minutes: window
            .window_duration_mins
            .and_then(|minutes| u64::try_from(minutes).ok()),
        resets_at: window.resets_at,
    }
}

fn codex_display_name(value: &str) -> String {
    match value {
        "chatgpt" => "ChatGPT".to_owned(),
        "apikey" | "apiKey" => "API key".to_owned(),
        _ => value
            .split(['_', '-'])
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut characters = word.chars();
                characters
                    .next()
                    .map(|first| first.to_uppercase().chain(characters).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

async fn load_models(client: &CodexClient) -> Result<Vec<Model>> {
    let mut models = Vec::new();
    let mut cursor = None;
    loop {
        let response = client
            .models(&ModelListParams {
                cursor,
                include_hidden: Some(false),
                limit: Some(100),
            })
            .await
            .context("failed to read Codex models")?;
        models.extend(response.data.into_iter().filter(|model| !model.hidden));
        cursor = response.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(models)
}

fn model_info(model: &Model, current_effort: Option<&str>) -> acp::ModelInfo {
    let reasoning_efforts = model
        .supported_reasoning_efforts
        .iter()
        .map(|effort| {
            json!({
                "id": effort.reasoning_effort,
                "value": effort.reasoning_effort,
                "label": effort.reasoning_effort,
                "description": effort.description,
                "default": effort.reasoning_effort == model.default_reasoning_effort,
            })
        })
        .collect::<Vec<_>>();
    let input_modalities = model
        .input_modalities
        .iter()
        .map(|modality| match modality {
            bot_provider_codex::InputModality::Text => "text",
            bot_provider_codex::InputModality::Image => "image",
            bot_provider_codex::InputModality::Audio => "audio",
        })
        .collect::<Vec<_>>();
    let accepts_images = input_modalities.contains(&"image");
    let mut meta = Map::new();
    meta.insert(
        "supportsReasoningEffort".to_owned(),
        Value::Bool(!reasoning_efforts.is_empty()),
    );
    if !reasoning_efforts.is_empty() {
        meta.insert(
            "reasoningEffort".to_owned(),
            Value::String(
                current_effort
                    .unwrap_or(&model.default_reasoning_effort)
                    .to_owned(),
            ),
        );
    }
    meta.insert(
        "reasoningEfforts".to_owned(),
        Value::Array(reasoning_efforts),
    );
    meta.insert("acceptsImages".to_owned(), Value::Bool(accepts_images));
    meta.insert(
        "inputModalities".to_owned(),
        serde_json::to_value(input_modalities).unwrap_or_else(|_| Value::Array(Vec::new())),
    );
    meta.insert(
        "providerDisplayName".to_owned(),
        Value::String(model.display_name.clone()),
    );
    acp::ModelInfo::new(model.model.clone(), model.model.clone())
        .description(model.description.clone())
        .meta(meta)
}

fn resolve_model_effort(
    model: &Model,
    requested: Option<&str>,
    saved: Option<&str>,
) -> Result<Option<String>, acp::Error> {
    let supported = |value: &str| {
        model
            .supported_reasoning_efforts
            .iter()
            .any(|option| option.reasoning_effort == value)
    };
    if let Some(value) = requested {
        if !supported(value) {
            return Err(acp::Error::invalid_params().data(format!(
                "Codex model {} does not support effort {value}",
                model.model
            )));
        }
        return Ok(Some(value.to_owned()));
    }
    Ok(saved
        .filter(|value| supported(value))
        .or_else(|| {
            supported(&model.default_reasoning_effort)
                .then_some(model.default_reasoning_effort.as_str())
        })
        .map(str::to_owned))
}

fn prompt_input(
    blocks: &[acp::ContentBlock],
    image_dir: &Path,
) -> Result<Vec<UserInput>, acp::Error> {
    let mut input = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        match block {
            acp::ContentBlock::Text(text) => {
                if let Some((skill, arguments)) = codex_skill_input(text) {
                    input.push(skill);
                    if !arguments.is_empty() {
                        input.push(UserInput::Text {
                            text: arguments,
                            text_elements: Vec::new(),
                        });
                    }
                } else {
                    input.push(UserInput::Text {
                        text: text.text.clone(),
                        text_elements: Vec::new(),
                    });
                }
            }
            acp::ContentBlock::Image(image) => {
                let path = local_image_path(image).unwrap_or_else(|| {
                    image_dir.join(format!(
                        "image-{index}.{}",
                        image_extension(&image.mime_type)
                    ))
                });
                if !path.exists() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&image.data)
                        .map_err(acp::Error::into_internal_error)?;
                    std::fs::write(&path, bytes).map_err(acp::Error::into_internal_error)?;
                }
                input.push(UserInput::LocalImage { path, detail: None });
            }
            acp::ContentBlock::ResourceLink(link) => input.push(UserInput::Text {
                text: format!("Resource: {} ({})", link.name, link.uri),
                text_elements: Vec::new(),
            }),
            acp::ContentBlock::Resource(resource) => input.push(UserInput::Text {
                text: serde_json::to_string(&resource.resource)
                    .map_err(acp::Error::into_internal_error)?,
                text_elements: Vec::new(),
            }),
            acp::ContentBlock::Audio(_) => {
                return Err(acp::Error::invalid_params().data("Codex audio input is not enabled"));
            }
            _ => return Err(acp::Error::invalid_params().data("Unsupported Codex input block")),
        }
    }
    Ok(input)
}

fn codex_skill_input(text: &acp::TextContent) -> Option<(UserInput, String)> {
    let command = text.meta.as_ref()?.get("botCommand")?;
    let ownership = serde_json::from_value::<CommandOwnership>(command.clone()).ok()?;
    if ownership.provider() != "codex" || ownership.kind() != CommandKind::Skill {
        return None;
    }
    let name = command.get("name")?.as_str()?.to_owned();
    let path = PathBuf::from(command.get("path")?.as_str()?);
    let arguments = command
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some((UserInput::Skill { name, path }, arguments))
}

fn local_image_path(image: &acp::ImageContent) -> Option<PathBuf> {
    let uri = image.uri.as_deref()?;
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    path.is_file().then_some(path)
}

fn image_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

fn event_matches(event: &CodexEvent, thread_id: &str, turn_id: &str) -> bool {
    let params = match event {
        CodexEvent::Notification(notification) => Some(&notification.params),
        CodexEvent::Request(request) => Some(&request.params),
        CodexEvent::UnmatchedResponse(_) => return false,
        CodexEvent::ConnectionClosed(_) => return true,
    };
    let Some(params) = params else {
        return false;
    };
    if let Some(candidate) = params.get("threadId").and_then(Value::as_str)
        && candidate != thread_id
    {
        return false;
    }
    if let Some(candidate) = params.get("conversationId").and_then(Value::as_str)
        && candidate != thread_id
    {
        return false;
    }
    if let Some(candidate) = params.get("turnId").and_then(Value::as_str)
        && candidate != turn_id
    {
        return false;
    }
    if let Some(candidate) = params
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        && candidate != turn_id
    {
        return false;
    }
    true
}

fn protocol_session_update(
    notification: &bot_provider_codex::ServerNotification,
) -> Option<acp::SessionUpdate> {
    match notification.method.as_str() {
        "turn/plan/updated" => {
            let entries = notification
                .params
                .get("plan")?
                .as_array()?
                .iter()
                .filter_map(|entry| {
                    let content = entry.get("step")?.as_str()?;
                    let status = match entry.get("status")?.as_str()? {
                        "pending" => acp::PlanEntryStatus::Pending,
                        "inProgress" => acp::PlanEntryStatus::InProgress,
                        "completed" => acp::PlanEntryStatus::Completed,
                        _ => return None,
                    };
                    Some(acp::PlanEntry::new(
                        content,
                        acp::PlanEntryPriority::Medium,
                        status,
                    ))
                })
                .collect();
            Some(acp::SessionUpdate::Plan(acp::Plan::new(entries)))
        }
        _ => None,
    }
}

fn protocol_file_change_calls(
    session_id: &acp::SessionId,
    notification: &bot_provider_codex::ServerNotification,
) -> Vec<(String, acp::ToolCall)> {
    match notification.method.as_str() {
        "item/started" | "item/completed" => {
            let Some(item) = notification.params.get("item") else {
                return Vec::new();
            };
            if item.get("type").and_then(Value::as_str) != Some("fileChange") {
                return Vec::new();
            }
            file_change_calls(
                session_id,
                item.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("file-change"),
                item.get("changes"),
                item.get("status").and_then(Value::as_str),
            )
        }
        "item/fileChange/patchUpdated" => file_change_calls(
            session_id,
            notification
                .params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or("file-change"),
            notification.params.get("changes"),
            Some("inProgress"),
        ),
        _ => Vec::new(),
    }
}

fn file_change_calls(
    session_id: &acp::SessionId,
    item_id: &str,
    changes: Option<&Value>,
    status: Option<&str>,
) -> Vec<(String, acp::ToolCall)> {
    let Some(changes) = changes.and_then(Value::as_array) else {
        return Vec::new();
    };
    let multiple = changes.len() > 1;
    changes
        .iter()
        .enumerate()
        .filter_map(|(index, change)| {
            let path = change.get("path")?.as_str()?;
            let diff = change
                .get("diff")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let base_id = codex_tool_id(session_id, item_id);
            let tool_id = if multiple {
                format!("{base_id}:{index}")
            } else {
                base_id
            };
            let call = file_diff_call(
                tool_id.clone(),
                path,
                diff,
                codex_tool_status(status),
                change.clone(),
            );
            Some((tool_id, call))
        })
        .collect()
}

fn turn_diff_calls(
    session_id: &acp::SessionId,
    update: &TurnDiffUpdatedNotification,
    item_file_paths: &HashSet<PathBuf>,
) -> Vec<(String, acp::ToolCall)> {
    let files = split_turn_diff(&update.diff)
        .into_iter()
        .filter(|file| !item_file_paths.contains(Path::new(&file.path)))
        .collect::<Vec<_>>();
    if files.is_empty() {
        if update.diff.trim().is_empty() || !item_file_paths.is_empty() {
            return Vec::new();
        }
        let tool_id = codex_tool_id(session_id, &format!("turn-diff:{}", update.turn_id));
        let call = acp::ToolCall::new(tool_id.clone(), "Turn changes")
            .kind(acp::ToolKind::Other)
            .status(acp::ToolCallStatus::Completed)
            .content(vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                acp::TextContent::new(update.diff.clone()),
            ))])
            .raw_input(json!({"diff": update.diff}));
        return vec![(tool_id, call)];
    }
    let multiple = files.len() > 1;
    let base_id = codex_tool_id(session_id, &format!("turn-diff:{}", update.turn_id));
    files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            let tool_id = if multiple {
                format!("{base_id}:{index}")
            } else {
                base_id.clone()
            };
            let raw_input = json!({"path": &file.path, "diff": &file.diff});
            let call = file_diff_call(
                tool_id.clone(),
                &file.path,
                &file.diff,
                acp::ToolCallStatus::Completed,
                raw_input,
            );
            (tool_id, call)
        })
        .collect()
}

fn file_diff_call(
    tool_id: String,
    path: &str,
    diff: &str,
    status: acp::ToolCallStatus,
    raw_input: Value,
) -> acp::ToolCall {
    let details = parse_unified_diff(diff);
    let (kind, content) = if details.is_empty() && !diff.trim().is_empty() {
        (
            acp::ToolKind::Other,
            vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                acp::TextContent::new(diff.to_owned()),
            ))],
        )
    } else {
        let meta = json!({ "details": details }).as_object().cloned();
        (
            acp::ToolKind::Edit,
            vec![acp::ToolCallContent::Diff(
                acp::Diff::new(path, "").meta(meta),
            )],
        )
    };
    acp::ToolCall::new(tool_id, format!("Edit {path}"))
        .kind(kind)
        .status(status)
        .content(content)
        .locations(vec![acp::ToolCallLocation::new(path)])
        .raw_input(raw_input)
}

fn codex_tool_status(status: Option<&str>) -> acp::ToolCallStatus {
    match status {
        Some("completed") => acp::ToolCallStatus::Completed,
        Some("failed" | "declined" | "error" | "cancelled" | "interrupted") => {
            acp::ToolCallStatus::Failed
        }
        Some("pending") => acp::ToolCallStatus::Pending,
        _ => acp::ToolCallStatus::InProgress,
    }
}

fn mcp_elicitation_payload(
    session_id: &acp::SessionId,
    request: &ServerRequest,
) -> Result<McpElicitExtRequest, serde_json::Error> {
    let params =
        serde_json::from_value::<McpServerElicitationRequestParams>(request.params.clone())?;
    let (message, meta, mode) = match params.request {
        McpServerElicitationRequest::UserVerification {
            title,
            description,
            challenge,
        } => (
            description,
            None,
            McpElicitModeFields::Form {
                requested_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "proof": {
                            "type": "string",
                            "title": title,
                            "description": challenge
                        }
                    },
                    "required": ["proof"]
                })),
            },
        ),
        McpServerElicitationRequest::Form {
            meta,
            message,
            requested_schema,
        }
        | McpServerElicitationRequest::OpenAiForm {
            meta,
            message,
            requested_schema,
        }
        | McpServerElicitationRequest::OpenAiElicitationForm {
            meta,
            message,
            requested_schema,
        } => (
            message,
            meta,
            McpElicitModeFields::Form {
                requested_schema: Some(requested_schema),
            },
        ),
        McpServerElicitationRequest::Url {
            meta,
            message,
            url,
            elicitation_id,
        } => (
            message,
            meta,
            McpElicitModeFields::Url {
                url,
                elicitation_id,
            },
        ),
    };
    Ok(McpElicitExtRequest {
        session_id: session_id.0.to_string(),
        tool_call_id: format!("mcp-elicit-{}", request.id),
        server_name: params.server_name,
        meta,
        message,
        mode,
    })
}

fn mcp_elicitation_result(response: McpElicitExtResponse) -> McpServerElicitationRequestResponse {
    match response {
        McpElicitExtResponse::Accept { content } => McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Accept,
            content,
            meta: None,
            extra: BTreeMap::new(),
        },
        McpElicitExtResponse::Decline => McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Decline,
            content: None,
            meta: None,
            extra: BTreeMap::new(),
        },
        McpElicitExtResponse::Cancel => McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Cancel,
            content: None,
            meta: None,
            extra: BTreeMap::new(),
        },
    }
}

fn text_update(
    constructor: fn(acp::ContentChunk) -> acp::SessionUpdate,
    text: String,
) -> acp::SessionUpdate {
    constructor(acp::ContentChunk::new(acp::ContentBlock::Text(
        acp::TextContent::new(text),
    )))
}

fn codex_session_list_payload(threads: Vec<Thread>) -> Value {
    let sessions = threads
        .into_iter()
        .filter_map(|thread| {
            let presentation = codex_thread_presentation(&thread)?;
            Some(json!({
                "sessionId": thread.id,
                "cwd": thread.cwd.unwrap_or_default(),
                "summary": presentation.summary,
                "firstPrompt": presentation.preview,
                "source": "provider",
                "createdAt": presentation.created_at,
                "updatedAt": presentation.updated_at,
                "modelId": thread.model,
                "numMessages": thread.turns.len(),
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "sessions": sessions,
        "_meta": { "x.ai/listScope": "cwd" },
    })
}

fn codex_session_search_payload(matches: Vec<ThreadSearchResult>) -> Value {
    let results = matches
        .into_iter()
        .filter_map(|matched| {
            let thread = matched.thread;
            let presentation = codex_thread_presentation(&thread)?;
            Some(json!({
                "sessionId": thread.id,
                "cwd": thread.cwd.unwrap_or_default(),
                "summary": presentation.summary,
                "updatedAt": presentation.updated_at,
                "score": 1.0,
                "matchedFields": ["content"],
                "snippet": matched.snippet,
            }))
        })
        .collect::<Vec<_>>();
    let total_estimate = results.len();
    json!({
        "results": results,
        "nextOffset": null,
        "totalEstimate": total_estimate,
        "bootstrapping": false,
    })
}

struct CodexThreadPresentation {
    summary: String,
    preview: String,
    created_at: String,
    updated_at: String,
}

fn codex_thread_presentation(thread: &Thread) -> Option<CodexThreadPresentation> {
    let updated_at = timestamp_rfc3339(thread.updated_at.or(thread.created_at)?)?;
    let created_at = thread
        .created_at
        .and_then(timestamp_rfc3339)
        .unwrap_or_else(|| updated_at.clone());
    let preview = thread.preview.as_deref().unwrap_or_default().trim();
    let summary = thread
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map(str::trim)
        .unwrap_or_else(|| preview.lines().next().unwrap_or_default())
        .chars()
        .take(160)
        .collect::<String>();
    Some(CodexThreadPresentation {
        summary,
        preview: preview.to_owned(),
        created_at,
        updated_at,
    })
}

fn codex_mcp_list_payload(servers: Vec<McpServerStatus>) -> Value {
    let servers = servers
        .into_iter()
        .map(|server| {
            let auth_required = server.auth_status == McpAuthStatus::NotLoggedIn
                || server.runtime_status == Some(McpServerConnectionStatus::AuthenticationRequired);
            let enabled = server.runtime_status != Some(McpServerConnectionStatus::Disabled);
            let tools = server
                .tools
                .into_values()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "displayName": tool.title,
                        "description": tool.description,
                        "enabled": true,
                    })
                })
                .collect::<Vec<_>>();
            let status = match server.runtime_status {
                Some(McpServerConnectionStatus::Connected) => "ready",
                Some(McpServerConnectionStatus::Starting) => "initializing",
                None if !tools.is_empty() => "ready",
                _ => "unavailable",
            };
            let source_label = server
                .plugin_id
                .as_deref()
                .map(|plugin_id| format!("plugin: {plugin_id}"));
            let display_name = server
                .server_info
                .and_then(|info| info.title)
                .filter(|title| !title.trim().is_empty());
            json!({
                "name": server.name,
                "displayName": display_name,
                "source": "local",
                "sourceLabel": source_label,
                "session": {
                    "enabled": enabled,
                    "status": status,
                    "tools": tools,
                    "authRequired": auth_required,
                    "setupRequired": false,
                },
            })
        })
        .collect::<Vec<_>>();
    json!({ "servers": servers })
}

fn codex_hooks_list_payload(
    response: bot_provider_codex::HooksListResponse,
) -> xai_hooks_plugins_types::HooksListResponse {
    let mut hooks = Vec::new();
    let mut load_errors = Vec::new();
    let mut project_trusted = true;
    for entry in response.data {
        for error in entry.errors {
            load_errors.push(format!("{}: {}", error.path.display(), error.message));
        }
        load_errors.extend(entry.warnings);
        for hook in entry.hooks {
            if hook.source == CodexHookSource::Project
                && !matches!(
                    hook.trust_status,
                    CodexHookTrustStatus::Managed | CodexHookTrustStatus::Trusted
                )
            {
                project_trusted = false;
            }
            hooks.push(codex_hook_info(hook));
        }
    }
    xai_hooks_plugins_types::HooksListResponse {
        hooks,
        project_trusted,
        load_errors,
        actions_supported: false,
    }
}

fn codex_hook_info(hook: CodexHookMetadata) -> xai_hooks_plugins_types::HookInfo {
    use xai_hooks_plugins_types::{HookEvent, HookHandlerType, HookInfo};

    let event = match hook.event_name {
        CodexHookEventName::PreToolUse => HookEvent::PreToolUse,
        CodexHookEventName::PermissionRequest => HookEvent::PermissionRequest,
        CodexHookEventName::PostToolUse => HookEvent::PostToolUse,
        CodexHookEventName::PreCompact => HookEvent::PreCompact,
        CodexHookEventName::PostCompact => HookEvent::PostCompact,
        CodexHookEventName::SessionStart => HookEvent::SessionStart,
        CodexHookEventName::SessionEnd => HookEvent::SessionEnd,
        CodexHookEventName::UserPromptSubmit => HookEvent::UserPromptSubmit,
        CodexHookEventName::SubagentStart => HookEvent::SubagentStart,
        CodexHookEventName::SubagentStop => HookEvent::SubagentStop,
        CodexHookEventName::Stop => HookEvent::Stop,
        CodexHookEventName::Interrupt => HookEvent::Interrupt,
        CodexHookEventName::Unknown => HookEvent::Unknown,
    };
    let (handler_type, command) = match hook.handler {
        CodexHookHandlerMetadata::Command { command, .. } => {
            (HookHandlerType::Command, Some(command))
        }
        CodexHookHandlerMetadata::McpTool { server, tool } => (
            HookHandlerType::McpTool,
            Some(format!("MCP tool: {server}.{tool}")),
        ),
        CodexHookHandlerMetadata::Prompt => {
            (HookHandlerType::Prompt, Some("Prompt hook".to_owned()))
        }
        CodexHookHandlerMetadata::Agent => (HookHandlerType::Agent, Some("Agent hook".to_owned())),
        CodexHookHandlerMetadata::Unknown => (HookHandlerType::Unknown, None),
    };
    let source_path = hook.source_path;
    let source_dir = source_path
        .parent()
        .unwrap_or(source_path.as_path())
        .display()
        .to_string();
    HookInfo {
        name: hook.key,
        event,
        handler_type,
        matcher: hook.matcher,
        command,
        url: None,
        timeout_ms: hook.timeout_sec.saturating_mul(1_000),
        source_dir,
        disabled: !hook.enabled,
        pinned: hook.is_managed,
        removable: false,
    }
}

fn codex_skill_info(
    skill: CodexSkillMetadata,
) -> xai_grok_tools::implementations::skills::types::SkillInfo {
    use xai_grok_tools::implementations::skills::types::{SkillInfo, SkillScope};

    let display_name = skill
        .interface
        .as_ref()
        .and_then(|interface| interface.display_name.clone());
    let short_description = skill
        .interface
        .as_ref()
        .and_then(|interface| interface.short_description.clone())
        .or(skill.short_description);
    let scope = if skill.plugin_id.is_some() {
        SkillScope::Plugin
    } else {
        match skill.scope {
            CodexSkillScope::Repo => SkillScope::Repo,
            CodexSkillScope::User => SkillScope::User,
            CodexSkillScope::System => SkillScope::Bundled,
            CodexSkillScope::Admin => SkillScope::Server,
            CodexSkillScope::Unknown => SkillScope::User,
        }
    };
    let has_user_specified_description = !skill.description.trim().is_empty();
    SkillInfo {
        name: skill.name,
        display_name,
        description: skill.description,
        has_user_specified_description,
        paths: None,
        when_to_use: None,
        short_description,
        author: None,
        argument_hint: None,
        license: None,
        compatibility: None,
        metadata: None,
        path: skill.path.display().to_string(),
        scope,
        config_source: None,
        plugin_name: skill.plugin_id,
        plugin_version: None,
        plugin_root: None,
        plugin_data: None,
        allowed_tools: None,
        model: None,
        effort: None,
        user_invocable: true,
        disable_model_invocation: false,
        enabled: skill.enabled,
        body: None,
    }
}

fn codex_skill_commands(
    skills: Vec<xai_grok_tools::implementations::skills::types::SkillInfo>,
) -> Vec<acp::AvailableCommand> {
    let mut reserved: HashSet<String> = crate::slash::commands::builtin_commands()
        .into_iter()
        .flat_map(|command| {
            let mut names = Vec::with_capacity(command.aliases().len() + 1);
            names.push(command.name().to_ascii_lowercase());
            names.extend(
                command
                    .aliases()
                    .iter()
                    .map(|alias| alias.to_ascii_lowercase()),
            );
            names
        })
        .collect();
    reserved.extend(
        crate::slash::registry::BLOCKED_ACP_NAMES
            .iter()
            .map(|name| name.to_string()),
    );
    let mut skills: Vec<_> = skills
        .into_iter()
        .filter(|skill| {
            skill.enabled
                && skill.user_invocable
                && !skill.name.trim().is_empty()
                && !skill.path.trim().is_empty()
        })
        .collect();
    skills.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut bare_counts = HashMap::new();
    for skill in &skills {
        *bare_counts
            .entry(skill.name.to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    let mut claimed = HashSet::new();
    skills
        .into_iter()
        .filter_map(|skill| {
            let bare_key = skill.name.to_ascii_lowercase();
            let owner = skill
                .plugin_name
                .as_deref()
                .unwrap_or_else(|| skill.scope.as_ref())
                .to_owned();
            let ownership =
                CommandOwnership::current("codex", owner.clone(), CommandKind::Skill).ok()?;
            let mut command_name =
                if bare_counts.get(&bare_key) == Some(&1) && !reserved.contains(&bare_key) {
                    skill.name.clone()
                } else {
                    format!("{owner}:{}", skill.name)
                };
            let command_key = command_name.to_ascii_lowercase();
            if reserved.contains(&command_key) || !claimed.insert(command_key) {
                command_name =
                    format!("{owner}:{}:{:08x}", skill.name, stable_path_id(&skill.path));
                claimed.insert(command_name.to_ascii_lowercase());
            }
            let mut meta = Map::new();
            meta.insert(
                "botOwnership".to_owned(),
                serde_json::to_value(ownership).ok()?,
            );
            meta.insert("scope".to_owned(), json!(skill.scope));
            meta.insert("path".to_owned(), json!(skill.path));
            meta.insert("bareName".to_owned(), json!(skill.name));
            meta.insert("qualifiedName".to_owned(), json!(command_name));
            meta.insert("provider".to_owned(), json!("codex"));
            meta.insert("owner".to_owned(), json!(owner));
            if let Some(plugin_name) = skill.plugin_name {
                meta.insert("pluginName".to_owned(), json!(plugin_name));
            }
            Some(
                acp::AvailableCommand::new(
                    command_name,
                    skill
                        .short_description
                        .filter(|description| !description.trim().is_empty())
                        .unwrap_or(skill.description),
                )
                .meta(meta),
            )
        })
        .collect()
}

fn stable_path_id(path: &str) -> u32 {
    path.as_bytes().iter().fold(0x811c9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    })
}

fn codex_plugins_list_payload(marketplaces: Vec<PluginMarketplace>, cwd: &Path) -> Value {
    let mut plugins = Vec::new();
    for marketplace in marketplaces {
        for plugin in marketplace
            .plugins
            .into_iter()
            .filter(|plugin| plugin.installed)
        {
            let root = match &plugin.source {
                PluginSource::Local { path } => path.display().to_string(),
                PluginSource::Git { url } => url.clone(),
                PluginSource::Npm { package } => package.clone(),
                PluginSource::Remote | PluginSource::Unknown => marketplace
                    .path
                    .as_ref()
                    .map(|path| path.join(&plugin.name).display().to_string())
                    .unwrap_or_default(),
            };
            let scope = if !root.is_empty() && Path::new(&root).starts_with(cwd) {
                "project"
            } else {
                "user"
            };
            let git_url = match &plugin.source {
                PluginSource::Git { url } => Some(url.clone()),
                _ => None,
            };
            let display_name = plugin
                .interface
                .as_ref()
                .and_then(|interface| interface.display_name.as_deref())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&plugin.name)
                .to_owned();
            let description = plugin.interface.as_ref().and_then(|interface| {
                interface
                    .short_description
                    .clone()
                    .or_else(|| interface.long_description.clone())
            });
            plugins.push(json!({
                "name": display_name,
                "id": plugin.id,
                "root": root,
                "scope": scope,
                "trusted": true,
                "enabled": plugin.enabled,
                "version": plugin.local_version.or(plugin.version),
                "description": description,
                "skillCount": 0,
                "agentCount": 0,
                "hookStatus": "none",
                "hookCount": 0,
                "mcpServerCount": 0,
                "mcpStatus": "none",
                "marketplaceSource": marketplace.name.clone(),
                "origin": {
                    "type": "marketplace_install",
                    "source_name": marketplace.name.clone(),
                    "git_url": git_url,
                },
            }));
        }
    }
    json!({ "plugins": plugins })
}

fn codex_reconcile_outcome(value: Value) -> xai_hooks_plugins_types::ActionOutcome {
    use xai_hooks_plugins_types::{ActionOutcome, OutcomeStatus};

    let changed = value
        .get("changedPlugins")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let failed = value
        .get("failedRemotePluginIds")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
        + value
            .get("failedMaterializationRemotePluginIds")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
    if failed == 0 {
        ActionOutcome {
            status: OutcomeStatus::Success,
            message: format!("Reloaded Codex plugins. {changed} plugin change(s) found."),
            requires_reload: false,
            requires_restart: false,
        }
    } else {
        ActionOutcome {
            status: OutcomeStatus::InternalError,
            message: format!(
                "Codex reloaded plugins with {failed} remote failure(s). {changed} plugin change(s) were applied."
            ),
            requires_reload: false,
            requires_restart: false,
        }
    }
}

fn codex_plugin_error(
    action: &str,
    error: bot_provider_codex::CodexTransportError,
) -> xai_hooks_plugins_types::ActionOutcome {
    xai_hooks_plugins_types::ActionOutcome {
        status: xai_hooks_plugins_types::OutcomeStatus::InternalError,
        message: format!("Codex could not {action}: {error}"),
        requires_reload: false,
        requires_restart: false,
    }
}

fn timestamp_rfc3339(timestamp: i64) -> Option<String> {
    let seconds = if timestamp.abs() >= 10_000_000_000 {
        timestamp / 1_000
    } else {
        timestamp
    };
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn codex_rewind_points(turns: &[Turn]) -> Vec<CodexRewindPoint> {
    turns
        .iter()
        .filter_map(|turn| {
            let message = turn
                .items
                .iter()
                .find(|item| item.get("type").and_then(Value::as_str) == Some("userMessage"))?;
            let text = message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|content| content.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            let text = (!text.trim().is_empty()).then_some(text);
            let created_at = turn
                .extra
                .get("createdAt")
                .or_else(|| turn.extra.get("startedAt"))
                .and_then(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| value.as_i64().and_then(timestamp_rfc3339))
                })
                .unwrap_or_default();
            Some((turn, text, created_at))
        })
        .enumerate()
        .map(
            |(prompt_index, (turn, prompt_text, created_at))| CodexRewindPoint {
                turn_id: turn.id.clone(),
                prompt_index,
                prompt_preview: prompt_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("Image prompt")
                    .to_owned(),
                prompt_text,
                created_at,
                has_file_changes: turn
                    .items
                    .iter()
                    .any(|item| item.get("type").and_then(Value::as_str) == Some("fileChange")),
            },
        )
        .collect()
}

fn replay_updates(session_id: &acp::SessionId, turns: &[Turn]) -> Vec<acp::SessionUpdate> {
    turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .flat_map(|item| replay_item(session_id, item))
        .collect()
}

fn replay_turn_completion(
    session_id: &acp::SessionId,
    turn: &Turn,
) -> serde_json::Result<Option<acp::ExtNotification>> {
    let stop_reason = match turn.status.as_str() {
        "completed" => "end_turn",
        "cancelled" | "interrupted" => "cancelled",
        "failed" => "error",
        _ => return Ok(None),
    };
    let payload = XaiSessionNotification {
        session_id: session_id.clone(),
        update: XaiSessionUpdate::TurnCompleted {
            prompt_id: turn.id.clone(),
            stop_reason: stop_reason.to_owned(),
            agent_result: None,
            error_kind: None,
            usage: None,
            elapsed_ms: None,
        },
        meta: Some(json!({ "isReplay": true })),
    };
    let raw = serde_json::value::to_raw_value(&payload)?;
    Ok(Some(acp::ExtNotification::new(
        "x.ai/session/update",
        raw.into(),
    )))
}

fn replay_item(session_id: &acp::SessionId, item: &Value) -> Vec<acp::SessionUpdate> {
    match item.get("type").and_then(Value::as_str) {
        Some("userMessage") => item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(replay_user_content)
            .map(acp::SessionUpdate::UserMessageChunk)
            .collect(),
        Some("reasoning") => item
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| text_update(acp::SessionUpdate::AgentThoughtChunk, text.to_owned()))
            .collect(),
        Some("agentMessage" | "plan") => item
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| text_update(acp::SessionUpdate::AgentMessageChunk, text.to_owned()))
            .into_iter()
            .collect(),
        Some("fileChange") => file_change_calls(
            session_id,
            item.get("id")
                .and_then(Value::as_str)
                .unwrap_or("history-file-change"),
            item.get("changes"),
            item.get("status").and_then(Value::as_str),
        )
        .into_iter()
        .map(|(_, call)| acp::SessionUpdate::ToolCall(call))
        .collect(),
        Some(_) => historical_tool_call(item)
            .map(|call| {
                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("history-tool");
                let tool = history_tool_call(codex_tool_id(session_id, item_id), &call, item);
                acp::SessionUpdate::ToolCall(tool)
            })
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

fn replay_user_content(content: &Value) -> Option<acp::ContentChunk> {
    let block = match content.get("type").and_then(Value::as_str)? {
        "text" => acp::ContentBlock::Text(acp::TextContent::new(
            content.get("text").and_then(Value::as_str)?.to_owned(),
        )),
        "image" => acp::ContentBlock::Image(
            acp::ImageContent::new("", "image/png").uri(
                content
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ),
        ),
        "localImage" => {
            let path = content.get("path").and_then(Value::as_str)?;
            let uri = url::Url::from_file_path(path)
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_else(|| path.to_owned());
            acp::ContentBlock::Image(acp::ImageContent::new("", image_mime_type(path)).uri(uri))
        }
        _ => return None,
    };
    Some(acp::ContentChunk::new(block))
}

fn protocol_command_completion_call(
    session_id: &acp::SessionId,
    notification: &bot_provider_codex::ServerNotification,
) -> Option<(String, acp::ToolCall)> {
    if notification.method != "item/completed" {
        return None;
    }
    let item = notification.params.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("commandExecution") {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?;
    let call = historical_tool_call(item)?;
    let tool_id = codex_tool_id(session_id, item_id);
    Some((tool_id.clone(), history_tool_call(tool_id, &call, item)))
}

fn image_mime_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("tif" | "tiff") => "image/tiff",
        _ => "image/png",
    }
}

fn history_tool_call(id: String, call: &bot_core::ToolCall, item: &Value) -> acp::ToolCall {
    let mut tool = tool_call(id, call).raw_input(item.clone());
    if item.get("type").and_then(Value::as_str) == Some("commandExecution") {
        let command = item
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = item
            .get("aggregatedOutput")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let exit_code = item
            .get("exitCode")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| {
                if call.state == ToolCallState::Completed {
                    0
                } else {
                    1
                }
            });
        tool = tool
            .raw_input(json!({
                "command": command,
                "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
            }))
            .raw_output(json!({
                "type": "Bash",
                "output": output.as_bytes(),
                "output_for_prompt": output,
                "exit_code": exit_code,
                "command": command,
                "truncated": false,
                "signal": null,
                "timed_out": false,
                "description": null,
                "current_dir": item.get("cwd").and_then(Value::as_str).unwrap_or_default(),
                "output_file": "",
                "total_bytes": output.len(),
                "output_delta": null,
                "was_bare_echo": false,
            }));
    }
    tool
}

fn tool_call(id: String, call: &bot_core::ToolCall) -> acp::ToolCall {
    let mut tool = acp::ToolCall::new(id, call.title.clone())
        .kind(tool_kind(call.kind))
        .status(tool_status(call.state));
    let content = tool_content(call);
    if !content.is_empty() {
        tool = tool.content(content);
    }
    if call.kind == ToolCallKind::Command {
        if let Some(input) = command_raw_input(call) {
            tool = tool.raw_input(input);
        }
        if let Some(output) = command_raw_output(call) {
            tool = tool.raw_output(output);
        }
    } else if let Some(output) = call.output.as_ref() {
        tool = tool.raw_output(json!(output));
    }
    tool
}

fn tool_call_update(id: String, call: &bot_core::ToolCall) -> acp::ToolCallUpdate {
    let mut fields = acp::ToolCallUpdateFields::new()
        .title(call.title.clone())
        .kind(tool_kind(call.kind))
        .status(tool_status(call.state));
    let content = tool_content(call);
    if !content.is_empty() {
        fields = fields.content(content);
    }
    if call.kind == ToolCallKind::Command {
        fields = fields.raw_input(command_raw_input(call));
        fields = fields.raw_output(command_raw_output(call));
    } else if let Some(output) = call.output.as_ref() {
        fields = fields.raw_output(json!(output));
    }
    acp::ToolCallUpdate::new(id, fields)
}

fn command_raw_input(call: &bot_core::ToolCall) -> Option<Value> {
    let command = command_from_call(call)?;
    Some(json!({"command": command}))
}

fn command_raw_output(call: &bot_core::ToolCall) -> Option<Value> {
    let command = command_from_call(call)?;
    let output = call.output.as_deref().unwrap_or_default();
    if output.is_empty()
        && !matches!(
            call.state,
            ToolCallState::Completed | ToolCallState::Cancelled | ToolCallState::Failed
        )
    {
        return None;
    }
    let exit_code = if matches!(call.state, ToolCallState::Cancelled | ToolCallState::Failed) {
        1
    } else {
        0
    };
    serde_json::to_value(ToolOutput::Bash(BashOutput {
        output: output.as_bytes().to_vec(),
        output_for_prompt: BashOutput::make_output_for_prompt(output),
        exit_code,
        command,
        truncated: false,
        signal: None,
        timed_out: false,
        description: None,
        current_dir: String::new(),
        output_file: String::new(),
        total_bytes: output.len(),
        output_delta: None,
        was_bare_echo: false,
    }))
    .ok()
}

fn command_from_call(call: &bot_core::ToolCall) -> Option<String> {
    call.detail
        .as_deref()
        .and_then(|detail| detail.strip_prefix("$ "))
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            (call.title != "Run command" && !call.title.trim().is_empty())
                .then(|| call.title.clone())
        })
}

fn tool_content(call: &bot_core::ToolCall) -> Vec<acp::ToolCallContent> {
    [call.detail.as_deref(), call.output.as_deref()]
        .into_iter()
        .flatten()
        .filter(|text| !text.is_empty())
        .map(|text| {
            acp::ToolCallContent::from(acp::ContentBlock::Text(acp::TextContent::new(text)))
        })
        .collect()
}

fn tool_kind(kind: ToolCallKind) -> acp::ToolKind {
    match kind {
        ToolCallKind::Command => acp::ToolKind::Execute,
        ToolCallKind::FileChange => acp::ToolKind::Edit,
        ToolCallKind::WebSearch => acp::ToolKind::Search,
        ToolCallKind::Mcp | ToolCallKind::Image => acp::ToolKind::Fetch,
        ToolCallKind::Wait => acp::ToolKind::Think,
        ToolCallKind::Collaboration | ToolCallKind::Other => acp::ToolKind::Other,
    }
}

fn tool_status(state: ToolCallState) -> acp::ToolCallStatus {
    match state {
        ToolCallState::Pending => acp::ToolCallStatus::Pending,
        ToolCallState::Running => acp::ToolCallStatus::InProgress,
        ToolCallState::Completed => acp::ToolCallStatus::Completed,
        ToolCallState::Cancelled | ToolCallState::Failed => acp::ToolCallStatus::Failed,
    }
}

fn codex_tool_id(session_id: &acp::SessionId, item_id: &str) -> String {
    format!("{}:codex:item:{item_id}", session_id.0)
}

fn codex_question_answers(
    response: AskUserQuestionExtResponse,
    answer_ids: &HashMap<String, String>,
) -> BTreeMap<String, ToolRequestUserInputAnswer> {
    let mut result = BTreeMap::new();
    match response {
        AskUserQuestionExtResponse::Accepted {
            answers,
            annotations,
        } => {
            for (question, mut values) in answers {
                let Some(id) = answer_ids.get(&question) else {
                    continue;
                };
                if let Some(notes) = annotations
                    .as_ref()
                    .and_then(|items| items.get(&question))
                    .and_then(|annotation: &QuestionAnnotation| annotation.notes.as_ref())
                    .filter(|notes| !notes.trim().is_empty())
                {
                    values.retain(|value| value != "Other");
                    values.push(format!("user_note: {notes}"));
                }
                result.insert(id.clone(), ToolRequestUserInputAnswer { answers: values });
            }
        }
        AskUserQuestionExtResponse::ChatAboutThis { partial_answers }
        | AskUserQuestionExtResponse::SkipInterview { partial_answers } => {
            for (question, answer) in partial_answers {
                if let Some(id) = answer_ids.get(&question) {
                    result.insert(
                        id.clone(),
                        ToolRequestUserInputAnswer {
                            answers: vec![answer],
                        },
                    );
                }
            }
        }
        AskUserQuestionExtResponse::Cancelled => {}
    }
    result
}

fn acp_error(message: impl Into<String>) -> acp::Error {
    acp::Error::internal_error().data(message.into())
}

fn codex_dynamic_tools(
    meta: Option<&acp::Meta>,
) -> Result<Option<Vec<DynamicToolSpec>>, acp::Error> {
    let Some(value) = meta.and_then(|meta| meta.get(CODEX_DYNAMIC_TOOLS_META_KEY)) else {
        return Ok(None);
    };
    let tools: Vec<DynamicToolSpec> = serde_json::from_value(value.clone())
        .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
    Ok((!tools.is_empty()).then_some(tools))
}

fn dynamic_tool_failure(message: &str) -> DynamicToolCallResponse {
    DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::Text {
            text: message.to_owned(),
        }],
        success: false,
    }
}

#[derive(Clone, Copy)]
enum PermissionDecision {
    AllowOnce,
    AllowAlways,
    Reject,
    Cancel,
}

fn legacy_approval_response(decision: PermissionDecision) -> Value {
    match decision {
        PermissionDecision::AllowOnce => json!({ "decision": "approved" }),
        PermissionDecision::AllowAlways => json!({ "decision": "approved_for_session" }),
        PermissionDecision::Reject => {
            json!({ "decision": { "denied": { "rejection": "User declined" } } })
        }
        PermissionDecision::Cancel => json!({ "decision": "abort" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::Agent as _;
    use bot_provider_codex::{InputModality, ReasoningEffortOption};
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::collections::VecDeque;

    #[cfg(unix)]
    fn codex_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../xai-grok-pager-bin/tests/fixtures/codex")
    }

    #[cfg(unix)]
    fn fixture_turn(directory: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(directory.join("last-turn.json")).unwrap()).unwrap()
    }

    #[cfg(unix)]
    async fn set_permission_mode(agent: &CodexAcpAgent, session_id: &acp::SessionId, mode: &str) {
        let params = json!({
            "permission_mode": mode,
            "sessionId": session_id.0.to_string(),
        });
        agent
            .ext_notification(acp::ExtNotification::new(
                "x.ai/yolo_mode_changed",
                serde_json::value::to_raw_value(&params).unwrap().into(),
            ))
            .await
            .unwrap();
    }

    #[cfg(unix)]
    async fn prompt_fixture(
        agent: &CodexAcpAgent,
        session_id: &acp::SessionId,
        text: &str,
    ) -> acp::PromptResponse {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            agent.prompt(acp::PromptRequest::new(
                session_id.clone(),
                vec![acp::ContentBlock::Text(acp::TextContent::new(text))],
            )),
        )
        .await
        .unwrap()
        .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bridges_codex_dynamic_tool_calls_to_the_acp_client() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtMethod(args) => {
                                assert_eq!(
                                    args.request.method.as_ref(),
                                    CODEX_DYNAMIC_TOOL_CALL_METHOD
                                );
                                let request: DynamicToolCallParams =
                                    serde_json::from_str(args.request.params.get()).unwrap();
                                assert_eq!(request.thread_id, "bot-fixture-thread");
                                assert_eq!(request.turn_id, "bot-fixture-turn");
                                assert_eq!(request.call_id, "dynamic-fixture");
                                assert_eq!(request.tool, "lookup");
                                assert_eq!(request.namespace.as_deref(), Some("records"));
                                assert_eq!(request.arguments, json!({"key": "alpha"}));
                                assert_eq!(request.extra["futureField"], json!({"kept": true}));
                                let response = DynamicToolCallResponse {
                                    success: true,
                                    content_items: vec![DynamicToolCallOutputContentItem::Text {
                                        text: "record alpha".to_owned(),
                                    }],
                                };
                                let raw = serde_json::value::to_raw_value(&response).unwrap();
                                let _ =
                                    args.response_tx.send(Ok(acp::ExtResponse::new(raw.into())));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let tools = vec![DynamicToolSpec::Function {
                    name: "lookup".to_owned(),
                    description: "Look up a record".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {"key": {"type": "string"}},
                        "required": ["key"]
                    }),
                    defer_loading: Some(false),
                }];
                let mut meta = acp::Meta::new();
                meta.insert(
                    CODEX_DYNAMIC_TOOLS_META_KEY.to_owned(),
                    serde_json::to_value(&tools).unwrap(),
                );
                let session_id = agent
                    .new_session(
                        acp::NewSessionRequest::new(directory.path().to_path_buf())
                            .meta(Some(meta)),
                    )
                    .await
                    .unwrap()
                    .session_id;
                let response = prompt_fixture(&agent, &session_id, "DYNAMIC_TOOL").await;
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);

                let start: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-start.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(start["dynamicTools"], serde_json::to_value(tools).unwrap());
                let response: DynamicToolCallResponse = serde_json::from_slice(
                    &std::fs::read(directory.path().join("dynamic-tool-response.json")).unwrap(),
                )
                .unwrap();
                assert!(response.success);
                assert_eq!(
                    response.content_items,
                    vec![DynamicToolCallOutputContentItem::Text {
                        text: "record alpha".to_owned()
                    }]
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[test]
    fn builds_a_device_login_url_with_a_visible_code() {
        assert_eq!(
            device_auth_url("https://example.com/device", "ABCD-EFGH"),
            "https://example.com/device?user_code=ABCD-EFGH"
        );
        assert_eq!(
            device_auth_url("https://example.com/device?client=bot", "AB CD"),
            "https://example.com/device?client=bot&user_code=AB+CD"
        );
    }

    #[test]
    fn prefers_device_login_without_a_local_browser() {
        let remote = codex_interactive_auth_methods(false);
        assert_eq!(remote[0].id().0.as_ref(), CODEX_DEVICE_AUTH_METHOD);
        assert_eq!(remote[1].id().0.as_ref(), CODEX_CHATGPT_AUTH_METHOD);

        let local = codex_interactive_auth_methods(true);
        assert_eq!(local[0].id().0.as_ref(), CODEX_CHATGPT_AUTH_METHOD);
        assert_eq!(local[1].id().0.as_ref(), CODEX_DEVICE_AUTH_METHOD);
    }

    #[test]
    fn maps_codex_account_windows_to_provider_usage() {
        let response: AccountRateLimitsResponse = serde_json::from_value(json!({
            "rateLimits": {
                "limitId": "codex",
                "limitName": "Codex",
                "normalModelSlug": "gpt-5.6-sol",
                "primary": {
                    "usedPercent": 37,
                    "windowDurationMins": 300,
                    "resetsAt": 1_789_238_400
                },
                "secondary": {
                    "usedPercent": 8,
                    "windowDurationMins": 10_080,
                    "resetsAt": 1_789_843_200
                }
            }
        }))
        .expect("rate limits");

        let usage = ProviderUsage {
            provider: CoreProviderId::Codex,
            lifetime_tokens: Some(1_234_567),
            limits: map_usage_limits(response),
            extensions: BTreeMap::new(),
        };
        let (limit, window) = usage.longest_window().expect("quota window");
        assert_eq!(limit.name, "Codex");
        assert_eq!(limit.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(window.label, "Secondary");
        assert_eq!(window.used_percent, 8.0);
        assert_eq!(window.duration_minutes, Some(10_080));
        assert_eq!(window.remaining_percent(), 92.0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn publishes_codex_provider_usage_during_a_turn() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let (usage_tx, mut usage_rx) = tokio::sync::mpsc::unbounded_channel();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                if args.request.method.as_ref() == PROVIDER_USAGE_UPDATED_METHOD {
                                    let update: ProviderUsageUpdate =
                                        serde_json::from_str(args.request.params.get()).unwrap();
                                    let _ = usage_tx.send(update);
                                }
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;

                let response = prompt_fixture(&agent, &session_id, "USAGE").await;
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                let update = usage_rx.recv().await.expect("provider usage update");
                assert_eq!(update.session_id, session_id.0.as_ref());
                assert_eq!(update.usage.provider, CoreProviderId::Codex);
                assert_eq!(update.usage.lifetime_tokens, Some(1_234_567));
                let (_, window) = update.usage.longest_window().expect("quota window");
                assert_eq!(window.duration_minutes, Some(10_080));
                assert_eq!(window.remaining_percent(), 92.0);

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn advertises_and_completes_codex_account_login() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_channel, agent_channel) = acp_channels();
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let initialized = agent
                    .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::V1))
                    .await
                    .unwrap();
                assert_eq!(initialized.auth_methods.len(), 3);
                assert_eq!(
                    initialized.auth_methods[0].id().0.as_ref(),
                    CODEX_CACHED_AUTH_METHOD
                );
                let interactive_ids = initialized.auth_methods[1..]
                    .iter()
                    .map(|method| method.id().0.as_ref())
                    .collect::<HashSet<_>>();
                assert_eq!(
                    interactive_ids,
                    HashSet::from([CODEX_CHATGPT_AUTH_METHOD, CODEX_DEVICE_AUTH_METHOD])
                );

                agent.pending_login.replace(Some(CodexPendingLogin {
                    login_id: "cancel-fixture".to_owned(),
                    auth_url: "https://example.com/cancel".to_owned(),
                    mode: "device",
                    request_seq: Some(9),
                }));
                assert!(!agent.cancel_pending_login(Some(8)).await.unwrap());
                assert!(agent.cancel_pending_login(Some(9)).await.unwrap());

                let authenticate = agent.authenticate(
                    acp::AuthenticateRequest::new(CODEX_DEVICE_AUTH_METHOD).meta(Some(
                        json!({"request_seq": 9}).as_object().cloned().unwrap(),
                    )),
                );
                let read_url = async {
                    loop {
                        let response = agent
                            .ext_method(acp::ExtRequest::new(
                                AUTH_GET_URL_METHOD,
                                serde_json::value::to_raw_value(&json!({})).unwrap().into(),
                            ))
                            .await
                            .unwrap();
                        let value: Value = serde_json::from_str(response.0.get()).unwrap();
                        if value["auth_url"].is_string() {
                            break value;
                        }
                        tokio::task::yield_now().await;
                    }
                };
                let (authenticated, auth_url) = tokio::join!(authenticate, read_url);
                authenticated.unwrap();
                assert_eq!(auth_url["mode"], "device");
                assert_eq!(
                    auth_url["auth_url"],
                    "https://example.com/device?user_code=ABCD-EFGH"
                );

                let status_response = agent
                    .ext_method(acp::ExtRequest::new(
                        ACCOUNT_STATUS_METHOD,
                        serde_json::value::to_raw_value(&json!({})).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let status: ProviderAccountStatus =
                    serde_json::from_str(status_response.0.get()).unwrap();
                assert!(status.signed_in);
                assert_eq!(status.email.as_deref(), Some("bot@example.com"));
                assert_eq!(status.plan.as_deref(), Some("Pro"));
                assert_eq!(
                    status.rate_limits[0].primary.as_ref().unwrap().used_percent,
                    37
                );

                agent
                    .ext_method(acp::ExtRequest::new(
                        AUTH_LOGOUT_METHOD,
                        serde_json::value::to_raw_value(&json!({})).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                assert!(!agent.is_signed_in());
                assert_eq!(agent.auth_methods().len(), 2);
                let signed_out = agent.account_status().await.unwrap();
                assert!(!signed_out.signed_in);
                assert!(signed_out.rate_limits.is_empty());
                agent.client.close().await.unwrap();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn compacts_a_codex_thread_with_the_native_method() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0.to_string()});
                agent
                    .ext_method(acp::ExtRequest::new(
                        COMPACT_CONVERSATION_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let compact: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-compact.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(compact, json!({"threadId": session_id.0.to_string()}));

                let params = json!({
                    "sessionId": session_id.0.to_string(),
                    "userContext": "Keep authentication details",
                });
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            COMPACT_CONVERSATION_METHOD,
                            serde_json::value::to_raw_value(&params).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn steers_an_active_codex_turn_with_the_native_method() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let notifications = Rc::new(RefCell::new(Vec::new()));
                let received = notifications.clone();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                if args.request.method.as_ref() != PROVIDER_USAGE_UPDATED_METHOD {
                                    received.borrow_mut().push(json!({
                                        "method": args.request.method.as_ref(),
                                        "params": serde_json::from_str::<Value>(
                                            args.request.params.get()
                                        )
                                        .unwrap(),
                                    }));
                                }
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                agent
                    .sessions
                    .borrow_mut()
                    .get_mut(&session_id)
                    .unwrap()
                    .turn_state = CodexTurnState::Active {
                    turn_id: "bot-fixture-turn".to_owned(),
                };
                let params = json!({
                    "sessionId": session_id.0.to_string(),
                    "text": "Use the smaller test set.",
                    "interjectionId": "interjection-fixture",
                    "content": [{"type": "text", "text": "Use the smaller test set."}],
                });
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        INTERJECT_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(response.0.get()).unwrap(),
                    json!({"turnId": "bot-fixture-turn"})
                );
                let steer: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-turn-steer.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    steer,
                    json!({
                        "threadId": "bot-fixture-thread",
                        "expectedTurnId": "bot-fixture-turn",
                        "input": [{"type": "text", "text": "Use the smaller test set."}],
                        "clientUserMessageId": "interjection-fixture",
                    })
                );
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while notifications.borrow().is_empty() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(
                    notifications.borrow().as_slice(),
                    [json!({
                        "method": "x.ai/session/interjection",
                        "params": {
                            "sessionId": session_id.0.to_string(),
                            "text": "Use the smaller test set.",
                            "interjectionId": "interjection-fixture",
                        },
                    })]
                );

                agent
                    .sessions
                    .borrow_mut()
                    .get_mut(&session_id)
                    .unwrap()
                    .turn_state = CodexTurnState::Idle;
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            INTERJECT_METHOD,
                            serde_json::value::to_raw_value(&params).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rewinds_codex_history_with_the_native_revert_method() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;

                let points_params = json!({"sessionId": session_id.0.to_string()});
                let points = agent
                    .ext_method(acp::ExtRequest::new(
                        REWIND_POINTS_METHOD,
                        serde_json::value::to_raw_value(&points_params)
                            .unwrap()
                            .into(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(points.0.get()).unwrap(),
                    json!({
                        "rewindPoints": [
                            {
                                "promptIndex": 0,
                                "createdAt": "2026-09-14T00:00:00Z",
                                "numFileSnapshots": 0,
                                "promptPreview": "First prompt",
                                "hasFileChanges": false,
                            },
                            {
                                "promptIndex": 1,
                                "createdAt": "2026-09-14T00:01:00Z",
                                "numFileSnapshots": 0,
                                "promptPreview": "Second prompt",
                                "hasFileChanges": true,
                            }
                        ]
                    })
                );

                let execute_params = json!({
                    "sessionId": session_id.0.to_string(),
                    "targetPromptIndex": 1,
                });
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        REWIND_EXECUTE_METHOD,
                        serde_json::value::to_raw_value(&execute_params)
                            .unwrap()
                            .into(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(response.0.get()).unwrap(),
                    json!({
                        "success": true,
                        "targetPromptIndex": 1,
                        "revertedFiles": [],
                        "cleanFiles": [],
                        "conflicts": [],
                        "mode": "conversation_only",
                        "promptText": "Second prompt",
                    })
                );
                let revert: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-revert.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    revert,
                    json!({
                        "threadId": "bot-fixture-thread",
                        "beforeTurnId": "turn-2",
                    })
                );

                let invalid = json!({
                    "sessionId": session_id.0.to_string(),
                    "targetPromptIndex": 9,
                });
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            REWIND_EXECUTE_METHOD,
                            serde_json::value::to_raw_value(&invalid).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn renames_and_deletes_a_codex_thread_with_native_methods() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let rename = json!({
                    "sessionId": session_id.0.to_string(),
                    "title": " Focused work ",
                    "resetToAuto": false,
                });
                agent
                    .ext_method(acp::ExtRequest::new(
                        SESSION_RENAME_METHOD,
                        serde_json::value::to_raw_value(&rename).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let recorded_name: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-name.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    recorded_name,
                    json!({"threadId": session_id.0.to_string(), "name": "Focused work"})
                );

                let reset = json!({
                    "sessionId": session_id.0.to_string(),
                    "title": "",
                    "resetToAuto": true,
                });
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            SESSION_RENAME_METHOD,
                            serde_json::value::to_raw_value(&reset).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                assert!(agent.sessions.borrow().contains_key(&session_id));
                let delete = json!({"sessionId": session_id.0.to_string()});
                agent
                    .ext_method(acp::ExtRequest::new(
                        SESSION_DELETE_METHOD,
                        serde_json::value::to_raw_value(&delete).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let recorded_delete: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-delete.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    recorded_delete,
                    json!({"threadId": session_id.0.to_string()})
                );
                assert!(!agent.sessions.borrow().contains_key(&session_id));

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forks_and_loads_a_codex_thread_with_the_native_method() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let parent_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let fork = json!({
                    "sourceSessionId": parent_id.0.to_string(),
                    "sourceCwd": directory.path(),
                    "newCwd": directory.path(),
                    "sessionKind": "fork",
                });
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        SESSION_FORK_METHOD,
                        serde_json::value::to_raw_value(&fork).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let response: Value = serde_json::from_str(response.0.get()).unwrap();
                let child_id = response["newSessionId"].as_str().unwrap();
                assert_eq!(child_id, "bot-fixture-fork-2");
                let recorded_fork: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-fork.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    recorded_fork,
                    json!({
                        "threadId": parent_id.0.to_string(),
                        "cwd": directory.path(),
                    })
                );
                let child_id = acp::SessionId::new(child_id.to_owned());
                agent
                    .load_session(acp::LoadSessionRequest::new(
                        child_id.clone(),
                        directory.path().to_path_buf(),
                    ))
                    .await
                    .unwrap();
                assert!(agent.sessions.borrow().contains_key(&child_id));

                let custom_id = json!({
                    "sourceSessionId": parent_id.0.to_string(),
                    "newCwd": directory.path(),
                    "newSessionId": "client-selected",
                });
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            SESSION_FORK_METHOD,
                            serde_json::value::to_raw_value(&custom_id).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn searches_codex_threads_with_the_native_catalog() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap();
                let search = json!({
                    "query": "provider authentication",
                    "limit": 20,
                    "includeContent": true,
                    "headless": "exclude",
                });
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        SESSION_SEARCH_METHOD,
                        serde_json::value::to_raw_value(&search).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let response: Value = serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(
                    response["results"][0]["sessionId"],
                    "bot-fixture-search-thread"
                );
                assert_eq!(response["results"][0]["summary"], "Provider authentication");
                assert_eq!(
                    response["results"][0]["snippet"],
                    "Matched provider authentication in session content"
                );
                assert_eq!(response["results"][0]["updatedAt"], "2026-09-14T12:02:03Z");
                assert_eq!(response["bootstrapping"], false);
                let recorded_list: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-thread-search.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    recorded_list,
                    json!({
                        "limit": 20,
                        "archived": false,
                        "searchTerm": "provider authentication",
                        "sortKey": "updated_at",
                        "sortDirection": "desc",
                    })
                );

                let blank = json!({"query": "   "});
                assert!(
                    agent
                        .ext_method(acp::ExtRequest::new(
                            SESSION_SEARCH_METHOD,
                            serde_json::value::to_raw_value(&blank).unwrap().into(),
                        ))
                        .await
                        .is_err()
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_mcp_inventory_to_the_extensions_contract() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0});
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        MCP_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(value["servers"].as_array().unwrap().len(), 2);
                assert_eq!(value["servers"][0]["displayName"], "Documentation");
                assert_eq!(value["servers"][0]["session"]["status"], "ready");
                assert_eq!(
                    value["servers"][0]["session"]["tools"][0]["name"],
                    "read_docs"
                );
                assert_eq!(value["servers"][1]["sourceLabel"], "plugin: search-plugin");
                assert_eq!(value["servers"][1]["session"]["authRequired"], true);
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reloads_codex_mcp_servers_before_an_explicit_refresh() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0, "cache": false});
                agent
                    .ext_method(acp::ExtRequest::new(
                        MCP_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                assert!(directory.path().join("mcp-servers-reloaded").exists());
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_plugin_inventory_to_the_extensions_contract() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0});
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        PLUGINS_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_str(response.0.get()).unwrap();
                let plugins = value["plugins"].as_array().unwrap();
                assert_eq!(plugins.len(), 1);
                assert_eq!(plugins[0]["name"], "Docs plugin");
                assert_eq!(plugins[0]["id"], "docs-plugin@fixture-marketplace");
                assert_eq!(
                    plugins[0]["root"],
                    directory
                        .path()
                        .join(".codex/plugins/docs")
                        .display()
                        .to_string()
                );
                assert_eq!(plugins[0]["scope"], "project");
                assert_eq!(plugins[0]["enabled"], true);
                assert_eq!(plugins[0]["version"], "1.2.3");
                assert_eq!(plugins[0]["description"], "Reads project documentation");
                assert_eq!(plugins[0]["marketplaceSource"], "fixture-marketplace");
                assert_eq!(plugins[0]["origin"]["type"], "marketplace_install");
                assert_eq!(plugins[0]["origin"]["source_name"], "fixture-marketplace");
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_hooks_to_a_read_only_extensions_contract() {
        use xai_hooks_plugins_types::{
            HookEvent, HookHandlerType, HooksAction, HooksActionRequest, HooksListResponse,
            OutcomeStatus,
        };

        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0.to_string()});
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        HOOKS_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let hooks: HooksListResponse = serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(hooks.hooks.len(), 2);
                assert!(hooks.project_trusted);
                assert!(!hooks.actions_supported);
                assert_eq!(hooks.hooks[0].event, HookEvent::PreToolUse);
                assert_eq!(hooks.hooks[0].handler_type, HookHandlerType::Command);
                assert_eq!(hooks.hooks[0].timeout_ms, 10_000);
                assert_eq!(
                    hooks.hooks[0].source_dir,
                    directory.path().join(".codex").display().to_string()
                );
                assert_eq!(hooks.hooks[1].handler_type, HookHandlerType::McpTool);
                assert!(hooks.hooks[1].disabled);

                let reload = HooksActionRequest {
                    session_id: session_id.0.to_string(),
                    action: HooksAction::Reload,
                };
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        HOOKS_ACTION_METHOD,
                        serde_json::value::to_raw_value(&reload).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let outcome: xai_hooks_plugins_types::ActionOutcome =
                    serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(outcome.status, OutcomeStatus::Success);

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_skills_to_the_extensions_contract() {
        use xai_grok_tools::implementations::skills::types::{SkillInfo, SkillScope};

        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0.to_string(), "cwd": "."});
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        SKILLS_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_str(response.0.get()).unwrap();
                let skills: Vec<SkillInfo> =
                    serde_json::from_value(value["skills"].clone()).unwrap();
                assert_eq!(skills.len(), 2);
                assert_eq!(skills[0].name, "repo-skill");
                assert_eq!(skills[0].display_name.as_deref(), Some("Repository check"));
                assert_eq!(
                    skills[0].short_description.as_deref(),
                    Some("Check this repository")
                );
                assert_eq!(skills[0].scope, SkillScope::Repo);
                assert_eq!(
                    skills[0].path,
                    directory
                        .path()
                        .join(".agents/skills/repo-skill/SKILL.md")
                        .display()
                        .to_string()
                );
                assert_eq!(skills[1].scope, SkillScope::Plugin);
                assert_eq!(
                    skills[1].plugin_name.as_deref(),
                    Some("docs-plugin@fixture-marketplace")
                );

                let toggle = json!({
                    "sessionId": session_id.0.to_string(),
                    "cwd": ".",
                    "name": "repo-skill",
                    "enabled": false,
                });
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        SKILLS_TOGGLE_METHOD,
                        serde_json::value::to_raw_value(&toggle).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(value["effectiveEnabled"], false);
                let skills: Vec<SkillInfo> =
                    serde_json::from_value(value["skills"].clone()).unwrap();
                assert!(!skills[0].enabled);
                let write: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-skill-config-write.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(write, json!({"name": "repo-skill", "enabled": false}));

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_skills_to_owned_commands() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let params = json!({"sessionId": session_id.0.to_string()});
                let response = agent
                    .ext_method(acp::ExtRequest::new(
                        COMMANDS_LIST_METHOD,
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_str(response.0.get()).unwrap();
                let commands: Vec<acp::AvailableCommand> =
                    serde_json::from_value(value["commands"].clone()).unwrap();
                assert_eq!(
                    commands
                        .iter()
                        .map(|command| command.name.as_str())
                        .collect::<Vec<_>>(),
                    ["plugin-skill", "repo-skill"]
                );
                let plugin = &commands[0];
                assert_eq!(plugin.meta.as_ref().unwrap()["provider"], "codex");
                assert_eq!(
                    plugin.meta.as_ref().unwrap()["owner"],
                    "docs-plugin@fixture-marketplace"
                );
                assert_eq!(plugin.meta.as_ref().unwrap()["bareName"], "plugin-skill");
                assert_eq!(
                    plugin.meta.as_ref().unwrap()["botOwnership"],
                    json!({
                        "version": 1,
                        "provider": "codex",
                        "owner": "docs-plugin@fixture-marketplace",
                        "kind": "skill",
                    })
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[test]
    fn maps_owned_skill_blocks_to_native_codex_input() {
        let meta = json!({
            "botCommand": {
                "version": 1,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill",
                "name": "repo-skill",
                "path": "/work/.agents/skills/repo-skill/SKILL.md",
                "arguments": "check auth",
            }
        })
        .as_object()
        .cloned();
        let block = acp::TextContent::new("/repo-skill check auth").meta(meta);
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            prompt_input(&[acp::ContentBlock::Text(block)], directory.path()).unwrap(),
            [
                UserInput::Skill {
                    name: "repo-skill".to_string(),
                    path: PathBuf::from("/work/.agents/skills/repo-skill/SKILL.md"),
                },
                UserInput::Text {
                    text: "check auth".to_string(),
                    text_elements: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn qualifies_codex_skill_command_collisions() {
        use xai_grok_tools::implementations::skills::types::{SkillInfo, SkillScope};

        let commands = codex_skill_commands(vec![
            SkillInfo {
                name: "login".to_owned(),
                description: "Reserved name".to_owned(),
                path: "/repo/.agents/skills/login/SKILL.md".to_owned(),
                scope: SkillScope::Repo,
                ..SkillInfo::default()
            },
            SkillInfo {
                name: "audit".to_owned(),
                description: "First audit".to_owned(),
                path: "/one/audit/SKILL.md".to_owned(),
                scope: SkillScope::User,
                ..SkillInfo::default()
            },
            SkillInfo {
                name: "audit".to_owned(),
                description: "Second audit".to_owned(),
                path: "/two/audit/SKILL.md".to_owned(),
                scope: SkillScope::User,
                ..SkillInfo::default()
            },
            SkillInfo {
                name: " ".to_owned(),
                description: "Invalid skill".to_owned(),
                path: "/invalid/SKILL.md".to_owned(),
                scope: SkillScope::User,
                ..SkillInfo::default()
            },
        ]);

        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].name, "user:audit");
        assert!(commands[1].name.starts_with("user:audit:"));
        assert_eq!(commands[2].name, "repo:login");
        assert_eq!(commands[0].meta.as_ref().unwrap()["bareName"], "audit");
        assert_eq!(commands[2].meta.as_ref().unwrap()["bareName"], "login");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.name.to_ascii_lowercase())
                .collect::<HashSet<_>>()
                .len(),
            commands.len()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sends_owned_skill_commands_as_native_codex_input() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let skill_path = directory.path().join(".agents/skills/repo-skill/SKILL.md");
                let meta = json!({
                    "botCommand": {
                        "version": 1,
                        "provider": "codex",
                        "owner": "repo",
                        "kind": "skill",
                        "name": "repo-skill",
                        "path": skill_path,
                        "arguments": "check auth",
                    }
                })
                .as_object()
                .cloned();
                let block = acp::TextContent::new("/repo-skill check auth").meta(meta);
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    agent.prompt(acp::PromptRequest::new(
                        session_id,
                        vec![acp::ContentBlock::Text(block)],
                    )),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                let turn = fixture_turn(directory.path());
                assert_eq!(
                    turn["input"],
                    json!([
                        {
                            "type": "skill",
                            "name": "repo-skill",
                            "path": skill_path,
                        },
                        {
                            "type": "text",
                            "text": "check auth",
                        }
                    ])
                );

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn maps_codex_plugin_actions_to_native_provider_methods() {
        use xai_hooks_plugins_types::{ActionOutcome, OutcomeStatus, PluginsAction};

        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;

                async fn action(
                    agent: &CodexAcpAgent,
                    session_id: &acp::SessionId,
                    action: PluginsAction,
                ) -> ActionOutcome {
                    let params = xai_hooks_plugins_types::PluginsActionRequest {
                        session_id: session_id.0.to_string(),
                        action,
                    };
                    let response = agent
                        .ext_method(acp::ExtRequest::new(
                            PLUGINS_ACTION_METHOD,
                            serde_json::value::to_raw_value(&params).unwrap().into(),
                        ))
                        .await
                        .unwrap();
                    serde_json::from_str(response.0.get()).unwrap()
                }

                let reload = action(&agent, &session_id, PluginsAction::Reload).await;
                assert_eq!(reload.status, OutcomeStatus::Success);
                let reconcile: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-plugin-reconcile.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(reconcile["reason"], "Bot plugin reload");

                let install = action(
                    &agent,
                    &session_id,
                    PluginsAction::Install {
                        source: "unused-plugin@fixture-marketplace".to_owned(),
                    },
                )
                .await;
                assert_eq!(install.status, OutcomeStatus::Success);
                assert!(install.requires_reload);
                let install_params: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-plugin-install.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(install_params["pluginName"], "unused-plugin");
                assert_eq!(
                    install_params["marketplacePath"],
                    directory
                        .path()
                        .join(".codex/marketplace.json")
                        .display()
                        .to_string()
                );

                let disable = action(
                    &agent,
                    &session_id,
                    PluginsAction::Disable {
                        plugin_id: "docs-plugin@fixture-marketplace".to_owned(),
                    },
                )
                .await;
                assert_eq!(disable.status, OutcomeStatus::Success);
                let config_write: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-config-write.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    config_write["keyPath"],
                    "plugins.\"docs-plugin@fixture-marketplace\".enabled"
                );
                assert_eq!(config_write["value"], false);
                assert_eq!(config_write["mergeStrategy"], "upsert");

                let uninstall = action(
                    &agent,
                    &session_id,
                    PluginsAction::Uninstall {
                        plugin_id: "docs-plugin@fixture-marketplace".to_owned(),
                        confirmed: true,
                    },
                )
                .await;
                assert_eq!(uninstall.status, OutcomeStatus::Success);
                assert!(uninstall.requires_reload);
                let uninstall_params: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-plugin-uninstall.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    uninstall_params["pluginId"],
                    "docs-plugin@fixture-marketplace"
                );

                let update = action(
                    &agent,
                    &session_id,
                    PluginsAction::Update {
                        plugin_id: Some("docs-plugin@fixture-marketplace".to_owned()),
                    },
                )
                .await;
                assert_eq!(update.status, OutcomeStatus::Unsupported);
                assert!(update.message.contains("per-plugin update"));

                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_plan_mode_reaches_the_provider_and_returns_to_default() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../xai-grok-pager-bin/tests/fixtures/codex");
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    executable,
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap();
                assert_eq!(session.modes.unwrap().available_modes.len(), 2);
                for mode in ["plan", "default"] {
                    agent
                        .set_session_mode(acp::SetSessionModeRequest::new(
                            session.session_id.clone(),
                            mode,
                        ))
                        .await
                        .unwrap();
                    let response = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        agent.prompt(acp::PromptRequest::new(
                            session.session_id.clone(),
                            vec![acp::ContentBlock::Text(acp::TextContent::new("OK"))],
                        )),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                    let value: Value = serde_json::from_slice(
                        &std::fs::read(directory.path().join("last-turn.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(value["collaborationMode"]["mode"], mode);
                    assert_eq!(
                        value["collaborationMode"]["settings"]["model"],
                        "test-model"
                    );
                    assert_eq!(
                        value["collaborationMode"]["settings"]["reasoning_effort"],
                        "low"
                    );
                    assert!(
                        value["collaborationMode"]["settings"]["developer_instructions"].is_null()
                    );
                }
                assert!(
                    agent
                        .set_session_mode(acp::SetSessionModeRequest::new(
                            session.session_id.clone(),
                            "unknown"
                        ))
                        .await
                        .is_err()
                );
                {
                    let mut sessions = agent.sessions.borrow_mut();
                    sessions.get_mut(&session.session_id).unwrap().turn_state =
                        CodexTurnState::Active {
                            turn_id: "busy".into(),
                        };
                }
                assert!(
                    agent
                        .set_session_mode(acp::SetSessionModeRequest::new(
                            session.session_id.clone(),
                            "plan"
                        ))
                        .await
                        .is_err()
                );
                agent
                    .sessions
                    .borrow_mut()
                    .get_mut(&session.session_id)
                    .unwrap()
                    .turn_state = CodexTurnState::Idle;
                let params = json!({
                    "permission_mode": "read-only",
                    "sessionId": session.session_id.0,
                });
                agent
                    .ext_notification(acp::ExtNotification::new(
                        "x.ai/yolo_mode_changed",
                        serde_json::value::to_raw_value(&params).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    agent.sessions.borrow()[&session.session_id].permission_mode,
                    CodexPermissionMode::ReadOnly
                );
                assert_eq!(
                    *agent.default_permission_mode.borrow(),
                    CodexPermissionMode::Default
                );
                agent
                    .prompt(acp::PromptRequest::new(
                        session.session_id.clone(),
                        vec![acp::ContentBlock::Text(acp::TextContent::new("OK"))],
                    ))
                    .await
                    .unwrap();
                let value: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("last-turn.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(value["approvalPolicy"], "on-request");
                assert_eq!(value["sandboxPolicy"]["type"], "readOnly");
                assert_eq!(
                    agent.sessions.borrow()[&session.session_id].mode,
                    CollaborationModeKind::Default
                );
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn keeps_codex_events_emitted_before_turn_start_response() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../xai-grok-pager-bin/tests/fixtures/codex");
                let (mut channel, agent_channel) = acp_channels();
                let (updates_tx, mut updates_rx) = tokio::sync::mpsc::unbounded_channel();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                if let acp::SessionUpdate::AgentMessageChunk(chunk) =
                                    &args.request.update
                                    && let acp::ContentBlock::Text(text) = &chunk.content
                                {
                                    updates_tx.send(text.text.clone()).unwrap();
                                }
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    executable,
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap();
                let response = agent
                    .prompt(acp::PromptRequest::new(
                        session.session_id,
                        vec![acp::ContentBlock::Text(acp::TextContent::new("OK"))],
                    ))
                    .await
                    .unwrap();
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                assert_eq!(
                    tokio::time::timeout(std::time::Duration::from_secs(1), updates_rx.recv())
                        .await
                        .unwrap()
                        .unwrap(),
                    "BOT_FIXTURE_OK\n"
                );
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn emits_aggregate_turn_diffs_through_the_full_adapter() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let (updates_tx, mut updates_rx) = tokio::sync::mpsc::unbounded_channel();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                updates_tx.send(args.request.update.clone()).unwrap();
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let response = prompt_fixture(&agent, &session_id, "TURN_DIFF").await;
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                let edit_call = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    loop {
                        if let acp::SessionUpdate::ToolCall(call) = updates_rx.recv().await.unwrap()
                            && call.kind == acp::ToolKind::Edit
                        {
                            break call;
                        }
                    }
                })
                .await
                .unwrap();
                assert_eq!(edit_call.locations[0].path, PathBuf::from("src/main.rs"));
                let (hunks, count) = xai_grok_pager_diff::extract_edit_hunks(&edit_call);
                assert_eq!(count, 1);
                assert_eq!(hunks.len(), 1);
                assert!(hunks[0].iter().any(|line| line.text.contains("old")));
                assert!(hunks[0].iter().any(|line| line.text.contains("new")));
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forwards_codex_mcp_elicitations_through_the_full_adapter() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let observed = Rc::new(RefCell::new(Vec::new()));
                let observed_receiver = observed.clone();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::ExtMethod(args) => {
                                assert_eq!(args.request.method.as_ref(), "x.ai/mcp/elicit");
                                let request: McpElicitExtRequest =
                                    serde_json::from_str(args.request.params.get()).unwrap();
                                let response = match request.server_name.as_str() {
                                    "form-fixture" => McpElicitExtResponse::Accept {
                                        content: Some(json!({"value": "done"})),
                                    },
                                    "oauth-fixture" => {
                                        McpElicitExtResponse::Accept { content: None }
                                    }
                                    other => panic!("unexpected MCP server: {other}"),
                                };
                                observed_receiver
                                    .borrow_mut()
                                    .push(serde_json::to_value(request).unwrap());
                                let raw = serde_json::value::to_raw_value(&response).unwrap();
                                let _ =
                                    args.response_tx.send(Ok(acp::ExtResponse::new(raw.into())));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                if args.request.method.as_ref() == PROVIDER_USAGE_UPDATED_METHOD {
                                    let _ = args.response_tx.send(Ok(()));
                                    continue;
                                }
                                assert_eq!(
                                    args.request.method.as_ref(),
                                    "x.ai/mcp/elicit_complete"
                                );
                                observed_receiver
                                    .borrow_mut()
                                    .push(serde_json::from_str(args.request.params.get()).unwrap());
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let form = prompt_fixture(&agent, &session_id, "MCP_FORM").await;
                assert_eq!(form.stop_reason, acp::StopReason::EndTurn);
                let form_response: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("mcp-form-response.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(form_response["action"], "accept");
                assert_eq!(form_response["content"]["value"], "done");
                let url = prompt_fixture(&agent, &session_id, "MCP_URL").await;
                assert_eq!(url.stop_reason, acp::StopReason::EndTurn);
                let url_response: Value = serde_json::from_slice(
                    &std::fs::read(directory.path().join("mcp-url-response.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(url_response["action"], "accept");
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while observed.borrow().len() < 3 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                {
                    let observed = observed.borrow();
                    assert_eq!(observed.len(), 3);
                    assert_eq!(observed[0]["serverName"], "form-fixture");
                    assert_eq!(observed[0]["_meta"]["trace"], "form-fixture");
                    assert_eq!(observed[0]["requestedSchema"]["required"][0], "value");
                    assert_eq!(observed[1]["serverName"], "oauth-fixture");
                    assert_eq!(observed[1]["_meta"]["trace"], "url-fixture");
                    assert_eq!(observed[1]["elicitationId"], "elicit-fixture");
                    assert_eq!(observed[2]["elicitationId"], "elicit-fixture");
                    assert_eq!(observed[2]["serverName"], "oauth-fixture");
                }
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forwards_codex_questions_through_the_full_adapter() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let observed = Rc::new(RefCell::new(None));
                let observed_receiver = observed.clone();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::ExtMethod(args) => {
                                assert_eq!(args.request.method.as_ref(), "x.ai/ask_user_question");
                                let request: AskUserQuestionExtRequest =
                                    serde_json::from_str(args.request.params.get()).unwrap();
                                let response = AskUserQuestionExtResponse::Accepted {
                                    answers: indexmap::IndexMap::from([
                                        ("Mode: Choose a mode".to_owned(), vec!["Fast".to_owned()]),
                                        ("Token: Enter token".to_owned(), vec!["Other".to_owned()]),
                                    ]),
                                    annotations: Some(HashMap::from([(
                                        "Token: Enter token".to_owned(),
                                        QuestionAnnotation {
                                            preview: None,
                                            notes: Some("swordfish".to_owned()),
                                        },
                                    )])),
                                };
                                *observed_receiver.borrow_mut() = Some(request);
                                let raw = serde_json::value::to_raw_value(&response).unwrap();
                                let _ =
                                    args.response_tx.send(Ok(acp::ExtResponse::new(raw.into())));
                            }
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args)
                                if args.request.method.as_ref()
                                    == PROVIDER_USAGE_UPDATED_METHOD =>
                            {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                let response = prompt_fixture(&agent, &session_id, "QUESTION").await;
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                {
                    let request = observed.borrow();
                    let request = request.as_ref().unwrap();
                    assert_eq!(request.tool_call_id, "question-fixture");
                    assert_eq!(
                        request.auto_resolution_ms,
                        Some(CODEX_NON_BLOCKING_QUESTION_TIMEOUT_MS)
                    );
                    assert!(!request.question_metadata["choice"].allow_freeform);
                    assert!(request.question_metadata["secret"].allow_freeform);
                    assert!(request.question_metadata["secret"].secret);
                }
                let native_response: ToolRequestUserInputResponse = serde_json::from_slice(
                    &std::fs::read(directory.path().join("request-user-input-response.json"))
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    native_response.answers["choice"].answers,
                    vec!["Fast".to_owned()]
                );
                assert_eq!(
                    native_response.answers["secret"].answers,
                    vec!["user_note: swordfish".to_owned()]
                );
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn queues_cancellation_while_codex_turn_start_is_pending() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../xai-grok-pager-bin/tests/fixtures/codex");
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = Rc::new(
                    CodexAcpAgent::start(
                        executable,
                        agent_channel.tx,
                        CodexPermissionMode::Default,
                    )
                    .await
                    .unwrap(),
                );
                let session = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap();
                let session_id = session.session_id;
                let prompt_agent = agent.clone();
                let prompt_session_id = session_id.clone();
                let prompt = tokio::task::spawn_local(async move {
                    prompt_agent
                        .prompt(acp::PromptRequest::new(
                            prompt_session_id,
                            vec![acp::ContentBlock::Text(acp::TextContent::new("START_WAIT"))],
                        ))
                        .await
                });
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while !directory.path().join("turn-starting").exists() {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(
                    agent.sessions.borrow()[&session_id].turn_state,
                    CodexTurnState::Starting {
                        cancel_requested: false,
                        stop_background_terminals: false,
                    }
                );
                assert!(
                    agent
                        .set_session_mode(acp::SetSessionModeRequest::new(
                            session_id.clone(),
                            "plan",
                        ))
                        .await
                        .is_err()
                );
                assert!(
                    agent
                        .set_session_model(acp::SetSessionModelRequest::new(
                            session_id.clone(),
                            "test-model",
                        ))
                        .await
                        .is_err()
                );
                agent
                    .cancel(
                        acp::CancelNotification::new(session_id.clone()).meta(
                            json!({"stopBackgroundTerminals": true})
                                .as_object()
                                .cloned(),
                        ),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    agent.sessions.borrow()[&session_id].turn_state,
                    CodexTurnState::Starting {
                        cancel_requested: true,
                        stop_background_terminals: true,
                    }
                );
                let response = tokio::time::timeout(std::time::Duration::from_secs(5), prompt)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(response.stop_reason, acp::StopReason::Cancelled);
                assert!(directory.path().join("interrupted").exists());
                assert!(directory.path().join("cancelled").exists());
                assert_eq!(
                    agent.sessions.borrow()[&session_id].turn_state,
                    CodexTurnState::Idle
                );
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn permission_transitions_restore_defaults_and_stay_session_scoped() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let first_directory = tempfile::tempdir().unwrap();
                let second_directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let first = agent
                    .new_session(acp::NewSessionRequest::new(
                        first_directory.path().to_path_buf(),
                    ))
                    .await
                    .unwrap()
                    .session_id;
                let second = agent
                    .new_session(acp::NewSessionRequest::new(
                        second_directory.path().to_path_buf(),
                    ))
                    .await
                    .unwrap()
                    .session_id;
                set_permission_mode(&agent, &second, "read-only").await;
                for (mode, approval, reviewer, sandbox) in [
                    (
                        "ask",
                        Some("on-request"),
                        Some("user"),
                        Some("workspaceWrite"),
                    ),
                    (
                        "auto",
                        Some("on-request"),
                        Some("auto_review"),
                        Some("workspaceWrite"),
                    ),
                    (
                        "read-only",
                        Some("on-request"),
                        Some("user"),
                        Some("readOnly"),
                    ),
                    (
                        "always-approve",
                        Some("never"),
                        Some("user"),
                        Some("dangerFullAccess"),
                    ),
                    ("default", None, None, None),
                ] {
                    set_permission_mode(&agent, &first, mode).await;
                    let response = prompt_fixture(&agent, &first, "OK").await;
                    assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                    let turn = fixture_turn(first_directory.path());
                    assert_eq!(turn["approvalPolicy"].as_str(), approval);
                    assert_eq!(turn["approvalsReviewer"].as_str(), reviewer);
                    assert_eq!(turn["sandboxPolicy"]["type"].as_str(), sandbox);
                }
                let response = prompt_fixture(&agent, &second, "OK").await;
                assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                let second_turn = fixture_turn(second_directory.path());
                assert_eq!(second_turn["approvalPolicy"], "on-request");
                assert_eq!(second_turn["approvalsReviewer"], "user");
                assert_eq!(second_turn["sandboxPolicy"]["type"], "readOnly");
                assert_eq!(
                    agent.sessions.borrow()[&first].permission_mode,
                    CodexPermissionMode::Default
                );
                assert_eq!(
                    agent.sessions.borrow()[&second].permission_mode,
                    CodexPermissionMode::ReadOnly
                );
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forwards_codex_approval_denial_and_cancellation_decisions() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let (mut channel, agent_channel) = acp_channels();
                let receiver = tokio::task::spawn_local(async move {
                    let mut decisions = VecDeque::from([
                        Some("allow_once"),
                        Some("allow_always"),
                        Some("reject"),
                        None,
                    ]);
                    while let Some(message) = channel.rx.recv().await {
                        match message.boxed() {
                            xai_acp_lib::AcpClientMessageBox::RequestPermission(args) => {
                                let outcome = match decisions.pop_front().unwrap() {
                                    Some(selected) => {
                                        let option = args
                                            .request
                                            .options
                                            .iter()
                                            .find(|option| option.option_id.0.as_ref() == selected)
                                            .unwrap();
                                        acp::RequestPermissionOutcome::Selected(
                                            acp::SelectedPermissionOutcome::new(
                                                option.option_id.clone(),
                                            ),
                                        )
                                    }
                                    None => acp::RequestPermissionOutcome::Cancelled,
                                };
                                let _ = args
                                    .response_tx
                                    .send(Ok(acp::RequestPermissionResponse::new(outcome)));
                            }
                            xai_acp_lib::AcpClientMessageBox::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessageBox::ExtNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            _ => panic!("unexpected client request"),
                        }
                    }
                });
                let agent = CodexAcpAgent::start(
                    codex_fixture(),
                    agent_channel.tx,
                    CodexPermissionMode::Default,
                )
                .await
                .unwrap();
                let session_id = agent
                    .new_session(acp::NewSessionRequest::new(directory.path().to_path_buf()))
                    .await
                    .unwrap()
                    .session_id;
                for expected in ["accept", "acceptForSession", "decline", "cancel"] {
                    let response = prompt_fixture(&agent, &session_id, "APPROVAL").await;
                    assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
                    let value: Value = serde_json::from_slice(
                        &std::fs::read(directory.path().join("approval-response.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(value["decision"], expected);
                }
                agent.client.close().await.unwrap();
                receiver.abort();
            })
            .await;
    }

    #[test]
    fn replays_proposed_plans_as_visible_messages() {
        let updates = replay_item(
            &acp::SessionId::new("thread-1"),
            &json!({"type": "plan", "id": "plan-1", "text": "# Proposed plan"}),
        );
        assert_eq!(updates.len(), 1);
        let acp::SessionUpdate::AgentMessageChunk(chunk) = &updates[0] else {
            panic!("expected a visible message");
        };
        let acp::ContentBlock::Text(text) = &chunk.content else {
            panic!("expected plan text");
        };
        assert_eq!(text.text, "# Proposed plan");
    }

    fn model() -> Model {
        Model {
            id: "model-row".to_owned(),
            model: "gpt-5".to_owned(),
            display_name: "GPT-5".to_owned(),
            description: "Test model".to_owned(),
            hidden: false,
            default_reasoning_effort: "medium".to_owned(),
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: "medium".to_owned(),
                description: "Balanced".to_owned(),
            }],
            input_modalities: vec![InputModality::Text, InputModality::Image],
            is_default: true,
            supports_personality: false,
            extra: Default::default(),
        }
    }

    #[test]
    fn reads_every_permission_mode_from_the_ui_notification() {
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "default"})),
            Some(CodexPermissionMode::Default)
        );
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "ask"})),
            Some(CodexPermissionMode::Ask)
        );
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "auto"})),
            Some(CodexPermissionMode::Auto)
        );
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "read-only"})),
            Some(CodexPermissionMode::ReadOnly)
        );
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "always-approve"})),
            Some(CodexPermissionMode::AlwaysApprove)
        );
        assert_eq!(
            CodexPermissionMode::from_wire(&json!({"permission_mode": "unknown"})),
            None
        );
    }

    #[test]
    fn maps_permission_modes_to_stable_turn_settings() {
        assert_eq!(CodexPermissionMode::Default.approval_policy(), None);
        assert_eq!(CodexPermissionMode::Default.approvals_reviewer(), None);
        assert_eq!(CodexPermissionMode::Default.sandbox_policy(), None);
        assert_eq!(
            CodexPermissionMode::Auto.approval_policy().as_deref(),
            Some("on-request")
        );
        assert_eq!(
            CodexPermissionMode::Auto.approvals_reviewer().as_deref(),
            Some("auto_review")
        );
        assert_eq!(
            CodexPermissionMode::Auto.sandbox_policy(),
            Some(json!({"type": "workspaceWrite"}))
        );
        assert_eq!(
            CodexPermissionMode::ReadOnly.sandbox_policy(),
            Some(json!({"type": "readOnly"}))
        );
        assert_eq!(
            CodexPermissionMode::AlwaysApprove
                .approval_policy()
                .as_deref(),
            Some("never")
        );
        assert_eq!(
            CodexPermissionMode::AlwaysApprove.sandbox_policy(),
            Some(json!({"type": "dangerFullAccess"}))
        );
    }

    #[test]
    fn converts_codex_models_to_the_existing_picker_contract() {
        let info = model_info(&model(), None);
        assert_eq!(info.model_id.0.as_ref(), "gpt-5");
        assert_eq!(info.name, "gpt-5");
        let meta = info.meta.expect("model metadata");
        assert_eq!(meta["reasoningEffort"], "medium");
        assert_eq!(meta["supportsReasoningEffort"], true);
        assert_eq!(meta["acceptsImages"], true);
    }

    #[test]
    fn model_effort_uses_only_the_selected_model_catalog() {
        let mut model = model();
        assert_eq!(
            resolve_model_effort(&model, None, Some("high")).unwrap(),
            Some("medium".into())
        );
        assert!(resolve_model_effort(&model, Some("ultra"), None).is_err());
        model
            .supported_reasoning_efforts
            .push(ReasoningEffortOption {
                reasoning_effort: "ultra".into(),
                description: "Ultra".into(),
            });
        assert_eq!(
            resolve_model_effort(&model, Some("ultra"), None).unwrap(),
            Some("ultra".into())
        );
        assert_eq!(
            resolve_model_effort(&model, None, Some("ultra")).unwrap(),
            Some("ultra".into())
        );
        let info = model_info(&model, Some("ultra"));
        let state = crate::acp::model_state::ModelState::from(Some(acp::SessionModelState::new(
            model.model.clone(),
            vec![info],
        )));
        assert_eq!(state.reasoning_effort.unwrap().to_string(), "ultra");
        assert_eq!(state.reasoning_effort_options().len(), 2);
        model.supported_reasoning_efforts.clear();
        assert_eq!(
            resolve_model_effort(&model, None, Some("medium")).unwrap(),
            None
        );
        assert!(
            model_info(&model, None)
                .meta
                .unwrap()
                .get("reasoningEffort")
                .is_none()
        );
    }

    #[test]
    fn maps_codex_question_notes_to_the_original_question_id() {
        let mut ids = HashMap::new();
        ids.insert("Choice?".to_owned(), "choice".to_owned());
        let mut answers = indexmap::IndexMap::new();
        answers.insert("Choice?".to_owned(), vec!["Other".to_owned()]);
        let mut annotations = HashMap::new();
        annotations.insert(
            "Choice?".to_owned(),
            QuestionAnnotation {
                preview: None,
                notes: Some("Custom".to_owned()),
            },
        );
        let result = codex_question_answers(
            AskUserQuestionExtResponse::Accepted {
                answers,
                annotations: Some(annotations),
            },
            &ids,
        );
        assert_eq!(
            result["choice"],
            ToolRequestUserInputAnswer {
                answers: vec!["user_note: Custom".to_owned()]
            }
        );
    }

    #[test]
    fn maps_codex_threads_to_the_existing_session_picker_contract() {
        let payload = codex_session_list_payload(vec![Thread {
            id: "thread-1".to_owned(),
            cwd: Some(PathBuf::from("/workspace")),
            name: None,
            preview: Some("First request\nMore detail".to_owned()),
            model: Some("gpt-5".to_owned()),
            reasoning_effort: Some("medium".to_owned()),
            created_at: Some(1_725_192_000),
            updated_at: Some(1_725_192_123_000),
            turns: vec![Turn {
                id: "turn-1".to_owned(),
                status: "completed".to_owned(),
                items: Vec::new(),
                extra: BTreeMap::new(),
            }],
            extra: BTreeMap::new(),
        }]);
        assert_eq!(payload["_meta"]["x.ai/listScope"], "cwd");
        assert_eq!(payload["sessions"][0]["sessionId"], "thread-1");
        assert_eq!(payload["sessions"][0]["summary"], "First request");
        assert_eq!(
            payload["sessions"][0]["firstPrompt"],
            "First request\nMore detail"
        );
        assert_eq!(payload["sessions"][0]["source"], "provider");
        assert_eq!(payload["sessions"][0]["updatedAt"], "2024-09-01T12:02:03Z");
    }

    #[test]
    fn replays_codex_text_reasoning_and_command_history() {
        let turns = vec![Turn {
            id: "turn-1".to_owned(),
            status: "completed".to_owned(),
            items: vec![
                json!({
                    "id": "user-1",
                    "type": "userMessage",
                    "content": [{"type": "text", "text": "Hello"}],
                }),
                json!({
                    "id": "reasoning-1",
                    "type": "reasoning",
                    "summary": ["Checking"],
                }),
                json!({
                    "id": "hook-1",
                    "type": "hookPrompt",
                    "fragments": [{
                        "hookRunId": "hook-run-1",
                        "text": "Use the project policy."
                    }],
                }),
                json!({
                    "id": "output-1",
                    "type": "functionCallOutput",
                    "name": "lookup",
                    "namespace": "records",
                    "output": [{"type": "input_text", "text": "record alpha"}],
                }),
                json!({
                    "id": "dynamic-1",
                    "type": "dynamicToolCall",
                    "tool": "lookup",
                    "namespace": "records",
                    "arguments": {"key": "alpha"},
                    "status": "completed",
                    "success": true,
                    "contentItems": [{"type": "inputText", "text": "record alpha"}],
                }),
                json!({
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": "pwd",
                    "cwd": "/workspace",
                    "status": "completed",
                    "aggregatedOutput": "/workspace\n",
                    "exitCode": 0,
                }),
                json!({
                    "id": "agent-1",
                    "type": "agentMessage",
                    "text": "Done",
                }),
            ],
            extra: BTreeMap::new(),
        }];
        let updates = replay_updates(&acp::SessionId::new("thread-1"), &turns);
        assert_eq!(updates.len(), 7);
        let serialized = serde_json::to_value(updates).expect("serialize replay updates");
        assert_eq!(serialized[0]["sessionUpdate"], "user_message_chunk");
        assert_eq!(serialized[1]["sessionUpdate"], "agent_thought_chunk");
        assert_eq!(serialized[2]["title"], "Apply hook context");
        assert_eq!(
            serialized[2]["content"][1]["content"]["text"],
            "Use the project policy."
        );
        assert_eq!(
            serialized[3]["title"],
            "Receive output from records · lookup"
        );
        assert_eq!(serialized[3]["rawOutput"], "record alpha");
        assert_eq!(serialized[4]["title"], "Run records · lookup");
        assert_eq!(serialized[4]["rawInput"]["arguments"]["key"], "alpha");
        assert_eq!(serialized[4]["rawOutput"], "record alpha");
        assert_eq!(serialized[5]["sessionUpdate"], "tool_call");
        assert_eq!(serialized[5]["rawInput"]["command"], "pwd");
        assert_eq!(serialized[5]["rawOutput"]["type"], "Bash");
        assert_eq!(serialized[6]["sessionUpdate"], "agent_message_chunk");

        let completion = replay_turn_completion(&acp::SessionId::new("thread-1"), &turns[0])
            .expect("serialize turn completion")
            .expect("completed turn has a receipt");
        assert_eq!(completion.method.as_ref(), "x.ai/session/update");
        let completion: Value =
            serde_json::from_str(completion.params.get()).expect("parse turn completion");
        assert_eq!(completion["sessionId"], "thread-1");
        assert_eq!(completion["update"]["sessionUpdate"], "turn_completed");
        assert_eq!(completion["update"]["prompt_id"], "turn-1");
        assert_eq!(completion["update"]["stop_reason"], "end_turn");
        assert_eq!(completion["_meta"]["isReplay"], true);
    }

    #[test]
    fn replays_only_terminal_codex_turn_receipts() {
        let session_id = acp::SessionId::new("thread-1");
        let cases = [
            ("completed", Some("end_turn")),
            ("cancelled", Some("cancelled")),
            ("interrupted", Some("cancelled")),
            ("failed", Some("error")),
            ("inProgress", None),
            ("futureStatus", None),
        ];

        for (status, expected_stop_reason) in cases {
            let turn = Turn {
                id: format!("turn-{status}"),
                status: status.to_owned(),
                items: Vec::new(),
                extra: BTreeMap::new(),
            };
            let completion = replay_turn_completion(&session_id, &turn)
                .expect("serialize terminal turn completion");
            let actual_stop_reason = completion
                .map(|notification| {
                    serde_json::from_str::<Value>(notification.params.get())
                        .expect("parse terminal turn completion")
                })
                .map(|value| {
                    value["update"]["stop_reason"]
                        .as_str()
                        .expect("stop reason is text")
                        .to_owned()
                });
            assert_eq!(actual_stop_reason.as_deref(), expected_stop_reason);
        }
    }

    #[test]
    fn maps_live_codex_commands_to_native_execute_fields() {
        let call = bot_core::ToolCall {
            kind: ToolCallKind::Command,
            title: "find . -maxdepth 1 -type f".to_owned(),
            detail: Some("$ find . -maxdepth 1 -type f".to_owned()),
            output: Some("./.DS_Store\n".to_owned()),
            state: ToolCallState::Completed,
            duration_ms: Some(12),
        };
        let tool = tool_call("command-1".to_owned(), &call);
        let serialized = serde_json::to_value(tool).expect("tool call");
        assert_eq!(
            serialized["rawInput"]["command"],
            "find . -maxdepth 1 -type f"
        );
        assert_eq!(serialized["rawOutput"]["type"], "Bash");
        assert_eq!(
            serialized["rawOutput"]["output"],
            json!([46, 47, 46, 68, 83, 95, 83, 116, 111, 114, 101, 10])
        );
        assert_eq!(serialized["rawOutput"]["exit_code"], 0);
    }

    #[test]
    fn preserves_live_codex_command_exit_details() {
        let notification = bot_provider_codex::ServerNotification {
            method: "item/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": "test -f missing.txt",
                    "cwd": "/workspace",
                    "status": "failed",
                    "aggregatedOutput": "missing.txt: not found\n",
                    "exitCode": 7
                }
            }),
        };
        let (_, tool) =
            protocol_command_completion_call(&acp::SessionId::new("thread-1"), &notification)
                .expect("command completion");
        let serialized = serde_json::to_value(tool).expect("tool call");
        assert_eq!(serialized["rawInput"]["command"], "test -f missing.txt");
        assert_eq!(serialized["rawInput"]["cwd"], "/workspace");
        assert_eq!(serialized["rawOutput"]["exit_code"], 7);
        assert_eq!(serialized["rawOutput"]["current_dir"], "/workspace");
        assert_eq!(serialized["status"], "failed");
    }

    #[test]
    fn maps_codex_plan_updates_to_the_existing_todo_contract() {
        let notification = bot_provider_codex::ServerNotification {
            method: "turn/plan/updated".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "explanation": "Updated after inspection",
                "plan": [
                    {"step": "Inspect the code", "status": "completed"},
                    {"step": "Apply the fix", "status": "inProgress"},
                    {"step": "Run checks", "status": "pending"}
                ]
            }),
        };
        let update = protocol_session_update(&notification).expect("plan update");
        let acp::SessionUpdate::Plan(plan) = update else {
            panic!("plan");
        };
        assert_eq!(plan.entries.len(), 3);
        assert_eq!(plan.entries[0].content, "Inspect the code");
        assert_eq!(plan.entries[0].status, acp::PlanEntryStatus::Completed);
        assert_eq!(plan.entries[1].status, acp::PlanEntryStatus::InProgress);
        assert_eq!(plan.entries[2].status, acp::PlanEntryStatus::Pending);
    }

    #[test]
    fn maps_codex_file_changes_to_the_existing_diff_component() {
        let notification = bot_provider_codex::ServerNotification {
            method: "item/completed".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 2000,
                "item": {
                    "id": "edit-1",
                    "type": "fileChange",
                    "status": "completed",
                    "changes": [{
                        "path": "src/main.rs",
                        "kind": {"type": "update"},
                        "diff": "@@ -1,3 +1,3 @@\n fn main() {\n-    old();\n+    new();\n }\n"
                    }]
                }
            }),
        };
        let changes = protocol_file_change_calls(&acp::SessionId::new("thread-1"), &notification);
        assert_eq!(changes.len(), 1);
        let (tool_id, call) = &changes[0];
        assert_eq!(tool_id, "thread-1:codex:item:edit-1");
        assert_eq!(call.kind, acp::ToolKind::Edit);
        assert_eq!(call.status, acp::ToolCallStatus::Completed);
        assert_eq!(call.locations[0].path, PathBuf::from("src/main.rs"));
        let (hunks, count) = xai_grok_pager_diff::extract_edit_hunks(call);
        assert_eq!(count, 1);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].iter().any(|line| line.text.contains("old();")));
        assert!(hunks[0].iter().any(|line| line.text.contains("new();")));
    }

    #[test]
    fn keeps_unparsed_codex_file_changes_visible() {
        let calls = file_change_calls(
            &acp::SessionId::new("thread-1"),
            "edit-1",
            Some(&json!([{
                "path": "src/main.rs",
                "kind": {"type": "update"},
                "diff": "non-standard patch"
            }])),
            Some("inProgress"),
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.kind, acp::ToolKind::Other);
        let serialized = serde_json::to_value(&calls[0].1).expect("tool call");
        assert_eq!(
            serialized["content"][0]["content"]["text"],
            "non-standard patch"
        );
    }

    #[test]
    fn maps_aggregate_turn_diffs_to_completed_edit_cards() {
        let update = TurnDiffUpdatedNotification {
            thread_id: "thread-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            diff: "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n".to_owned(),
            extra: BTreeMap::new(),
        };
        let calls = turn_diff_calls(&acp::SessionId::new("thread-1"), &update, &HashSet::new());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "thread-1:codex:item:turn-diff:turn-1");
        assert_eq!(calls[0].1.kind, acp::ToolKind::Edit);
        assert_eq!(calls[0].1.status, acp::ToolCallStatus::Completed);
        assert_eq!(calls[0].1.locations[0].path, PathBuf::from("src/main.rs"));
        let (hunks, count) = xai_grok_pager_diff::extract_edit_hunks(&calls[0].1);
        assert_eq!(count, 1);
        assert_eq!(hunks.len(), 1);
    }

    #[test]
    fn aggregate_turn_diffs_skip_item_level_file_changes() {
        let update = TurnDiffUpdatedNotification {
            thread_id: "thread-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            diff: "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/docs/readme.md b/docs/readme.md\n--- a/docs/readme.md\n+++ b/docs/readme.md\n@@ -1 +1 @@\n-old docs\n+new docs\n".to_owned(),
            extra: BTreeMap::new(),
        };
        let item_paths = HashSet::from([PathBuf::from("src/main.rs")]);
        let calls = turn_diff_calls(&acp::SessionId::new("thread-1"), &update, &item_paths);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1.locations[0].path,
            PathBuf::from("docs/readme.md")
        );
    }

    #[test]
    fn routes_legacy_requests_to_the_matching_codex_thread() {
        let matching = CodexEvent::Request(ServerRequest {
            id: RequestId::Number(1),
            method: "execCommandApproval".to_owned(),
            params: json!({"conversationId": "thread-1", "callId": "call-1"}),
        });
        let other = CodexEvent::Request(ServerRequest {
            id: RequestId::Number(2),
            method: "execCommandApproval".to_owned(),
            params: json!({"conversationId": "thread-2", "callId": "call-2"}),
        });
        assert!(event_matches(&matching, "thread-1", "turn-1"));
        assert!(!event_matches(&other, "thread-1", "turn-1"));
    }

    #[test]
    fn serializes_legacy_approval_decisions() {
        assert_eq!(
            legacy_approval_response(PermissionDecision::AllowOnce),
            json!({"decision": "approved"})
        );
        assert_eq!(
            legacy_approval_response(PermissionDecision::AllowAlways),
            json!({"decision": "approved_for_session"})
        );
        assert_eq!(
            legacy_approval_response(PermissionDecision::Reject),
            json!({"decision": {"denied": {"rejection": "User declined"}}})
        );
        assert_eq!(
            legacy_approval_response(PermissionDecision::Cancel),
            json!({"decision": "abort"})
        );
    }

    #[test]
    fn maps_codex_mcp_forms_to_the_existing_elicitation_card() {
        let request = ServerRequest {
            id: RequestId::Number(7),
            method: "mcpServer/elicitation/request".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "serverName": "demo",
                "mode": "form",
                "_meta": {"trace": "form-1"},
                "message": "Enter a value",
                "requestedSchema": {
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                }
            }),
        };
        let payload = mcp_elicitation_payload(&acp::SessionId::new("thread-1"), &request).unwrap();
        assert_eq!(payload.session_id, "thread-1");
        assert_eq!(payload.tool_call_id, "mcp-elicit-7");
        assert_eq!(payload.server_name, "demo");
        assert_eq!(payload.meta.as_ref().unwrap()["trace"], "form-1");
        assert_eq!(payload.message, "Enter a value");
        let McpElicitModeFields::Form { requested_schema } = payload.mode else {
            panic!("form");
        };
        assert_eq!(
            requested_schema.expect("schema")["properties"]["value"]["type"],
            "string"
        );
        assert_eq!(
            serde_json::to_value(mcp_elicitation_result(McpElicitExtResponse::Accept {
                content: Some(json!({"value": "done"}))
            }))
            .unwrap(),
            json!({"action": "accept", "content": {"value": "done"}, "_meta": null})
        );
        assert_eq!(
            serde_json::to_value(mcp_elicitation_result(McpElicitExtResponse::Decline)).unwrap(),
            json!({"action": "decline", "content": null, "_meta": null})
        );
    }

    #[test]
    fn maps_codex_mcp_urls_to_the_existing_oauth_card() {
        let request = ServerRequest {
            id: RequestId::String("oauth-1".to_owned()),
            method: "mcpServer/elicitation/request".to_owned(),
            params: json!({
                "threadId": "thread-1",
                "serverName": "demo",
                "mode": "url",
                "_meta": {"trace": "url-1"},
                "message": "Sign in",
                "elicitationId": "elicit-1",
                "url": "https://example.com/login"
            }),
        };
        let payload = mcp_elicitation_payload(&acp::SessionId::new("thread-1"), &request).unwrap();
        let McpElicitModeFields::Url {
            url,
            elicitation_id,
        } = payload.mode
        else {
            panic!("url");
        };
        assert_eq!(url, "https://example.com/login");
        assert_eq!(elicitation_id, "elicit-1");
        assert_eq!(payload.meta.unwrap()["trace"], "url-1");
    }
}
