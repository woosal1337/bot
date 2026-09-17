use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

use crate::{
    ACCOUNT_LOGIN_CANCEL_METHOD, ACCOUNT_LOGIN_START_METHOD, ACCOUNT_LOGOUT_METHOD,
    ACCOUNT_RATE_LIMITS_READ_METHOD, ACCOUNT_READ_METHOD, ACCOUNT_USAGE_READ_METHOD,
    AccountRateLimitsResponse, AccountReadParams, AccountReadResponse, AccountUsageResponse,
    CONFIG_VALUE_WRITE_METHOD, CancelLoginAccountParams, CancelLoginAccountResponse,
    ConfigValueWriteParams, HOOKS_LIST_METHOD, HooksListParams, HooksListResponse,
    INITIALIZE_METHOD, INITIALIZED_METHOD, IncomingMessage, InitializeParams, InitializeResponse,
    LoginAccountParams, LoginAccountResponse, LogoutAccountResponse, MCP_SERVER_RELOAD_METHOD,
    MCP_SERVER_STATUS_LIST_METHOD, MODEL_LIST_METHOD, McpServerRefreshResponse,
    McpServerStatusListParams, McpServerStatusListResponse, ModelListParams, ModelListResponse,
    PLUGIN_INSTALL_METHOD, PLUGIN_LIST_METHOD, PLUGIN_RECONCILE_METHOD, PLUGIN_UNINSTALL_METHOD,
    PluginInstallParams, PluginListParams, PluginListResponse, PluginReconcileParams,
    PluginUninstallParams, ProtocolError, RemoteError, RequestId, SKILLS_CONFIG_WRITE_METHOD,
    SKILLS_LIST_METHOD, ServerNotification, ServerRequest, ServerResponse, SkillsConfigWriteParams,
    SkillsConfigWriteResponse, SkillsListParams, SkillsListResponse, THREAD_COMPACT_START_METHOD,
    THREAD_DELETE_METHOD, THREAD_FORK_METHOD, THREAD_LIST_METHOD, THREAD_RESUME_METHOD,
    THREAD_REVERT_METHOD, THREAD_SEARCH_METHOD, THREAD_SET_NAME_METHOD, THREAD_START_METHOD,
    THREAD_TURNS_LIST_METHOD, TURN_INTERRUPT_METHOD, TURN_START_METHOD, TURN_STEER_METHOD,
    ThreadCompactStartParams, ThreadCompactStartResponse, ThreadDeleteParams, ThreadDeleteResponse,
    ThreadForkParams, ThreadForkResponse, ThreadListParams, ThreadListResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadRevertParams, ThreadRevertResponse, ThreadSearchParams,
    ThreadSearchResponse, ThreadSetNameParams, ThreadSetNameResponse, ThreadStartParams,
    ThreadStartResponse, ThreadTurnsListParams, ThreadTurnsListResponse, TurnInterruptParams,
    TurnInterruptResponse, TurnStartParams, TurnStartResponse, TurnSteerParams, TurnSteerResponse,
    decode_line, encode_error_response, encode_notification, encode_request, encode_response,
};

type PendingResponse = oneshot::Sender<Result<Value, CodexTransportError>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, PendingResponse>>>;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub enum CodexEvent {
    Notification(ServerNotification),
    Request(ServerRequest),
    UnmatchedResponse(ServerResponse),
    ConnectionClosed(String),
}

#[derive(Debug, Error)]
pub enum CodexTransportError {
    #[error("Failed to start Codex app-server: {0}")]
    Spawn(#[source] io::Error),
    #[error("Codex app-server did not expose {0}")]
    MissingPipe(&'static str),
    #[error("Failed to write to Codex app-server: {0}")]
    Write(#[source] io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("Codex rejected the request with {code}: {message}")]
    Remote { code: i64, message: String },
    #[error("Codex returned an invalid result for {method}: {source}")]
    InvalidResult {
        method: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Codex request IDs are exhausted")]
    RequestIdsExhausted,
    #[error("Codex app-server did not initialize within 10 seconds")]
    StartupTimeout,
    #[error("{0}")]
    ConnectionClosed(String),
}

impl From<RemoteError> for CodexTransportError {
    fn from(error: RemoteError) -> Self {
        Self::Remote {
            code: error.code,
            message: error.message,
        }
    }
}

pub struct CodexClient {
    child: Child,
    process_scope: xai_tty_utils::ProcessScope,
    _process_group: Arc<xai_tty_utils::ProcessGroup>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingRequests,
    events: broadcast::Sender<CodexEvent>,
    reader_task: JoinHandle<()>,
    next_id: AtomicI64,
    server_info: InitializeResponse,
}

impl CodexClient {
    pub async fn start(executable: impl AsRef<OsStr>) -> Result<Self, CodexTransportError> {
        Self::start_with_config_overrides(executable, std::iter::empty::<&str>()).await
    }

    pub async fn start_with_config_overrides<I, S>(
        executable: impl AsRef<OsStr>,
        config_overrides: I,
    ) -> Result<Self, CodexTransportError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(executable);
        command.arg("app-server");
        for config_override in config_overrides {
            command.arg("--config").arg(config_override);
        }
        command
            .arg("--listen")
            .arg("stdio://")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let process_scope = xai_tty_utils::ProcessScope::new();
        let (mut child, process_group) = process_scope
            .spawn(command)
            .map_err(CodexTransportError::Spawn)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(CodexTransportError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(CodexTransportError::MissingPipe("stdout"))?;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(1024);
        let reader_task = tokio::spawn(read_messages(
            BufReader::new(stdout),
            pending.clone(),
            events.clone(),
        ));
        let mut client = Self {
            child,
            process_scope,
            _process_group: process_group,
            stdin: Arc::new(Mutex::new(Some(stdin))),
            pending,
            events,
            reader_task,
            next_id: AtomicI64::new(1),
            server_info: InitializeResponse {
                codex_home: Default::default(),
                platform_family: String::new(),
                platform_os: String::new(),
                user_agent: String::new(),
            },
        };
        let params = InitializeParams {
            client_info: crate::ClientInfo {
                name: "bot".to_owned(),
                title: Some("Bot".to_owned()),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: Some(crate::InitializeCapabilities {
                experimental_api: true,
                ..Default::default()
            }),
        };
        client.server_info = timeout(STARTUP_TIMEOUT, client.request(INITIALIZE_METHOD, &params))
            .await
            .map_err(|_| CodexTransportError::StartupTimeout)??;
        client.notify(INITIALIZED_METHOD, &json!({})).await?;
        Ok(client)
    }

    pub fn server_info(&self) -> &InitializeResponse {
        &self.server_info
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CodexEvent> {
        self.events.subscribe()
    }

    pub async fn account(&self) -> Result<AccountReadResponse, CodexTransportError> {
        self.request(ACCOUNT_READ_METHOD, &AccountReadParams::default())
            .await
    }

    pub async fn start_account_login(
        &self,
        params: &LoginAccountParams,
    ) -> Result<LoginAccountResponse, CodexTransportError> {
        self.request(ACCOUNT_LOGIN_START_METHOD, params).await
    }

    pub async fn cancel_account_login(
        &self,
        params: &CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse, CodexTransportError> {
        self.request(ACCOUNT_LOGIN_CANCEL_METHOD, params).await
    }

    pub async fn logout_account(&self) -> Result<LogoutAccountResponse, CodexTransportError> {
        self.request(ACCOUNT_LOGOUT_METHOD, &()).await
    }

    pub async fn account_rate_limits(
        &self,
    ) -> Result<AccountRateLimitsResponse, CodexTransportError> {
        self.request(ACCOUNT_RATE_LIMITS_READ_METHOD, &()).await
    }

    pub async fn account_usage(&self) -> Result<AccountUsageResponse, CodexTransportError> {
        self.request(ACCOUNT_USAGE_READ_METHOD, &()).await
    }

    pub async fn models(
        &self,
        params: &ModelListParams,
    ) -> Result<ModelListResponse, CodexTransportError> {
        self.request(MODEL_LIST_METHOD, params).await
    }

    pub async fn mcp_server_statuses(
        &self,
        params: &McpServerStatusListParams,
    ) -> Result<McpServerStatusListResponse, CodexTransportError> {
        self.request(MCP_SERVER_STATUS_LIST_METHOD, params).await
    }

    pub async fn reload_mcp_servers(
        &self,
    ) -> Result<McpServerRefreshResponse, CodexTransportError> {
        self.request(MCP_SERVER_RELOAD_METHOD, &()).await
    }

    pub async fn hooks(
        &self,
        params: &HooksListParams,
    ) -> Result<HooksListResponse, CodexTransportError> {
        self.request(HOOKS_LIST_METHOD, params).await
    }

    pub async fn plugins(
        &self,
        params: &PluginListParams,
    ) -> Result<PluginListResponse, CodexTransportError> {
        self.request(PLUGIN_LIST_METHOD, params).await
    }

    pub async fn install_plugin(
        &self,
        params: &PluginInstallParams,
    ) -> Result<Value, CodexTransportError> {
        self.request(PLUGIN_INSTALL_METHOD, params).await
    }

    pub async fn uninstall_plugin(
        &self,
        params: &PluginUninstallParams,
    ) -> Result<Value, CodexTransportError> {
        self.request(PLUGIN_UNINSTALL_METHOD, params).await
    }

    pub async fn reconcile_plugins(
        &self,
        params: &PluginReconcileParams,
    ) -> Result<Value, CodexTransportError> {
        self.request(PLUGIN_RECONCILE_METHOD, params).await
    }

    pub async fn write_config_value(
        &self,
        params: &ConfigValueWriteParams,
    ) -> Result<Value, CodexTransportError> {
        self.request(CONFIG_VALUE_WRITE_METHOD, params).await
    }

    pub async fn skills(
        &self,
        params: &SkillsListParams,
    ) -> Result<SkillsListResponse, CodexTransportError> {
        self.request(SKILLS_LIST_METHOD, params).await
    }

    pub async fn write_skill_config(
        &self,
        params: &SkillsConfigWriteParams,
    ) -> Result<SkillsConfigWriteResponse, CodexTransportError> {
        self.request(SKILLS_CONFIG_WRITE_METHOD, params).await
    }

    pub async fn start_thread(
        &self,
        params: &ThreadStartParams,
    ) -> Result<ThreadStartResponse, CodexTransportError> {
        self.request(THREAD_START_METHOD, params).await
    }

    pub async fn list_threads(
        &self,
        params: &ThreadListParams,
    ) -> Result<ThreadListResponse, CodexTransportError> {
        self.request(THREAD_LIST_METHOD, params).await
    }

    pub async fn search_threads(
        &self,
        params: &ThreadSearchParams,
    ) -> Result<ThreadSearchResponse, CodexTransportError> {
        self.request(THREAD_SEARCH_METHOD, params).await
    }

    pub async fn list_thread_turns(
        &self,
        params: &ThreadTurnsListParams,
    ) -> Result<ThreadTurnsListResponse, CodexTransportError> {
        self.request(THREAD_TURNS_LIST_METHOD, params).await
    }

    pub async fn revert_thread(
        &self,
        params: &ThreadRevertParams,
    ) -> Result<ThreadRevertResponse, CodexTransportError> {
        self.request(THREAD_REVERT_METHOD, params).await
    }

    pub async fn resume_thread(
        &self,
        params: &ThreadResumeParams,
    ) -> Result<ThreadResumeResponse, CodexTransportError> {
        self.request(THREAD_RESUME_METHOD, params).await
    }

    pub async fn compact_thread(
        &self,
        params: &ThreadCompactStartParams,
    ) -> Result<ThreadCompactStartResponse, CodexTransportError> {
        self.request(THREAD_COMPACT_START_METHOD, params).await
    }

    pub async fn set_thread_name(
        &self,
        params: &ThreadSetNameParams,
    ) -> Result<ThreadSetNameResponse, CodexTransportError> {
        self.request(THREAD_SET_NAME_METHOD, params).await
    }

    pub async fn delete_thread(
        &self,
        params: &ThreadDeleteParams,
    ) -> Result<ThreadDeleteResponse, CodexTransportError> {
        self.request(THREAD_DELETE_METHOD, params).await
    }

    pub async fn fork_thread(
        &self,
        params: &ThreadForkParams,
    ) -> Result<ThreadForkResponse, CodexTransportError> {
        self.request(THREAD_FORK_METHOD, params).await
    }

    pub async fn start_turn(
        &self,
        params: &TurnStartParams,
    ) -> Result<TurnStartResponse, CodexTransportError> {
        self.request(TURN_START_METHOD, params).await
    }

    pub async fn steer_turn(
        &self,
        params: &TurnSteerParams,
    ) -> Result<TurnSteerResponse, CodexTransportError> {
        self.request(TURN_STEER_METHOD, params).await
    }

    pub async fn interrupt_turn(
        &self,
        params: &TurnInterruptParams,
    ) -> Result<TurnInterruptResponse, CodexTransportError> {
        self.request(TURN_INTERRUPT_METHOD, params).await
    }

    pub async fn clean_background_terminals(
        &self,
        params: &crate::ThreadBackgroundTerminalsCleanParams,
    ) -> Result<(), CodexTransportError> {
        let _: Value = self
            .request(crate::THREAD_BACKGROUND_TERMINALS_CLEAN_METHOD, params)
            .await?;
        Ok(())
    }

    pub async fn request<T, R>(&self, method: &str, params: &T) -> Result<R, CodexTransportError>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let id = self.next_request_id()?;
        let line = encode_request(id.clone(), method, params)?;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);
        if let Err(error) = self.write(line.as_bytes()).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        let value = receiver.await.map_err(|_| {
            CodexTransportError::ConnectionClosed(
                "Codex app-server closed before it returned a response".to_owned(),
            )
        })??;
        serde_json::from_value(value).map_err(|source| CodexTransportError::InvalidResult {
            method: method.to_owned(),
            source,
        })
    }

    pub async fn notify<T: Serialize>(
        &self,
        method: &str,
        params: &T,
    ) -> Result<(), CodexTransportError> {
        let line = encode_notification(method, params)?;
        self.write(line.as_bytes()).await
    }

    pub async fn respond<T: Serialize>(
        &self,
        id: RequestId,
        result: &T,
    ) -> Result<(), CodexTransportError> {
        let line = encode_response(id, result)?;
        self.write(line.as_bytes()).await
    }

    pub async fn respond_error(
        &self,
        id: RequestId,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Result<(), CodexTransportError> {
        let line = encode_error_response(
            id,
            &RemoteError {
                code,
                message: message.into(),
                data,
            },
        )?;
        self.write(line.as_bytes()).await
    }

    pub async fn shutdown(mut self) -> Result<(), io::Error> {
        self.close().await?;
        self.child.wait().await.map(|_| ())
    }

    pub async fn close(&self) -> Result<(), io::Error> {
        let mut events = self.subscribe();
        if self.reader_task.is_finished() {
            return Ok(());
        }
        self.stdin.lock().await.take();
        timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(CodexEvent::ConnectionClosed(_))
                    | Err(broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Codex shutdown timed out"))
    }

    fn next_request_id(&self) -> Result<RequestId, CodexTransportError> {
        self.next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(RequestId::Number)
            .map_err(|_| CodexTransportError::RequestIdsExhausted)
    }

    async fn write(&self, bytes: &[u8]) -> Result<(), CodexTransportError> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or_else(|| {
            CodexTransportError::Write(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Codex input is closed",
            ))
        })?;
        stdin
            .write_all(bytes)
            .await
            .map_err(CodexTransportError::Write)?;
        stdin.flush().await.map_err(CodexTransportError::Write)
    }
}

impl Drop for CodexClient {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.process_scope.kill_all();
    }
}

async fn read_messages<R>(
    mut reader: R,
    pending: PendingRequests,
    events: broadcast::Sender<CodexEvent>,
) where
    R: AsyncBufRead + Unpin,
{
    let failure = loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => break "Codex app-server closed its output".to_owned(),
            Ok(_) => match decode_line(&line) {
                Ok(message) => dispatch_message(message, &pending, &events).await,
                Err(error) => break error.to_string(),
            },
            Err(error) => break format!("Failed to read from Codex app-server: {error}"),
        }
    };
    let _ = events.send(CodexEvent::ConnectionClosed(failure.clone()));
    for (_, sender) in pending.lock().await.drain() {
        let _ = sender.send(Err(CodexTransportError::ConnectionClosed(failure.clone())));
    }
}

async fn dispatch_message(
    message: IncomingMessage,
    pending: &PendingRequests,
    events: &broadcast::Sender<CodexEvent>,
) {
    match message {
        IncomingMessage::Response(response) => {
            let sender = pending.lock().await.remove(&response.id);
            if let Some(sender) = sender {
                let result = response.outcome.map_err(CodexTransportError::from);
                let _ = sender.send(result);
            } else {
                let _ = events.send(CodexEvent::UnmatchedResponse(response));
            }
        }
        IncomingMessage::Notification(notification) => {
            let _ = events.send(CodexEvent::Notification(notification));
        }
        IncomingMessage::Request(request) => {
            let _ = events.send(CodexEvent::Request(request));
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, BufReader, duplex};

    use super::*;

    #[tokio::test]
    async fn matches_responses_to_pending_requests() {
        let (mut writer, reader) = duplex(256);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(8);
        let task = tokio::spawn(read_messages(
            BufReader::new(reader),
            pending.clone(),
            events,
        ));
        let (sender, receiver) = oneshot::channel();
        pending.lock().await.insert(RequestId::Number(4), sender);
        writer
            .write_all(b"{\"id\":4,\"result\":{\"ready\":true}}\n")
            .await
            .expect("response");
        let value = receiver.await.expect("channel").expect("result");
        assert_eq!(value["ready"], true);
        task.abort();
    }

    #[tokio::test]
    async fn forwards_notifications_without_interpreting_them() {
        let (mut writer, reader) = duplex(256);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(8);
        let mut receiver = events.subscribe();
        let task = tokio::spawn(read_messages(BufReader::new(reader), pending, events));
        writer
            .write_all(b"{\"method\":\"future/event\",\"params\":{\"value\":8}}\n")
            .await
            .expect("notification");
        let event = receiver.recv().await.expect("event");
        let CodexEvent::Notification(notification) = event else {
            panic!("notification");
        };
        assert_eq!(notification.method, "future/event");
        assert_eq!(notification.params["value"], 8);
        task.abort();
    }

    #[tokio::test]
    async fn closes_pending_requests_after_malformed_input() {
        let (mut writer, reader) = duplex(256);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(8);
        let mut event_receiver = events.subscribe();
        let task = tokio::spawn(read_messages(
            BufReader::new(reader),
            pending.clone(),
            events,
        ));
        let (sender, response_receiver) = oneshot::channel();
        pending.lock().await.insert(RequestId::Number(2), sender);
        writer.write_all(b"not-json\n").await.expect("input");
        let error = response_receiver
            .await
            .expect("channel")
            .expect_err("closed request");
        assert!(matches!(error, CodexTransportError::ConnectionClosed(_)));
        let event = event_receiver.recv().await.expect("event");
        assert!(matches!(event, CodexEvent::ConnectionClosed(_)));
        task.await.expect("reader task");
    }
}
