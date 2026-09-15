use std::collections::BTreeSet;

use bot_core::ProviderId;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub const EXTENSION_CONTRACT_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCapability {
    Hooks,
    Plugins,
    PluginUpdates,
    Marketplace,
    Skills,
    Workflows,
    McpServers,
    McpMutations,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionContract {
    api_version: u16,
    capabilities: BTreeSet<ExtensionCapability>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionContractWire {
    api_version: u16,
    capabilities: BTreeSet<ExtensionCapability>,
}

impl<'de> Deserialize<'de> for ExtensionContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ExtensionContractWire::deserialize(deserializer)?;
        Self::new(wire.api_version, wire.capabilities).map_err(serde::de::Error::custom)
    }
}

impl ExtensionContract {
    pub fn new(
        api_version: u16,
        capabilities: impl IntoIterator<Item = ExtensionCapability>,
    ) -> Result<Self, ExtensionContractError> {
        if api_version != EXTENSION_CONTRACT_VERSION {
            return Err(ExtensionContractError::UnsupportedVersion(api_version));
        }
        let contract = Self {
            api_version,
            capabilities: capabilities.into_iter().collect(),
        };
        contract.validate_dependencies()?;
        Ok(contract)
    }

    pub fn api_version(&self) -> u16 {
        self.api_version
    }

    pub fn capabilities(&self) -> &BTreeSet<ExtensionCapability> {
        &self.capabilities
    }

    pub fn supports(&self, capability: ExtensionCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    pub fn validate(&self) -> Result<(), ExtensionContractError> {
        if self.api_version != EXTENSION_CONTRACT_VERSION {
            return Err(ExtensionContractError::UnsupportedVersion(self.api_version));
        }
        self.validate_dependencies()
    }

    fn validate_dependencies(&self) -> Result<(), ExtensionContractError> {
        for (capability, required) in [
            (
                ExtensionCapability::PluginUpdates,
                ExtensionCapability::Plugins,
            ),
            (
                ExtensionCapability::Marketplace,
                ExtensionCapability::Plugins,
            ),
            (
                ExtensionCapability::McpMutations,
                ExtensionCapability::McpServers,
            ),
        ] {
            if self.supports(capability) && !self.supports(required) {
                return Err(ExtensionContractError::MissingCapability {
                    capability,
                    required,
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExtensionContractError {
    #[error("Unsupported extension contract version: {0}")]
    UnsupportedVersion(u16),
    #[error("Extension capability {capability:?} requires {required:?}")]
    MissingCapability {
        capability: ExtensionCapability,
        required: ExtensionCapability,
    },
}

pub fn extension_contract(provider: &ProviderId) -> ExtensionContract {
    use ExtensionCapability as Capability;

    let capabilities = match provider {
        ProviderId::Grok => vec![
            Capability::Hooks,
            Capability::Plugins,
            Capability::PluginUpdates,
            Capability::Marketplace,
            Capability::Skills,
            Capability::Workflows,
            Capability::McpServers,
            Capability::McpMutations,
        ],
        ProviderId::Codex => vec![
            Capability::Hooks,
            Capability::Plugins,
            Capability::Skills,
            Capability::McpServers,
        ],
        ProviderId::Gemini | ProviderId::Claude | ProviderId::Custom(_) => Vec::new(),
    };
    ExtensionContract {
        api_version: EXTENSION_CONTRACT_VERSION,
        capabilities: capabilities.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_provider_capabilities_at_the_current_version() {
        let grok = extension_contract(&ProviderId::Grok);
        assert_eq!(grok.api_version(), EXTENSION_CONTRACT_VERSION);
        assert!(grok.supports(ExtensionCapability::Workflows));
        assert!(grok.supports(ExtensionCapability::PluginUpdates));

        let codex = extension_contract(&ProviderId::Codex);
        assert_eq!(codex.api_version(), EXTENSION_CONTRACT_VERSION);
        assert!(codex.supports(ExtensionCapability::Plugins));
        assert!(codex.supports(ExtensionCapability::Skills));
        assert!(!codex.supports(ExtensionCapability::Workflows));
        assert!(!codex.supports(ExtensionCapability::PluginUpdates));

        assert!(
            extension_contract(&ProviderId::Claude)
                .capabilities()
                .is_empty()
        );
    }

    #[test]
    fn rejects_an_unsupported_contract_version() {
        assert_eq!(
            ExtensionContract::new(EXTENSION_CONTRACT_VERSION + 1, []),
            Err(ExtensionContractError::UnsupportedVersion(
                EXTENSION_CONTRACT_VERSION + 1
            ))
        );
        assert!(
            serde_json::from_value::<ExtensionContract>(serde_json::json!({
                "apiVersion": EXTENSION_CONTRACT_VERSION + 1,
                "capabilities": []
            }))
            .is_err()
        );
    }

    #[test]
    fn rejects_capabilities_without_their_inventory_surface() {
        assert_eq!(
            ExtensionContract::new(
                EXTENSION_CONTRACT_VERSION,
                [ExtensionCapability::PluginUpdates]
            ),
            Err(ExtensionContractError::MissingCapability {
                capability: ExtensionCapability::PluginUpdates,
                required: ExtensionCapability::Plugins,
            })
        );
        assert_eq!(
            ExtensionContract::new(
                EXTENSION_CONTRACT_VERSION,
                [ExtensionCapability::McpMutations]
            ),
            Err(ExtensionContractError::MissingCapability {
                capability: ExtensionCapability::McpMutations,
                required: ExtensionCapability::McpServers,
            })
        );
    }

    #[test]
    fn serializes_a_stable_provider_neutral_manifest() {
        let contract = extension_contract(&ProviderId::Codex);
        assert_eq!(
            serde_json::to_value(contract).unwrap(),
            serde_json::json!({
                "apiVersion": 1,
                "capabilities": ["hooks", "plugins", "skills", "mcp_servers"]
            })
        );
    }
}
