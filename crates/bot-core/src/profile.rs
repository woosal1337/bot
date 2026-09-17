use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::AccountProfileId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Codex,
    Grok,
    Gemini,
    Claude,
    Custom(String),
}

impl ProviderId {
    pub fn label(&self) -> &str {
        match self {
            Self::Codex => "Codex",
            Self::Grok => "Grok",
            Self::Gemini => "Gemini",
            Self::Claude => "Claude",
            Self::Custom(label) => label,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Custom(key) => key,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "grok" | "xai" => Some(Self::Grok),
            "codex" | "openai" | "chatgpt" => Some(Self::Codex),
            "gemini" | "google" => Some(Self::Gemini),
            "claude" | "anthropic" | "claude-code" => Some(Self::Claude),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputModality {
    Text,
    Image,
    Audio,
    Skill,
    Mention,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageAttachment {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ProviderCapability {
    Input(InputModality),
    ModelSelection,
    EffortSelection,
    SessionResume,
    SessionFork,
    Tools,
    FileChanges,
    Approvals,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub supported_efforts: Vec<String>,
    pub input_modalities: BTreeSet<InputModality>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginStatus {
    Unknown,
    SignedOut,
    SigningIn,
    SignedIn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountProfile {
    pub id: AccountProfileId,
    pub provider: ProviderId,
    pub label: String,
    pub account_name: Option<String>,
    pub login_status: LoginStatus,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_aliases_resolve_to_canonical_ids() {
        assert_eq!(ProviderId::parse("xai"), Some(ProviderId::Grok));
        assert_eq!(ProviderId::parse("openai"), Some(ProviderId::Codex));
        assert_eq!(ProviderId::parse("chatgpt"), Some(ProviderId::Codex));
        assert_eq!(ProviderId::parse("google"), Some(ProviderId::Gemini));
        assert_eq!(ProviderId::parse("anthropic"), Some(ProviderId::Claude));
    }
}
