use std::path::PathBuf;

use crate::{AccountProfileId, ProviderId, SessionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Starting,
    Ready,
    Running,
    AwaitingApproval,
    Failed(String),
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    pub id: SessionId,
    pub provider: ProviderId,
    pub account_profile_id: AccountProfileId,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub status: SessionStatus,
}

impl SessionState {
    pub fn new(
        id: SessionId,
        provider: ProviderId,
        account_profile_id: AccountProfileId,
        workspace: PathBuf,
    ) -> Self {
        Self {
            id,
            provider,
            account_profile_id,
            workspace,
            model: None,
            effort: None,
            status: SessionStatus::Starting,
        }
    }

    pub fn mark_ready(&mut self) {
        self.status = SessionStatus::Ready;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_before_provider_readiness() {
        let state = SessionState::new(
            SessionId::new("session-1").expect("valid session ID"),
            ProviderId::Codex,
            AccountProfileId::new("codex-personal").expect("valid account profile ID"),
            PathBuf::from("/workspace"),
        );
        assert_eq!(state.status, SessionStatus::Starting);
    }

    #[test]
    fn records_provider_readiness() {
        let mut state = SessionState::new(
            SessionId::new("session-1").expect("valid session ID"),
            ProviderId::Codex,
            AccountProfileId::new("codex-personal").expect("valid account profile ID"),
            PathBuf::from("/workspace"),
        );
        state.mark_ready();
        assert_eq!(state.status, SessionStatus::Ready);
    }
}
