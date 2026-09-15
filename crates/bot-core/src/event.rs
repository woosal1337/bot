use crate::{EventId, SessionId, TurnId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolCallState {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolCallKind {
    Command,
    FileChange,
    Mcp,
    WebSearch,
    Image,
    Collaboration,
    Wait,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub kind: ToolCallKind,
    pub title: String,
    pub detail: Option<String>,
    pub output: Option<String>,
    pub state: ToolCallState,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalKind {
    Command,
    FileChange,
    Permission,
    Tool,
    Question,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalState {
    Pending,
    Sending,
    Approved,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEventKind {
    Notification,
    Request,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    TurnStarted {
        session_id: SessionId,
        turn_id: TurnId,
    },
    TurnCompleted {
        session_id: SessionId,
        turn_id: TurnId,
        outcome: TurnOutcome,
    },
    UserMessage {
        id: EventId,
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
    },
    AgentTextDelta {
        id: EventId,
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
    },
    ReasoningSummaryDelta {
        id: EventId,
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
    },
    ToolCallChanged {
        id: EventId,
        session_id: SessionId,
        turn_id: TurnId,
        call: ToolCall,
    },
    ApprovalRequested {
        id: EventId,
        session_id: SessionId,
        turn_id: TurnId,
        title: String,
        kind: ApprovalKind,
    },
    ApprovalResolved {
        id: EventId,
        session_id: SessionId,
        decision: ApprovalDecision,
    },
    UsageChanged {
        session_id: SessionId,
        usage: Usage,
    },
    ProviderEvent {
        session_id: SessionId,
        kind: ProviderEventKind,
        name: String,
        payload: String,
    },
    Warning {
        session_id: SessionId,
        message: String,
    },
    Error {
        session_id: SessionId,
        message: String,
    },
}
