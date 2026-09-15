#![forbid(unsafe_code)]

mod diff;
mod message;
mod normalize;
mod protocol;
mod transport;

pub use diff::{CodexDiffDetail, CodexFileDiff, parse_unified_diff, split_turn_diff};
pub use message::{
    IncomingMessage, ProtocolError, RemoteError, RequestId, ServerNotification, ServerRequest,
    ServerResponse, decode_line, encode_error_response, encode_notification, encode_request,
    encode_response,
};
pub use normalize::{CodexEventNormalizer, historical_tool_call};
pub use protocol::{
    Account, AccountLoginCompletedNotification, AccountRateLimitsResponse, AccountReadParams,
    AccountReadResponse, AccountUpdatedNotification, CancelLoginAccountParams,
    CancelLoginAccountResponse, CancelLoginAccountStatus, ClientInfo, CollaborationMode,
    CollaborationModeKind, CollaborationModeSettings, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalResponse, ConfigValueWriteParams, CreditsSnapshot,
    DynamicToolCallOutputContentItem, DynamicToolCallParams, DynamicToolCallResponse,
    DynamicToolNamespaceTool, DynamicToolSpec, FileChangeApprovalDecision,
    FileChangeRequestApprovalResponse, HookErrorInfo, HookEventName, HookHandlerMetadata,
    HookMetadata, HookSource, HookTrustStatus, HooksListEntry, HooksListParams, HooksListResponse,
    ImageDetail, InitializeCapabilities, InitializeParams, InitializeResponse, InputModality,
    LoginAccountParams, LoginAccountResponse, LoginAppBrand, LogoutAccountResponse, McpAuthStatus,
    McpServerConnectionStatus, McpServerElicitationAction, McpServerElicitationRequest,
    McpServerElicitationRequestParams, McpServerElicitationRequestResponse, McpServerInfo,
    McpServerRefreshResponse, McpServerStatus, McpServerStatusDetail, McpServerStatusListParams,
    McpServerStatusListResponse, McpTool, MergeStrategy, Model, ModelListParams, ModelListResponse,
    NetworkPolicyAmendment, NetworkPolicyRuleAction, PermissionGrantScope,
    PermissionsRequestApprovalResponse, PluginInstallParams, PluginInterface, PluginListParams,
    PluginListResponse, PluginMarketplace, PluginReconcileParams, PluginSource, PluginSummary,
    PluginUninstallParams, RateLimitResetCredit, RateLimitResetCreditsSummary, RateLimitSnapshot,
    RateLimitWindow, ReasoningEffortOption, ReasoningSummary, SkillErrorInfo, SkillInterface,
    SkillMetadata, SkillScope, SkillsConfigWriteParams, SkillsConfigWriteResponse, SkillsListEntry,
    SkillsListParams, SkillsListResponse, SpendControlLimitSnapshot, Thread,
    ThreadBackgroundTerminalsCleanParams, ThreadCompactStartParams, ThreadCompactStartResponse,
    ThreadDeleteParams, ThreadDeleteResponse, ThreadForkParams, ThreadForkResponse,
    ThreadListParams, ThreadListResponse, ThreadResumeParams, ThreadResumeResponse,
    ThreadRevertParams, ThreadRevertResponse, ThreadSearchParams, ThreadSearchResponse,
    ThreadSearchResult, ThreadSetNameParams, ThreadSetNameResponse, ThreadStartParams,
    ThreadStartResponse, ThreadTurnsListParams, ThreadTurnsListResponse,
    ToolRequestUserInputAnswer, ToolRequestUserInputOption, ToolRequestUserInputParams,
    ToolRequestUserInputQuestion, ToolRequestUserInputResponse, Turn, TurnDiffUpdatedNotification,
    TurnInterruptParams, TurnInterruptResponse, TurnStartParams, TurnStartResponse,
    TurnSteerParams, TurnSteerResponse, UserInput,
};
pub use transport::{CodexClient, CodexEvent, CodexTransportError};

pub const INITIALIZE_METHOD: &str = "initialize";
pub const INITIALIZED_METHOD: &str = "initialized";
pub const ACCOUNT_READ_METHOD: &str = "account/read";
pub const ACCOUNT_LOGIN_START_METHOD: &str = "account/login/start";
pub const ACCOUNT_LOGIN_CANCEL_METHOD: &str = "account/login/cancel";
pub const ACCOUNT_LOGIN_COMPLETED_NOTIFICATION: &str = "account/login/completed";
pub const ACCOUNT_LOGOUT_METHOD: &str = "account/logout";
pub const ACCOUNT_RATE_LIMITS_READ_METHOD: &str = "account/rateLimits/read";
pub const ACCOUNT_RATE_LIMITS_UPDATED_NOTIFICATION: &str = "account/rateLimits/updated";
pub const ACCOUNT_UPDATED_NOTIFICATION: &str = "account/updated";
pub const MODEL_LIST_METHOD: &str = "model/list";
pub const ITEM_TOOL_CALL_METHOD: &str = "item/tool/call";
pub const MCP_SERVER_RELOAD_METHOD: &str = "config/mcpServer/reload";
pub const MCP_SERVER_STATUS_LIST_METHOD: &str = "mcpServerStatus/list";
pub const HOOKS_LIST_METHOD: &str = "hooks/list";
pub const CONFIG_VALUE_WRITE_METHOD: &str = "config/value/write";
pub const PLUGIN_INSTALL_METHOD: &str = "plugin/install";
pub const PLUGIN_LIST_METHOD: &str = "plugin/list";
pub const PLUGIN_RECONCILE_METHOD: &str = "plugin/reconcile";
pub const PLUGIN_UNINSTALL_METHOD: &str = "plugin/uninstall";
pub const SKILLS_CONFIG_WRITE_METHOD: &str = "skills/config/write";
pub const SKILLS_LIST_METHOD: &str = "skills/list";
pub const THREAD_LIST_METHOD: &str = "thread/list";
pub const THREAD_SEARCH_METHOD: &str = "thread/search";
pub const THREAD_RESUME_METHOD: &str = "thread/resume";
pub const THREAD_REVERT_METHOD: &str = "thread/revert";
pub const THREAD_START_METHOD: &str = "thread/start";
pub const THREAD_TURNS_LIST_METHOD: &str = "thread/turns/list";
pub const THREAD_COMPACT_START_METHOD: &str = "thread/compact/start";
pub const THREAD_DELETE_METHOD: &str = "thread/delete";
pub const THREAD_FORK_METHOD: &str = "thread/fork";
pub const THREAD_SET_NAME_METHOD: &str = "thread/name/set";
pub const TURN_INTERRUPT_METHOD: &str = "turn/interrupt";
pub const TURN_START_METHOD: &str = "turn/start";
pub const TURN_STEER_METHOD: &str = "turn/steer";
pub const THREAD_BACKGROUND_TERMINALS_CLEAN_METHOD: &str = "thread/backgroundTerminals/clean";
