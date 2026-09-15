use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub const COMMAND_OWNERSHIP_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Skill,
    Workflow,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOwnership {
    version: u16,
    provider: String,
    owner: String,
    kind: CommandKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandOwnershipWire {
    version: u16,
    provider: String,
    owner: String,
    kind: CommandKind,
}

impl<'de> Deserialize<'de> for CommandOwnership {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CommandOwnershipWire::deserialize(deserializer)?;
        Self::new(wire.version, wire.provider, wire.owner, wire.kind)
            .map_err(serde::de::Error::custom)
    }
}

impl CommandOwnership {
    pub fn new(
        version: u16,
        provider: impl Into<String>,
        owner: impl Into<String>,
        kind: CommandKind,
    ) -> Result<Self, CommandOwnershipError> {
        if version != COMMAND_OWNERSHIP_VERSION {
            return Err(CommandOwnershipError::UnsupportedVersion(version));
        }
        let provider = provider.into();
        let provider = provider.trim();
        if provider.is_empty() {
            return Err(CommandOwnershipError::EmptyProvider);
        }
        let owner = owner.into();
        let owner = owner.trim();
        if owner.is_empty() {
            return Err(CommandOwnershipError::EmptyOwner);
        }
        Ok(Self {
            version,
            provider: provider.to_owned(),
            owner: owner.to_owned(),
            kind,
        })
    }

    pub fn current(
        provider: impl Into<String>,
        owner: impl Into<String>,
        kind: CommandKind,
    ) -> Result<Self, CommandOwnershipError> {
        Self::new(COMMAND_OWNERSHIP_VERSION, provider, owner, kind)
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn kind(&self) -> CommandKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CommandOwnershipError {
    #[error("Unsupported command ownership version: {0}")]
    UnsupportedVersion(u16),
    #[error("Command provider cannot be empty")]
    EmptyProvider,
    #[error("Command owner cannot be empty")]
    EmptyOwner,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_command_ownership() {
        let ownership = CommandOwnership::current("codex", "repo", CommandKind::Skill).unwrap();
        assert_eq!(
            serde_json::to_value(ownership).unwrap(),
            serde_json::json!({
                "version": 1,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill"
            })
        );
    }

    #[test]
    fn rejects_untrusted_versions_and_empty_owners() {
        assert_eq!(
            CommandOwnership::new(2, "codex", "repo", CommandKind::Skill),
            Err(CommandOwnershipError::UnsupportedVersion(2))
        );
        assert_eq!(
            CommandOwnership::current("codex", " ", CommandKind::Skill),
            Err(CommandOwnershipError::EmptyOwner)
        );
        assert!(
            serde_json::from_value::<CommandOwnership>(serde_json::json!({
                "version": 9,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill"
            }))
            .is_err()
        );
    }
}
