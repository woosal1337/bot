use crate::app::actions::Action;
use crate::slash::command::{
    AppCtx, ArgItem, CommandExecCtx, CommandResult, SlashCommand, slash_meta,
};

pub struct LoginCommand;

impl SlashCommand for LoginCommand {
    slash_meta! {
        name: "login",
        description: "Log in or re-authenticate with your account",
        usage: "/login [browser|device]",
        takes_args: true,
        arg_placeholder: "browser/device",
    }

    fn suggest_args(&self, _ctx: &AppCtx, _args_query: &str) -> Option<Vec<ArgItem>> {
        Some(vec![
            ArgItem {
                display: "browser".to_owned(),
                match_text: "browser chatgpt web".to_owned(),
                insert_text: "browser".to_owned(),
                description: "Sign in locally or through a forwarded callback".to_owned(),
            },
            ArgItem {
                display: "device".to_owned(),
                match_text: "device code headless remote".to_owned(),
                insert_text: "device".to_owned(),
                description: "Sign in with a device code".to_owned(),
            },
        ])
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        match args.trim().to_ascii_lowercase().as_str() {
            "" => CommandResult::Action(Action::Login),
            "browser" | "web" => CommandResult::Action(Action::LoginBrowser),
            "device" | "device-code" => CommandResult::Action(Action::LoginDevice),
            _ => CommandResult::Error("Usage: /login [browser|device]".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model_state::ModelState;
    use crate::app::bundle::BundleState;

    static EMPTY_BUNDLE: BundleState = BundleState {
        has_cache: false,
        version: String::new(),
        personas: Vec::new(),
        roles: Vec::new(),
        agents: Vec::new(),
        skills: Vec::new(),
        persona_details: Vec::new(),
        role_details: Vec::new(),
    };

    fn run(args: &str) -> CommandResult {
        let models = ModelState::default();
        let mut context = CommandExecCtx {
            models: &models,
            session_id: None,
            bundle_state: &EMPTY_BUNDLE,
            screen_mode: crate::app::ScreenMode::Fullscreen,
            pager_state: crate::settings::PagerLocalSnapshot::default(),
        };
        LoginCommand.run(&mut context, args)
    }

    #[test]
    fn selects_browser_and_device_login() {
        assert!(matches!(run(""), CommandResult::Action(Action::Login)));
        assert!(matches!(
            run("browser"),
            CommandResult::Action(Action::LoginBrowser)
        ));
        assert!(matches!(
            run("device"),
            CommandResult::Action(Action::LoginDevice)
        ));
        assert!(matches!(run("unknown"), CommandResult::Error(_)));
    }
}
