// Modified by the Bot project on 2026-09-13: Removed provider-specific usage gates.
//! Wrapper that turns an ACP `AvailableCommand` into a `SlashCommand`.
//!
//! ACP-advertised commands appear in the dropdown but pass through to the shell for execution.
//! The wrapper stores `String` fields, consistent with the `&str` trait design.
//!
//! Skills (`SkillMeta::Skill`) are also passed through as `/name args` for the shell to expand, but marked `InjectSkill` for rendering.

use agent_client_protocol as acp;
use bot_provider::{COMMAND_OWNERSHIP_VERSION, CommandKind, CommandOwnership};
use xai_grok_tools::implementations::skills::types::SkillScope;

use super::command::{CommandExecCtx, CommandProvenance, CommandResult, SlashCommand};

/// Identity of a skill as advertised in ACP `_meta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIdentity {
    pub path: String,
    pub scope: SkillScope,
    /// Plugin install name when present (`acme`).
    pub plugin_name: Option<String>,
    pub provider: Option<String>,
    pub provider_name: Option<String>,
}

impl SkillIdentity {
    /// Plugin install name, else the scope (plugin skills can carry any scope).
    pub fn source(&self) -> &str {
        self.plugin_name.as_deref().unwrap_or(self.scope.as_ref())
    }
}

/// Parsed ACP `_meta` skill fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillMeta {
    /// No skill keys.
    Absent,
    Skill(SkillIdentity),
    /// Unknown `scope` string (e.g. `"workflow"`). Pass through, don't error.
    Foreign,
    /// Skill-like keys present but invalid. Invocation errors rather than silently degrading.
    Malformed,
}

impl SkillMeta {
    pub fn parse(meta: Option<&serde_json::Map<String, serde_json::Value>>) -> Self {
        let Some(m) = meta else {
            return SkillMeta::Absent;
        };
        let path_val = m.get("path");
        let scope_val = m.get("scope");
        if path_val.is_none() && scope_val.is_none() {
            return SkillMeta::Absent;
        }
        let path = path_val.and_then(|v| v.as_str());
        let scope: Option<SkillScope> =
            scope_val.and_then(|v| serde_json::from_value(v.clone()).ok());
        let ownership = match m.get("botOwnership") {
            Some(value) => match serde_json::from_value::<CommandOwnership>(value.clone()) {
                Ok(ownership) if ownership.kind() == CommandKind::Skill => Some(ownership),
                Ok(_) => return SkillMeta::Foreign,
                Err(_) => return SkillMeta::Malformed,
            },
            None => None,
        };
        match (path, scope) {
            (Some(path), Some(scope)) => {
                let plugin_name = trimmed_string_field(m, "pluginName");
                let source = plugin_name.as_deref().unwrap_or(scope.as_ref());
                if ownership
                    .as_ref()
                    .is_some_and(|ownership| ownership.owner() != source)
                {
                    return SkillMeta::Malformed;
                }
                SkillMeta::Skill(SkillIdentity {
                    path: path.to_string(),
                    scope,
                    plugin_name,
                    provider: ownership
                        .as_ref()
                        .map(|ownership| ownership.provider().to_owned())
                        .or_else(|| trimmed_string_field(m, "provider")),
                    provider_name: trimmed_string_field(m, "bareName"),
                })
            }
            (_, None) if scope_val.is_some_and(|v| v.is_string()) => SkillMeta::Foreign,
            _ => SkillMeta::Malformed,
        }
    }
}

fn trimmed_string_field(
    m: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    m.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A slash command backed by an ACP `AvailableCommand`.
pub struct AcpSlashCommand {
    name: String,
    description: String,
    has_args: bool,
    arg_hint: Option<String>,
    skill: SkillMeta,
}

impl SlashCommand for AcpSlashCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn provenance(&self) -> CommandProvenance {
        match &self.skill {
            SkillMeta::Skill(identity) => CommandProvenance::Skill {
                source: identity.source().to_string(),
            },
            _ => CommandProvenance::Shell,
        }
    }

    fn usage(&self) -> &str {
        &self.name
    }

    fn takes_args(&self) -> bool {
        self.has_args
    }

    /// ACP commands always accept Enter; args are never required locally.
    /// The shell validates.
    fn args_required(&self) -> bool {
        false
    }

    fn arg_placeholder(&self) -> Option<&str> {
        self.arg_hint.as_deref()
    }

    fn is_skill(&self) -> bool {
        matches!(self.skill, SkillMeta::Skill(_))
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let text = if args.trim().is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{} {}", self.name, args)
        };
        match self.skill {
            SkillMeta::Malformed => {
                CommandResult::Error(format!("Malformed skill metadata for /{}", self.name))
            }
            SkillMeta::Absent | SkillMeta::Foreign => CommandResult::PassThrough(text),
            SkillMeta::Skill(ref identity) => {
                let mut block = acp::TextContent::new(text.clone());
                if identity.provider.as_deref() == Some("codex") {
                    let meta = serde_json::json!({
                        "botCommand": {
                            "version": COMMAND_OWNERSHIP_VERSION,
                            "provider": "codex",
                            "owner": identity.source(),
                            "kind": "skill",
                            "name": identity.provider_name.as_deref().unwrap_or(&self.name),
                            "path": identity.path,
                            "arguments": args.trim(),
                        }
                    })
                    .as_object()
                    .cloned();
                    block = block.meta(meta);
                }
                CommandResult::InjectSkill {
                    display_text: text.clone(),
                    prompt_blocks: vec![acp::ContentBlock::Text(block)],
                    display_as_skill: true,
                    scheduled_task_preview: None,
                }
            }
        }
    }
}

impl From<&acp::AvailableCommand> for AcpSlashCommand {
    fn from(cmd: &acp::AvailableCommand) -> Self {
        let arg_hint = cmd.input.as_ref().and_then(|input| match input {
            acp::AvailableCommandInput::Unstructured(u) => Some(u.hint.clone()),
            // TODO(acp-0.10): `AvailableCommandInput` is #[non_exhaustive].
            _ => None,
        });

        Self {
            name: cmd.name.clone(),
            description: cmd.description.clone(),
            // ACP commands always accept free-form input; the shell handles whatever text follows the command name
            // The `input` field only determines the placeholder hint, not whether args are allowed
            has_args: true,
            arg_hint,
            skill: SkillMeta::parse(cmd.meta.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cmd(name: &str, meta: Option<serde_json::Value>) -> acp::AvailableCommand {
        let mut cmd = acp::AvailableCommand::new(name.to_string(), format!("{name} command"));
        if let Some(m) = meta.and_then(|v| v.as_object().cloned()) {
            cmd = cmd.meta(m);
        }
        cmd
    }

    fn parse(meta: serde_json::Value) -> SkillMeta {
        SkillMeta::parse(meta.as_object())
    }

    #[test]
    fn no_meta_or_unrelated_meta_is_absent() {
        assert_eq!(SkillMeta::parse(None), SkillMeta::Absent);
        assert_eq!(
            parse(serde_json::json!({"foo": "bar", "baz": 42})),
            SkillMeta::Absent
        );
    }

    #[test]
    fn valid_skill_meta_parses_identity() {
        let meta = serde_json::json!({
            "scope": "local",
            "path": "/home/user/.grok/skills/commit/SKILL.md",
        });
        assert_eq!(
            parse(meta),
            SkillMeta::Skill(SkillIdentity {
                path: "/home/user/.grok/skills/commit/SKILL.md".to_string(),
                scope: SkillScope::Local,
                plugin_name: None,
                provider: None,
                provider_name: None,
            })
        );
    }

    #[test]
    fn plugin_meta_carries_plugin_name() {
        let meta = serde_json::json!({
            "scope": "plugin",
            "path": "/plugins/acme/skills/login/SKILL.md",
            "pluginName": "acme",
            "qualifiedName": "acme:login",
        });
        assert_eq!(
            parse(meta),
            SkillMeta::Skill(SkillIdentity {
                path: "/plugins/acme/skills/login/SKILL.md".to_string(),
                scope: SkillScope::Plugin,
                plugin_name: Some("acme".to_string()),
                provider: None,
                provider_name: None,
            })
        );
    }

    #[test]
    fn codex_meta_carries_provider_identity() {
        let meta = serde_json::json!({
            "scope": "repo",
            "path": "/work/.agents/skills/check/SKILL.md",
            "bareName": "check",
            "provider": "codex",
            "botOwnership": {
                "version": 1,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill",
            },
        });
        assert_eq!(
            parse(meta),
            SkillMeta::Skill(SkillIdentity {
                path: "/work/.agents/skills/check/SKILL.md".to_string(),
                scope: SkillScope::Repo,
                plugin_name: None,
                provider: Some("codex".to_string()),
                provider_name: Some("check".to_string()),
            })
        );
    }

    #[test]
    fn skill_ownership_must_match_the_advertised_source() {
        for bot_ownership in [
            serde_json::json!({
                "version": 1,
                "provider": "codex",
                "owner": "user",
                "kind": "skill",
            }),
            serde_json::json!({
                "version": 9,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill",
            }),
        ] {
            assert_eq!(
                parse(serde_json::json!({
                    "scope": "repo",
                    "path": "/work/.agents/skills/check/SKILL.md",
                    "botOwnership": bot_ownership,
                })),
                SkillMeta::Malformed
            );
        }
    }

    #[test]
    fn workflow_ownership_is_not_parsed_as_a_skill() {
        assert_eq!(
            parse(serde_json::json!({
                "scope": "repo",
                "path": "/work/workflows/check.rhai",
                "botOwnership": {
                    "version": 1,
                    "provider": "grok",
                    "owner": "repo",
                    "kind": "workflow",
                },
            })),
            SkillMeta::Foreign
        );
    }

    #[test]
    fn unknown_scope_string_is_foreign_not_malformed() {
        let meta = serde_json::json!({
            "scope": "workflow",
            "path": ".grok/workflows/pr-cleanup.rhai",
        });
        assert_eq!(parse(meta), SkillMeta::Foreign);
    }

    #[test]
    fn missing_or_mistyped_skill_keys_are_malformed() {
        for (label, meta) in [
            ("scope without path", serde_json::json!({"scope": "user"})),
            (
                "path without scope",
                serde_json::json!({"path": "/path/to/SKILL.md"}),
            ),
            (
                "path is not a string",
                serde_json::json!({"scope": "local", "path": 42}),
            ),
            (
                "scope is not a string",
                serde_json::json!({"scope": 42, "path": "/path/to/SKILL.md"}),
            ),
        ] {
            assert_eq!(parse(meta), SkillMeta::Malformed, "{label}");
        }
    }

    #[test]
    fn empty_plugin_name_is_dropped() {
        let meta = serde_json::json!({
            "scope": "plugin",
            "path": "/x/SKILL.md",
            "pluginName": "  ",
        });
        match parse(meta) {
            SkillMeta::Skill(identity) => assert_eq!(identity.plugin_name, None),
            other => panic!("expected Skill, got {other:?}"),
        }
    }

    fn identity(scope: SkillScope, plugin_name: Option<&str>) -> SkillIdentity {
        SkillIdentity {
            path: "/x/SKILL.md".to_string(),
            scope,
            plugin_name: plugin_name.map(str::to_string),
            provider: None,
            provider_name: None,
        }
    }

    #[test]
    fn source_prefers_plugin_name_over_scope() {
        assert_eq!(identity(SkillScope::Plugin, Some("acme")).source(), "acme");
        assert_eq!(identity(SkillScope::Repo, Some("acme")).source(), "acme");
        assert_eq!(identity(SkillScope::Plugin, None).source(), "plugin");
        assert_eq!(identity(SkillScope::Local, None).source(), "local");
    }

    #[test]
    fn provenance_distinguishes_skills_from_shell_commands() {
        let skill = AcpSlashCommand::from(&make_cmd(
            "login",
            Some(serde_json::json!({
                "scope": "plugin",
                "path": "/plugins/acme/skills/login/SKILL.md",
                "pluginName": "acme",
            })),
        ));
        assert!(skill.is_skill());
        assert_eq!(
            skill.provenance(),
            CommandProvenance::Skill {
                source: "acme".to_string()
            }
        );

        let shell_cmd = AcpSlashCommand::from(&make_cmd("flush", None));
        assert!(!shell_cmd.is_skill());
        assert_eq!(shell_cmd.provenance(), CommandProvenance::Shell);
    }

    fn make_skill_cmd(name: &str, path: &str, scope: SkillScope) -> AcpSlashCommand {
        AcpSlashCommand {
            name: name.to_string(),
            description: format!("{name} skill"),
            has_args: true,
            arg_hint: None,
            skill: SkillMeta::Skill(SkillIdentity {
                path: path.to_string(),
                scope,
                plugin_name: None,
                provider: None,
                provider_name: None,
            }),
        }
    }

    fn make_exec_ctx() -> CommandExecCtx<'static> {
        use crate::acp::model_state::ModelState;
        let models = Box::leak(Box::new(ModelState::default()));
        let bundle = Box::leak(Box::new(crate::app::bundle::BundleState::default()));
        CommandExecCtx {
            models,
            session_id: None,
            bundle_state: bundle,
            screen_mode: crate::app::ScreenMode::Inline,
            pager_state: crate::settings::PagerLocalSnapshot {
                multiline_mode: false,
                yolo_mode: false,
                ..crate::settings::PagerLocalSnapshot::default()
            },
        }
    }

    #[test]
    fn run_non_skill_passes_through() {
        let cmd = AcpSlashCommand::from(&make_cmd("flush", None));
        let mut ctx = make_exec_ctx();
        let result = cmd.run(&mut ctx, "");
        assert!(matches!(result, CommandResult::PassThrough(t) if t == "/flush"));
    }

    #[test]
    fn run_foreign_kind_passes_through_with_args() {
        let cmd = AcpSlashCommand::from(&make_cmd(
            "pr-cleanup",
            Some(serde_json::json!({
                "scope": "workflow",
                "path": ".grok/workflows/pr-cleanup.rhai",
            })),
        ));
        let mut ctx = make_exec_ctx();
        match cmd.run(&mut ctx, "fix the branch") {
            CommandResult::PassThrough(text) => assert_eq!(text, "/pr-cleanup fix the branch"),
            other => panic!("expected PassThrough, got {other:?}"),
        }
    }

    #[test]
    fn run_malformed_meta_returns_error() {
        let cmd = AcpSlashCommand::from(&make_cmd(
            "broken",
            Some(serde_json::json!({"scope": "local", "path": 42})),
        ));
        let mut ctx = make_exec_ctx();
        let result = cmd.run(&mut ctx, "");
        assert!(matches!(result, CommandResult::Error(msg) if msg.contains("Malformed")));
    }

    #[test]
    fn run_skill_sends_raw_slash_text_to_shell() {
        for (name, args, expected) in [
            ("commit", "fix the auth bug", "/commit fix the auth bug"),
            ("deploy", "", "/deploy"),
            ("local:compact", "", "/local:compact"),
        ] {
            let cmd = make_skill_cmd(name, "/nonexistent/path/SKILL.md", SkillScope::Local);
            let mut ctx = make_exec_ctx();
            match cmd.run(&mut ctx, args) {
                CommandResult::InjectSkill {
                    display_text,
                    prompt_blocks,
                    ..
                } => {
                    assert_eq!(display_text, expected, "/{name} {args}");
                    let [acp::ContentBlock::Text(block)] = &prompt_blocks[..] else {
                        panic!("/{name}: expected a single Text block, got {prompt_blocks:?}");
                    };
                    assert_eq!(block.text, expected, "/{name} {args}");
                    assert!(
                        !block.text.contains('<'),
                        "/{name}: no client-side XML markup: {}",
                        block.text
                    );
                }
                other => panic!("/{name}: expected InjectSkill, got {other:?}"),
            }
        }
    }

    #[test]
    fn run_codex_skill_carries_native_identity() {
        let cmd = AcpSlashCommand::from(&make_cmd(
            "repo:check",
            Some(serde_json::json!({
                "scope": "repo",
                "path": "/work/.agents/skills/check/SKILL.md",
                "bareName": "check",
                "provider": "codex",
            })),
        ));
        let mut ctx = make_exec_ctx();
        let CommandResult::InjectSkill {
            display_text,
            prompt_blocks,
            ..
        } = cmd.run(&mut ctx, "inspect auth")
        else {
            panic!("expected skill injection");
        };
        assert_eq!(display_text, "/repo:check inspect auth");
        let [acp::ContentBlock::Text(block)] = &prompt_blocks[..] else {
            panic!("expected one text block");
        };
        assert_eq!(block.text, "/repo:check inspect auth");
        assert_eq!(
            block.meta.as_ref().and_then(|meta| meta.get("botCommand")),
            Some(&serde_json::json!({
                "version": 1,
                "provider": "codex",
                "owner": "repo",
                "kind": "skill",
                "name": "check",
                "path": "/work/.agents/skills/check/SKILL.md",
                "arguments": "inspect auth",
            }))
        );
    }
}
