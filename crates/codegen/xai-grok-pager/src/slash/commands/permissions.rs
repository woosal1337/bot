// Modified by the Bot project on 2026-09-13: Removed announcement fixture state.
use crate::app::actions::{Action, PermissionModeKind};
use crate::slash::command::{
    AppCtx, ArgItem, CommandExecCtx, CommandResult, SlashCommand, slash_meta,
};

pub struct PermissionsCommand;

impl SlashCommand for PermissionsCommand {
    slash_meta! {
        name: "permissions",
        aliases: ["permission", "approval"],
        description: "Choose when the agent asks for tool permission",
        usage: "/permissions <mode>",
        takes_args: true,
        args_required: true,
        session_scoped: true,
        arg_placeholder: "<mode>",
    }

    fn suggest_args(&self, _ctx: &AppCtx, _args_query: &str) -> Option<Vec<ArgItem>> {
        Some(
            crate::provider::active_permission_terms()
                .iter()
                .map(|term| mode(term.display, term.keyword, term.description))
                .collect(),
        )
    }

    fn run(&self, ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        if args.trim().is_empty() {
            let current = if ctx.pager_state.yolo_mode {
                "always-approve"
            } else if ctx.pager_state.auto_mode {
                "auto"
            } else {
                "default or ask"
            };
            return CommandResult::Error(format!(
                "Choose a permission mode. Current mode: {current}. Usage: {}",
                self.usage()
            ));
        }
        let Some(term) = crate::provider::parse_active_permission_term(args) else {
            let choices = crate::provider::active_permission_terms()
                .iter()
                .map(|term| term.keyword)
                .collect::<Vec<_>>()
                .join(", ");
            return CommandResult::Error(format!(
                "Unknown permission mode: {}. Choose one of: {choices}.",
                args.trim()
            ));
        };
        let Some(kind) = PermissionModeKind::from_canonical(term.kind) else {
            return CommandResult::Error(
                "The provider returned an unknown permission mode.".into(),
            );
        };
        CommandResult::Action(Action::SetPermissionMode(kind))
    }
}

fn mode(display: &str, canonical: &str, description: &str) -> ArgItem {
    ArgItem {
        display: display.to_owned(),
        match_text: format!("{display} {canonical}"),
        insert_text: canonical.to_owned(),
        description: description.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model_state::ModelState;
    use crate::slash::commands::tests::make_ctx;

    #[test]
    fn permissions_is_registered_with_aliases() {
        let registry = crate::slash::registry::CommandRegistry::new(
            crate::slash::commands::builtin_commands(),
        );
        for name in ["permissions", "permission", "approval"] {
            assert_eq!(
                registry.get(name).map(|command| command.name()),
                Some("permissions")
            );
        }
    }

    #[test]
    fn permissions_is_visible_in_the_initial_palette() {
        let models = ModelState::default();
        let mut controller = crate::slash::SlashController::with_builtins(".".into());
        let state = crate::slash::SlashState::default();
        controller.refresh(&state, "/", 1, &models);
        assert!(
            state
                .snapshot()
                .matches
                .iter()
                .any(|row| row.display == "/permissions")
        );
    }

    #[test]
    fn suggestions_cover_each_permission_mode() {
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
        let items = PermissionsCommand
            .suggest_args(&context, "")
            .unwrap_or_default();
        assert_eq!(
            items
                .iter()
                .map(|item| item.insert_text.as_str())
                .collect::<Vec<_>>(),
            ["ask", "auto", "always-approve"]
        );
    }

    #[test]
    fn each_permission_mode_dispatches_the_typed_action() {
        let models = ModelState::default();
        let mut context = make_ctx(&models);
        for (value, expected) in [
            ("default", PermissionModeKind::Ask),
            ("ask", PermissionModeKind::Ask),
            ("auto", PermissionModeKind::Auto),
            ("always approve", PermissionModeKind::AlwaysApprove),
        ] {
            assert!(matches!(
                PermissionsCommand.run(&mut context, value),
                CommandResult::Action(Action::SetPermissionMode(kind)) if kind == expected
            ));
        }
    }
}
