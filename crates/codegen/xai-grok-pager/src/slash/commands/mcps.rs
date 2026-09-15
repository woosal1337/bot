// Modified by the Bot project on 2026-09-13: hide MCP controls when a provider has no MCP catalog.
use crate::app::actions::Action;
use crate::slash::command::{AppCtx, CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct McpsCommand;

impl SlashCommand for McpsCommand {
    slash_meta! {
        name: "mcps",
        description: "Show MCP server status",
        usage: "/mcps",
    }

    fn visible(&self, _ctx: &AppCtx) -> bool {
        crate::provider::active_extension_capabilities().mcp_servers
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::OpenExtensionsModal {
            tab: crate::views::extensions_modal::ExtensionsTab::McpServers,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        })
    }
}
