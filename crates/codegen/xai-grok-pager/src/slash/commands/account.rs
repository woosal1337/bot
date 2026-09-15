use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct AccountCommand;

impl SlashCommand for AccountCommand {
    slash_meta! {
        name: "account",
        description: "Show account status and rate limits",
        usage: "/account",
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        if args.trim().is_empty() {
            CommandResult::Action(Action::ShowAccount)
        } else {
            CommandResult::Error("Usage: /account".to_owned())
        }
    }
}
