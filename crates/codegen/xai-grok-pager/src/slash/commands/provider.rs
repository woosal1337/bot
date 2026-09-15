// Modified by the Bot project on 2026-09-13: Removed announcement fixture state.
#![deny(unsafe_code)]

use crate::provider::{AdapterStatus, ProviderId, active_provider, provider, providers};
use crate::slash::command::{
    AppCtx, ArgItem, CommandExecCtx, CommandResult, SlashCommand, slash_meta,
};

pub struct ProviderCommand;

impl SlashCommand for ProviderCommand {
    slash_meta! {
        name: "provider",
        aliases: ["agent"],
        description: "Choose the agent provider",
        usage: "/provider [name]",
        takes_args: true,
        args_required: false,
        arg_placeholder: "<provider>",
    }

    fn suggest_args(&self, _ctx: &AppCtx, _args_query: &str) -> Option<Vec<ArgItem>> {
        Some(
            providers()
                .iter()
                .map(|provider| {
                    let active = provider.id() == &active_provider();
                    let display = if active {
                        format!("{} (active)", provider.display_name())
                    } else {
                        provider.display_name().to_string()
                    };
                    let description = match provider.status {
                        AdapterStatus::Ready => "Ready".to_string(),
                        AdapterStatus::Installed => "Installed; adapter not connected".to_string(),
                        AdapterStatus::NotInstalled => "CLI not found".to_string(),
                        AdapterStatus::PolicyGated => "Adapter not connected".to_string(),
                    };
                    ArgItem {
                        display,
                        match_text: provider.key().to_string(),
                        insert_text: provider.key().to_string(),
                        description,
                    }
                })
                .collect(),
        )
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            return CommandResult::Message(format!(
                "Active provider: {}. Use /provider <name> to choose an agent.",
                active_provider().label()
            ));
        }

        let Some(provider_id) = ProviderId::parse(trimmed) else {
            return CommandResult::Error(format!(
                "Unknown provider: {trimmed}. Available: Grok, Codex, Gemini, Claude"
            ));
        };
        let Some(provider) = provider(&provider_id) else {
            return CommandResult::Error(format!("Provider is not registered: {trimmed}"));
        };
        match provider.status {
            AdapterStatus::Ready if provider.id() == &active_provider() => {
                CommandResult::Message(format!("Provider: {} (active)", provider.display_name()))
            }
            AdapterStatus::Ready => {
                CommandResult::Action(crate::app::actions::Action::SwitchProvider(provider_id))
            }
            AdapterStatus::Installed => CommandResult::Error(format!(
                "{} is installed, but its adapter is not connected yet. {} remains active.",
                provider.display_name(),
                active_provider().label()
            )),
            AdapterStatus::NotInstalled => CommandResult::Error(format!(
                "{} CLI was not found. {} remains active.",
                provider.display_name(),
                active_provider().label()
            )),
            AdapterStatus::PolicyGated => CommandResult::Error(format!(
                "{} is not connected yet. {} remains active.",
                provider.display_name(),
                active_provider().label()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model_state::ModelState;
    use crate::slash::commands::tests::make_ctx;

    #[test]
    fn provider_is_registered_with_agent_alias() {
        let registry = crate::slash::registry::CommandRegistry::new(
            crate::slash::commands::builtin_commands(),
        );
        assert_eq!(
            registry.get("provider").map(|item| item.name()),
            Some("provider")
        );
        assert_eq!(
            registry.get("agent").map(|item| item.name()),
            Some("provider")
        );
    }

    #[test]
    fn provider_suggestions_report_live_adapter_state() {
        let models = ModelState::default();
        let context = crate::slash::command::AppCtx {
            models: &models,
            cwd: std::path::Path::new("."),
            workflows_available: false,
            saved_workflows: &[],
            workflow_runs: &[],
            screen_mode: crate::app::ScreenMode::Fullscreen,
            current_title: None,
        };
        let items = ProviderCommand
            .suggest_args(&context, "")
            .unwrap_or_default();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].display, "Grok (active)");
        assert_eq!(items[0].description, "Ready");
        assert_eq!(items[1].display, "Codex");
    }

    #[test]
    fn selecting_grok_keeps_the_ready_provider_active() {
        let models = ModelState::default();
        let mut context = make_ctx(&models);
        let CommandResult::Message(message) = ProviderCommand.run(&mut context, "xai") else {
            panic!("expected provider status");
        };
        assert_eq!(message, "Provider: Grok (active)");
    }

    #[test]
    fn selecting_ready_provider_requests_a_process_handoff() {
        let models = ModelState::default();
        let mut context = make_ctx(&models);
        let CommandResult::Action(crate::app::actions::Action::SwitchProvider(provider)) =
            ProviderCommand.run(&mut context, "codex")
        else {
            panic!("expected provider switch action");
        };
        assert_eq!(provider, ProviderId::Codex);
    }
}
