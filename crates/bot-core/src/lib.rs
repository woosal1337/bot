#![forbid(unsafe_code)]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod action;
mod event;
mod id;
mod profile;
mod session;
mod usage;

pub use action::Action;
pub use event::{
    AgentEvent, ApprovalDecision, ApprovalKind, ApprovalState, ProviderEventKind, ToolCall,
    ToolCallKind, ToolCallState, TurnOutcome, Usage,
};
pub use id::{AccountProfileId, EventId, IdError, SessionId, TurnId};
pub use profile::{
    AccountProfile, ImageAttachment, InputModality, LoginStatus, ModelInfo, ProviderCapability,
    ProviderId,
};
pub use session::{SessionState, SessionStatus};
pub use usage::{
    PROVIDER_USAGE_UPDATED_METHOD, ProviderUsage, ProviderUsageUpdate, UsageLimit, UsageLimitWindow,
};
