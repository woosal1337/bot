// Modified by the Bot project on 2026-09-13: provider-neutral extension capabilities.
#![deny(unsafe_code)]

use std::sync::OnceLock;

pub use bot_core::ProviderId;
use bot_provider::{
    ExtensionCapability, ProviderDescriptor, SupportLevel, discover_providers, extension_contract,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterStatus {
    Ready,
    Installed,
    NotInstalled,
    PolicyGated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderState {
    pub descriptor: ProviderDescriptor,
    pub status: AdapterStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionTerm {
    pub kind: &'static str,
    pub keyword: &'static str,
    pub display: &'static str,
    pub description: &'static str,
    aliases: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtensionCapabilities {
    pub hooks: bool,
    pub plugins: bool,
    pub plugin_updates: bool,
    pub marketplace: bool,
    pub skills: bool,
    pub workflows: bool,
    pub mcp_servers: bool,
    pub mcp_mutations: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsCapabilities {
    pub remember_tool_approvals: bool,
    pub ask_user_question_timeout: bool,
    pub prompt_suggestions: bool,
    pub hunk_tracker: bool,
    pub fork_secondary_model: bool,
}

const GROK_SETTINGS_CAPABILITIES: SettingsCapabilities = SettingsCapabilities {
    remember_tool_approvals: true,
    ask_user_question_timeout: true,
    prompt_suggestions: true,
    hunk_tracker: true,
    fork_secondary_model: true,
};

const COMMON_SETTINGS_CAPABILITIES: SettingsCapabilities = SettingsCapabilities {
    remember_tool_approvals: false,
    ask_user_question_timeout: false,
    prompt_suggestions: false,
    hunk_tracker: false,
    fork_secondary_model: false,
};

const GROK_PERMISSION_TERMS: &[PermissionTerm] = &[
    PermissionTerm {
        kind: "ask",
        keyword: "ask",
        display: "Ask",
        description: "Ask before tools that need permission.",
        aliases: &["default", "normal"],
    },
    PermissionTerm {
        kind: "auto",
        keyword: "auto",
        display: "Auto",
        description: "Let the Grok classifier review tool requests.",
        aliases: &[],
    },
    PermissionTerm {
        kind: "always-approve",
        keyword: "always-approve",
        display: "Always-approve",
        description: "Skip approval prompts. Deny rules, hooks, and the sandbox still apply.",
        aliases: &["yolo"],
    },
];

const CODEX_PERMISSION_TERMS: &[PermissionTerm] = &[
    PermissionTerm {
        kind: "ask",
        keyword: "ask-for-approval",
        display: "Ask for approval",
        description: "Work inside the workspace sandbox. Ask before extra access.",
        aliases: &["ask", "default", "workspace", ":workspace"],
    },
    PermissionTerm {
        kind: "auto",
        keyword: "approve-for-me",
        display: "Approve for me",
        description: "Let Codex review requests for extra access. Keep the workspace sandbox.",
        aliases: &["auto"],
    },
    PermissionTerm {
        kind: "always-approve",
        keyword: "full-access",
        display: "Full Access",
        description: "Disable approval prompts and the sandbox. Tools can access the whole machine.",
        aliases: &[
            "always-approve",
            "danger-full-access",
            ":danger-full-access",
            "bypasspermissions",
        ],
    },
    PermissionTerm {
        kind: "read-only",
        keyword: "read-only",
        display: "Read Only",
        description: "Read files. Request approval for changes or access outside the sandbox.",
        aliases: &[":read-only"],
    },
];

pub const PERMISSION_MODE_CLI_VALUES: &[&str] = &[
    "ask",
    "auto",
    "always-approve",
    "default",
    "ask-for-approval",
    "approve-for-me",
    "full-access",
    "read-only",
    "bypassPermissions",
    "acceptEdits",
];

pub fn permission_terms(provider: &ProviderId) -> &'static [PermissionTerm] {
    match provider {
        ProviderId::Codex => CODEX_PERMISSION_TERMS,
        _ => GROK_PERMISSION_TERMS,
    }
}

pub fn active_permission_terms() -> &'static [PermissionTerm] {
    permission_terms(&active_provider())
}

pub fn extension_capabilities(provider: &ProviderId) -> ExtensionCapabilities {
    let contract = extension_contract(provider);
    ExtensionCapabilities {
        hooks: contract.supports(ExtensionCapability::Hooks),
        plugins: contract.supports(ExtensionCapability::Plugins),
        plugin_updates: contract.supports(ExtensionCapability::PluginUpdates),
        marketplace: contract.supports(ExtensionCapability::Marketplace),
        skills: contract.supports(ExtensionCapability::Skills),
        workflows: contract.supports(ExtensionCapability::Workflows),
        mcp_servers: contract.supports(ExtensionCapability::McpServers),
        mcp_mutations: contract.supports(ExtensionCapability::McpMutations),
    }
}

pub fn active_extension_capabilities() -> ExtensionCapabilities {
    extension_capabilities(&active_provider())
}

pub fn settings_capabilities(provider: &ProviderId) -> SettingsCapabilities {
    match provider {
        ProviderId::Grok => GROK_SETTINGS_CAPABILITIES,
        ProviderId::Codex | ProviderId::Gemini | ProviderId::Claude | ProviderId::Custom(_) => {
            COMMON_SETTINGS_CAPABILITIES
        }
    }
}

pub fn parse_active_permission_term(value: &str) -> Option<&'static PermissionTerm> {
    parse_permission_term(&active_provider(), value)
}

pub fn parse_permission_term(
    provider: &ProviderId,
    value: &str,
) -> Option<&'static PermissionTerm> {
    let value = value.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    permission_terms(provider).iter().find(|term| {
        term.keyword == value
            || term.kind == value
            || term.aliases.iter().any(|alias| *alias == value)
    })
}

pub fn active_permission_keyword(kind: &'static str) -> &'static str {
    active_permission_terms()
        .iter()
        .find(|term| term.kind == kind)
        .map(|term| term.keyword)
        .unwrap_or(kind)
}

pub fn active_permission_display(kind: &'static str) -> &'static str {
    active_permission_terms()
        .iter()
        .find(|term| term.kind == kind)
        .map(|term| term.display)
        .unwrap_or(kind)
}

impl ProviderState {
    pub fn id(&self) -> &ProviderId {
        self.descriptor.id()
    }

    pub fn display_name(&self) -> &str {
        self.id().label()
    }

    pub fn key(&self) -> &str {
        self.id().key()
    }
}

static PROVIDERS: OnceLock<Vec<ProviderState>> = OnceLock::new();
static ACTIVE_PROVIDER: OnceLock<ProviderId> = OnceLock::new();

pub fn providers() -> &'static [ProviderState] {
    PROVIDERS.get_or_init(|| {
        discover_providers()
            .into_iter()
            .map(|provider| {
                let status = match (provider.id(), provider.support(), provider.is_installed()) {
                    (ProviderId::Grok, SupportLevel::Ready, _) => AdapterStatus::Ready,
                    (ProviderId::Codex, _, true) => AdapterStatus::Ready,
                    (ProviderId::Claude, _, _) => AdapterStatus::PolicyGated,
                    (_, _, true) => AdapterStatus::Installed,
                    _ => AdapterStatus::NotInstalled,
                };
                ProviderState {
                    descriptor: provider,
                    status,
                }
            })
            .collect()
    })
}

pub fn set_active_provider(provider: ProviderId) {
    let _ = ACTIVE_PROVIDER.set(provider);
}

pub fn active_provider() -> ProviderId {
    ACTIVE_PROVIDER.get().cloned().unwrap_or(ProviderId::Grok)
}

pub fn provider(id: &ProviderId) -> Option<&'static ProviderState> {
    providers().iter().find(|provider| provider.id() == id)
}

pub fn require_ready(key: &str) -> anyhow::Result<&'static ProviderState> {
    let id = ProviderId::parse(key).ok_or_else(|| anyhow::anyhow!("Unknown provider: {key}"))?;
    let provider =
        provider(&id).ok_or_else(|| anyhow::anyhow!("Provider is not registered: {key}"))?;
    match provider.status {
        AdapterStatus::Ready => Ok(provider),
        AdapterStatus::NotInstalled => {
            anyhow::bail!("{} CLI was not found", provider.display_name())
        }
        AdapterStatus::Installed | AdapterStatus::PolicyGated => {
            anyhow::bail!("{} adapter is not connected yet", provider.display_name())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_grok_runtime_is_ready_without_a_path_lookup() {
        assert_eq!(
            provider(&ProviderId::Grok).map(|provider| provider.status),
            Some(AdapterStatus::Ready)
        );
    }

    #[test]
    fn provider_catalog_keeps_the_supported_agents() {
        let ids = providers()
            .iter()
            .map(ProviderState::id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                &ProviderId::Grok,
                &ProviderId::Codex,
                &ProviderId::Gemini,
                &ProviderId::Claude
            ]
        );
    }

    #[test]
    fn launch_rejects_unknown_and_unconnected_providers() {
        assert!(
            require_ready("unknown")
                .unwrap_err()
                .to_string()
                .contains("Unknown provider")
        );
        assert!(
            require_ready("claude")
                .unwrap_err()
                .to_string()
                .contains("not connected")
        );
        assert_eq!(require_ready("grok").unwrap().id(), &ProviderId::Grok);
    }

    #[test]
    fn permission_terms_use_each_provider_native_keywords() {
        assert_eq!(
            permission_terms(&ProviderId::Grok)
                .iter()
                .map(|term| term.keyword)
                .collect::<Vec<_>>(),
            ["ask", "auto", "always-approve"]
        );
        assert_eq!(
            permission_terms(&ProviderId::Codex)
                .iter()
                .map(|term| term.keyword)
                .collect::<Vec<_>>(),
            [
                "ask-for-approval",
                "approve-for-me",
                "full-access",
                "read-only"
            ]
        );
        assert_eq!(
            parse_permission_term(&ProviderId::Codex, "bypassPermissions").map(|term| term.keyword),
            Some("full-access")
        );
        assert!(parse_permission_term(&ProviderId::Grok, "full-access").is_none());
    }

    #[test]
    fn extension_capabilities_match_provider_protocols() {
        let grok = extension_capabilities(&ProviderId::Grok);
        assert!(grok.marketplace);
        assert!(grok.workflows);
        assert!(grok.mcp_mutations);

        let codex = extension_capabilities(&ProviderId::Codex);
        assert!(codex.hooks);
        assert!(codex.plugins);
        assert!(codex.skills);
        assert!(codex.mcp_servers);
        assert!(!codex.plugin_updates);
        assert!(!codex.marketplace);
        assert!(!codex.workflows);
        assert!(!codex.mcp_mutations);

        assert_eq!(
            extension_capabilities(&ProviderId::Claude),
            ExtensionCapabilities {
                hooks: false,
                plugins: false,
                plugin_updates: false,
                marketplace: false,
                skills: false,
                workflows: false,
                mcp_servers: false,
                mcp_mutations: false,
            }
        );
    }
}
