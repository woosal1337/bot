// Modified by the Bot project on 2026-09-13: hide commands that the active provider cannot run.
//! Command registry: maps command names/aliases to `SlashCommand` implementations.
//!
//! Design choices:
//!
//! - `String` keys throughout (not `&'static str`) for ACP command support.
//! - `CommandSource` tracks provenance (Builtin vs Acp) for replacement logic.
//! - `set_acp_commands()` replaces ACP-sourced entries without touching builtins.
//! - ACP names that collide with a builtin trigger or blocked name are skipped.
//! - `rebuild_triggers()` regenerates the trigger list after mutations.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::acp_command::{AcpSlashCommand, SkillMeta};
use super::command::{CommandProvenance, SlashCommand, WorkflowChoice};
use super::mode_support::ModeSupport;

/// Shell ACP names the pager never offers (unified `/hooks` / `/plugins` UI, plus `/help`).
/// These must stay covered by [`xai_grok_shell::session::PAGER_COMMAND_KEYS`].
/// A skill of the same name is then advertised already qualified instead of being dropped here when the matching shell gate is off.
pub(crate) const BLOCKED_ACP_NAMES: &[&str] = &[
    "help",
    "hooks-add",
    "hooks-list",
    "hooks-remove",
    "hooks-trust",
    "hooks-untrust",
    "reload-plugins",
];

/// Source of a command in the registry. Used for precedence and replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    /// Pager-local builtin (e.g., /exit, /model).
    Builtin,
    /// Advertised by the shell/agent via ACP AvailableCommandsUpdate.
    Acp,
}

/// A trigger entry in the registry, one per canonical name or alias.
/// Triggers are what the fuzzy matcher operates on. Each command produces
/// at least one trigger (canonical name), plus one per alias.
#[derive(Debug, Clone)]
pub struct CommandTrigger {
    /// The canonical command name (e.g., "exit").
    pub canonical: String,
    /// If this trigger is an alias, the alias text. None for canonical triggers.
    pub alias: Option<String>,
    /// Display text for the dropdown (e.g., "/exit").
    pub display: String,
    pub match_text: String,
    /// Command description.
    pub description: String,
    /// Usage string.
    pub usage: String,
    /// Whether this command takes arguments.
    pub takes_args: bool,
    /// Whether arguments are required (only meaningful when `takes_args` is true).
    pub args_required: bool,
    /// Index into `CommandRegistry::commands`.
    pub command_index: usize,
    /// Source of this command.
    pub source: CommandSource,
    pub provenance: CommandProvenance,
}

impl CommandTrigger {
    fn new(
        command: &Arc<dyn SlashCommand>,
        alias: Option<&str>,
        canonical: &str,
        command_index: usize,
        source: CommandSource,
    ) -> Self {
        let key = alias.unwrap_or(canonical);
        Self {
            canonical: canonical.to_string(),
            alias: alias.map(|s| s.to_string()),
            display: format!("/{key}"),
            match_text: key.to_string(),
            description: command.description().to_string(),
            usage: command.usage().to_string(),
            takes_args: command.takes_args(),
            args_required: command.args_required(),
            command_index,
            source,
            provenance: command.provenance(),
        }
    }

    /// Sibling that fuzzy-matches the bare suffix of a qualified skill, so typing `/login` offers `/acme:login` beside the builtin.
    /// A real trigger (not a concatenated haystack) keeps scores and highlight indices honest.
    fn bare_suffix_sibling(&self) -> Option<Self> {
        if !matches!(self.provenance, CommandProvenance::Skill { .. }) {
            return None;
        }
        let (_, bare) = self.match_text.rsplit_once(':')?;
        if bare.is_empty() {
            return None;
        }
        let mut sibling = self.clone();
        sibling.match_text = bare.to_string();
        Some(sibling)
    }
}

/// Owns the command objects and provides lookup by name/alias.
/// Supports dynamic mutation via `set_acp_commands()` for runtime ACP command catalog updates.
pub struct CommandRegistry {
    commands: Vec<Arc<dyn SlashCommand>>,
    sources: Vec<CommandSource>,
    key_to_index: HashMap<String, usize>,
    triggers: Vec<CommandTrigger>,
    /// Commands hidden by name (not shown in dropdown, not executable).
    hidden: HashSet<String>,
    /// Commands hidden from the completion menu ONLY (no dropdown row, no ghost / palette trigger).
    /// They stay resolvable for dispatch via [`Self::get_for_dispatch`]: a fully-typed invocation still executes.
    /// Registry-level analogue of the per-command `SlashCommand::visible()` gate (the `/gboom` mechanism) for state the command object cannot see.
    menu_hidden: HashSet<String>,
    /// Names of tools the connected agent has advertised. Fail-closed. Otherwise the user could submit `/loop` from the
    /// home screen and start a session whose model can't actually run it.
    available_tools: Option<HashSet<String>>,
    /// Launchable workflow definitions from the last ACP catalog sync.
    /// Extracted from `_meta.workflowSource` on the incoming list (including names skipped as reserved/claimed) so `/workflow` can suggest them.
    saved_workflows: Vec<WorkflowChoice>,
    /// Names the last sync dropped: a sorted `reserved` run, then a sorted `duplicate` run.
    skipped_acp_names: Vec<String>,
}

impl CommandRegistry {
    /// Build a registry from builtin commands.
    /// Panics if two builtin commands share the same canonical name or alias.
    pub fn new(builtins: Vec<Arc<dyn SlashCommand>>) -> Self {
        let n = builtins.len();
        let sources = vec![CommandSource::Builtin; n];
        // Fail-closed until the matching `set_*_visible` call reveals them.
        let mut hidden = HashSet::new();
        hidden.insert("dashboard".to_string());
        hidden.insert("recap".to_string());
        // `/auto` is fail-closed: hidden until `set_auto_mode_available(true)`.
        hidden.insert("auto".to_string());
        let mut reg = Self {
            commands: builtins,
            sources,
            key_to_index: HashMap::new(),
            triggers: Vec::new(),
            hidden,
            menu_hidden: HashSet::new(),
            available_tools: None,
            saved_workflows: Vec::new(),
            skipped_acp_names: Vec::new(),
        };
        reg.rebuild_triggers();
        reg
    }

    fn set_command_visible(&mut self, name: &str, visible: bool) {
        if visible {
            self.hidden.remove(name);
        } else {
            self.hidden.insert(name.to_string());
        }
        self.rebuild_triggers();
    }

    /// Look up a command by canonical name or alias, applying every visibility gate.
    /// Returns `None` for hidden and menu-hidden commands, and for those whose `required_tools()` are not all in the advertised toolset.
    /// Dispatch call sites that execute a fully-typed submission must use [`Self::get_for_dispatch`] instead, which ignores the menu-only gate.
    pub fn get(&self, key: &str) -> Option<&Arc<dyn SlashCommand>> {
        self.get_for_dispatch(key)
            .filter(|cmd| !self.menu_hidden.contains(cmd.name()))
    }

    /// Look up a command by canonical name or alias for EXECUTION of a typed invocation, ignoring the menu-only gate
    /// (`menu_hidden`). `menu_hidden` means "don't OFFER this in completion", not "this command doesn't exist". A
    /// fully-typed submission must still reach the pager's own handler rather than fall through as an unknown command.
    pub fn get_for_dispatch(&self, key: &str) -> Option<&Arc<dyn SlashCommand>> {
        self.key_to_index
            .get(key)
            .and_then(|idx| self.commands.get(*idx))
            .filter(|cmd| !self.hidden.contains(cmd.name()))
            .filter(|cmd| self.tools_satisfied(cmd))
    }

    /// Declared modes for `key` (canonical name or alias), unfiltered by any runtime gate.
    pub(crate) fn mode_support(&self, key: &str) -> ModeSupport {
        self.commands
            .iter()
            .find(|cmd| cmd.name() == key || cmd.aliases().contains(&key))
            .map_or(ModeSupport::Both, |cmd| cmd.mode_support())
    }

    /// True when `cmd.required_tools()` is empty, or the toolset is known and every required tool is in the advertised set.
    ///
    /// When `available_tools == None` (pre-session bootstrap), commands with non-empty `required_tools()` are hidden; see the field doc.
    fn tools_satisfied(&self, cmd: &Arc<dyn SlashCommand>) -> bool {
        let required = cmd.required_tools();
        if required.is_empty() {
            return true;
        }
        match &self.available_tools {
            None => false,
            Some(set) => required.iter().all(|t| set.contains(*t)),
        }
    }

    /// Returns true if the command (by canonical name or alias) is a builtin.
    pub fn is_builtin(&self, key: &str) -> bool {
        self.key_to_index
            .get(key)
            .and_then(|idx| self.sources.get(*idx))
            .is_some_and(|s| *s == CommandSource::Builtin)
    }

    /// All triggers (for fuzzy matching).
    pub fn triggers(&self) -> &[CommandTrigger] {
        &self.triggers
    }

    /// Look up a command by its `triggers()` index.
    /// Used by the slash controller to resolve a `CommandTrigger.command_index` back to the underlying `SlashCommand` for visibility filtering.
    pub fn commands_by_index(&self, index: usize) -> Option<&Arc<dyn SlashCommand>> {
        self.commands.get(index)
    }

    /// Number of unique commands (not triggers).
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// Show or hide the /hooks and /plugins commands.
    /// When hidden, they won't appear in the dropdown or be executable.
    pub fn set_plugins_visible(&mut self, visible: bool) {
        let capabilities = crate::provider::active_extension_capabilities();
        for (name, available) in [
            ("hooks", capabilities.hooks),
            ("plugins", capabilities.plugins),
        ] {
            if visible && available {
                self.hidden.remove(name);
            } else {
                self.hidden.insert(name.to_string());
            }
        }
        self.rebuild_triggers();
    }

    /// Commands whose `required_tools()` aren't all in `tools` are hidden from the dropdown and `get()`. API note: once
    /// `Some` has been set this method only replaces the set. It cannot transition the registry back to the `None`
    /// "tool list unknown, show everything" bootstrap state. In practice the drain pipeline never delivers a clear.
    pub fn set_available_tools(&mut self, tools: HashSet<String>) {
        self.apply_available_tools(tools);
        self.rebuild_triggers();
    }

    fn apply_available_tools(&mut self, tools: HashSet<String>) {
        self.available_tools = Some(tools);
    }

    /// Show or hide the `/dashboard` command (feature-flag gating).
    /// The command is hidden by default (see [`Self::new`]) and revealed here when the dashboard feature flag (`dashboard_enabled()`) is on.
    /// When hidden it won't appear in the dropdown or be executable.
    pub fn set_dashboard_visible(&mut self, visible: bool) {
        self.set_command_visible("dashboard", visible);
    }

    /// Show or hide the `/recap` command (shell `sessionRecap` gate).
    /// Hidden by default in [`Self::new`]; revealed from initialize meta.
    pub fn set_recap_visible(&mut self, visible: bool) {
        self.set_command_visible("recap", visible);
    }

    /// Gate `/auto` on the auto permission-mode feature.
    /// When `available` is false, `/auto` is hard-hidden (fail-closed: neither offered nor executable).
    /// `/always-approve` is always offered; both commands are true toggles and stay on the menu while already active.
    pub fn set_auto_mode_available(&mut self, available: bool) {
        if available {
            self.hidden.remove("auto");
        } else {
            self.hidden.insert("auto".to_string());
        }
        self.rebuild_triggers();
    }

    /// Test-only: put `name` in (or out of) the menu-only hide set so unit tests can cover [`Self::get`] vs [`Self::get_for_dispatch`].
    #[cfg(test)]
    pub(crate) fn set_menu_hidden_for_test(&mut self, name: &str, hidden: bool) {
        if hidden {
            self.menu_hidden.insert(name.to_string());
        } else {
            self.menu_hidden.remove(name);
        }
        self.rebuild_triggers();
    }

    /// Apply both ACP-sourced commands and the agent's tool list in one shot, then rebuild triggers exactly once. keep
    /// the previous `available_tools` value. This is the preferred entry point from the per-tick ACP sync.
    pub fn set_acp_state(
        &mut self,
        commands: &[agent_client_protocol::AvailableCommand],
        tools: Option<HashSet<String>>,
    ) {
        self.apply_acp_commands(commands);
        if let Some(tools) = tools {
            self.apply_available_tools(tools);
        }
        self.rebuild_triggers();
    }

    /// Replace all ACP-sourced commands with a new set. Builtin commands are preserved. ACP names that collide with a
    /// builtin trigger or blocked name are skipped. The shell advertises colliding skills already qualified
    /// (`acme:login`). Triggers a full `rebuild_triggers()`.
    pub fn set_acp_commands(&mut self, commands: &[agent_client_protocol::AvailableCommand]) {
        self.apply_acp_commands(commands);
        self.rebuild_triggers();
    }

    /// Saved / built-in workflow definitions from the last ACP catalog.
    pub fn saved_workflows(&self) -> &[WorkflowChoice] {
        &self.saved_workflows
    }

    fn apply_acp_commands(&mut self, commands: &[agent_client_protocol::AvailableCommand]) {
        // The catalog of launchable workflows is independent of which ACP names survive reserved/claimed filtering; `/workflow` still needs them
        let mut saved_workflows: Vec<WorkflowChoice> = commands
            .iter()
            .filter_map(WorkflowChoice::from_acp)
            .collect();
        saved_workflows.sort_by(|a, b| a.name.cmp(&b.name));
        self.saved_workflows = saved_workflows;

        // Remove old ACP-sourced commands.
        let mut i = 0;
        while i < self.commands.len() {
            if self.sources[i] == CommandSource::Acp {
                self.commands.remove(i);
                self.sources.remove(i);
            } else {
                i += 1;
            }
        }

        let builtin_keys: HashSet<String> = self
            .commands
            .iter()
            .flat_map(|c| {
                std::iter::once(c.name().to_lowercase())
                    .chain(c.aliases().iter().map(|a| a.to_lowercase()))
            })
            .collect();

        let is_reserved = |name: &str| {
            builtin_keys.contains(name)
                || BLOCKED_ACP_NAMES
                    .iter()
                    .any(|b| b.eq_ignore_ascii_case(name))
        };

        // Shadowed builtins are by design; a shadowed skill or workflow breaks `PAGER_COMMAND_KEYS`
        let is_skill_or_workflow = |cmd: &agent_client_protocol::AvailableCommand| {
            matches!(SkillMeta::parse(cmd.meta.as_ref()), SkillMeta::Skill(_))
                || WorkflowChoice::from_acp(cmd).is_some()
        };
        let mut claimed: HashSet<String> = HashSet::new();
        let mut reserved: Vec<String> = Vec::new();
        let mut duplicate: Vec<String> = Vec::new();
        for acp_cmd in commands {
            let name = acp_cmd.name.to_lowercase();
            if is_reserved(&name) {
                if is_skill_or_workflow(acp_cmd) {
                    reserved.push(name);
                }
                continue;
            }
            if !claimed.insert(name) {
                if is_skill_or_workflow(acp_cmd) {
                    duplicate.push(acp_cmd.name.to_lowercase());
                }
                continue;
            }
            self.commands.push(Arc::new(AcpSlashCommand::from(acp_cmd)));
            self.sources.push(CommandSource::Acp);
        }
        // Every ACU re-runs this sync, so warn once per distinct skipped set rather than per call
        reserved.sort_unstable();
        duplicate.sort_unstable();
        let changed = !self
            .skipped_acp_names
            .iter()
            .eq(reserved.iter().chain(&duplicate));
        if changed {
            if !reserved.is_empty() || !duplicate.is_empty() {
                crate::unified_log::warn(
                    "slash.registry.skipped",
                    None,
                    Some(serde_json::json!({
                        "reserved": reserved,
                        "duplicate": duplicate,
                    })),
                );
            }
            reserved.extend(duplicate);
            self.skipped_acp_names = reserved;
        }
    }

    /// Regenerate trigger list and key-to-index map from the current commands.
    /// Called after any mutation (construction, ACP sync).
    /// Panics if two builtin commands share an alias (programmer error).
    fn rebuild_triggers(&mut self) {
        self.key_to_index.clear();
        self.triggers.clear();

        for (idx, command) in self.commands.iter().enumerate() {
            let source = self.sources[idx];
            let canonical = command.name();

            // Skip commands gated by missing tools, using the same skip pattern as `hidden`
            if !self.tools_satisfied(command) {
                continue;
            }

            // Skip hidden commands; they don't get triggers or key_to_index entries
            if self.hidden.contains(canonical) {
                continue;
            }

            // Menu-hidden commands keep their key entries so `get_for_dispatch()` resolves a typed invocation, but emit no triggers.
            let menu_only = self.menu_hidden.contains(canonical);

            // Insert canonical key.
            self.key_to_index.insert(canonical.to_string(), idx);
            if !menu_only {
                let trigger = CommandTrigger::new(command, None, canonical, idx, source);
                self.triggers.extend(trigger.bare_suffix_sibling());
                self.triggers.push(trigger);
            }

            // Insert alias keys.
            for alias in command.aliases() {
                if source == CommandSource::Builtin && self.key_to_index.contains_key(*alias) {
                    panic!(
                        "slash command alias '{}' is already registered (builtin collision)",
                        alias
                    );
                }
                self.key_to_index.insert(alias.to_string(), idx);
                if !menu_only {
                    let trigger = CommandTrigger::new(command, Some(alias), canonical, idx, source);
                    self.triggers.extend(trigger.bare_suffix_sibling());
                    self.triggers.push(trigger);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash::command::{CommandExecCtx, CommandResult};

    struct DummyCommand {
        name: &'static str,
        aliases: &'static [&'static str],
    }

    impl SlashCommand for DummyCommand {
        fn name(&self) -> &str {
            self.name
        }
        fn aliases(&self) -> &[&str] {
            self.aliases
        }
        fn description(&self) -> &str {
            "dummy"
        }
        fn usage(&self) -> &str {
            self.name
        }
        fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
            CommandResult::Handled
        }
    }

    struct ToolGatedCommand {
        name: &'static str,
        required: &'static [&'static str],
    }

    impl SlashCommand for ToolGatedCommand {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "tool-gated"
        }
        fn usage(&self) -> &str {
            self.name
        }
        fn required_tools(&self) -> &[&str] {
            self.required
        }
        fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
            CommandResult::Handled
        }
    }

    fn tool_set<I, S>(names: I) -> HashSet<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        names.into_iter().map(Into::into).collect()
    }

    #[test]
    fn lookup_by_canonical_name() {
        let cmd: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "test",
            aliases: &[],
        });
        let registry = CommandRegistry::new(vec![cmd]);
        assert!(registry.get("test").is_some());
        assert!(registry.get("unknown").is_none());
    }

    #[test]
    fn lookup_by_alias() {
        let cmd: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &["quit"],
        });
        let registry = CommandRegistry::new(vec![cmd]);
        assert!(registry.get("exit").is_some());
        assert!(registry.get("quit").is_some());
        // Both resolve to the same command.
        assert!(std::ptr::eq(
            registry.get("exit").unwrap().as_ref() as *const dyn SlashCommand,
            registry.get("quit").unwrap().as_ref() as *const dyn SlashCommand,
        ));
    }

    #[test]
    #[should_panic(expected = "alias")]
    fn registry_panics_on_builtin_alias_collision() {
        let cmd_a: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "alpha",
            aliases: &["dup"],
        });
        let cmd_b: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "beta",
            aliases: &["dup"],
        });
        let _ = CommandRegistry::new(vec![cmd_a, cmd_b]);
    }

    #[test]
    fn trigger_count_includes_aliases() {
        let cmd: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &["quit", "q"],
        });
        let registry = CommandRegistry::new(vec![cmd]);
        // 1 canonical + 2 aliases = 3 triggers.
        assert_eq!(registry.triggers().len(), 3);
        assert_eq!(registry.command_count(), 1);
    }

    #[test]
    fn set_acp_commands_replaces_only_acp() {
        let builtin: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &["quit"],
        });
        let mut registry = CommandRegistry::new(vec![builtin]);
        assert_eq!(registry.command_count(), 1);

        // Add ACP commands.
        let acp_cmds = vec![agent_client_protocol::AvailableCommand::new(
            "flush".to_string(),
            "Flush memory".to_string(),
        )];
        registry.set_acp_commands(&acp_cmds);
        assert_eq!(registry.command_count(), 2);
        assert!(registry.get("flush").is_some());

        // Replace ACP commands: flush is gone, the builtin stays
        registry.set_acp_commands(&[]);
        assert_eq!(registry.command_count(), 1);
        assert!(registry.get("exit").is_some());
        assert!(registry.get("flush").is_none());
    }

    fn acp_workflow(
        name: &str,
        description: &str,
        source: &str,
    ) -> agent_client_protocol::AvailableCommand {
        agent_client_protocol::AvailableCommand::new(name.to_string(), description.to_string())
            .meta(
                serde_json::json!({
                    "botOwnership": {
                        "version": 1,
                        "provider": "grok",
                        "owner": source,
                        "kind": "workflow",
                    },
                    "workflowSource": source,
                })
                .as_object()
                .cloned()
                .expect("object"),
            )
    }

    #[test]
    fn set_acp_commands_extracts_saved_workflows_sorted() {
        let mut registry = CommandRegistry::new(vec![Arc::new(DummyCommand {
            name: "exit",
            aliases: &[],
        })]);
        registry.set_acp_commands(&[
            agent_client_protocol::AvailableCommand::new(
                "flush".to_string(),
                "Flush memory".to_string(),
            ),
            acp_workflow("zeta-wf", "Workflow: Zeta", "user"),
            acp_workflow("alpha-wf", "Workflow: Alpha", "project"),
        ]);
        let names: Vec<&str> = registry
            .saved_workflows()
            .iter()
            .map(|w| w.name.as_str())
            .collect();
        assert_eq!(names, ["alpha-wf", "zeta-wf"]);
        assert_eq!(registry.saved_workflows()[0].description, "Alpha");
        assert_eq!(registry.saved_workflows()[1].description, "Zeta");

        registry.set_acp_commands(&[]);
        assert!(registry.saved_workflows().is_empty());
    }

    #[test]
    fn set_acp_commands_rejects_unowned_or_mismatched_workflows() {
        let mut registry = CommandRegistry::new(vec![]);
        let legacy = agent_client_protocol::AvailableCommand::new("legacy", "Workflow: Legacy")
            .meta(
                serde_json::json!({"workflowSource": "user"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            );
        let mismatched =
            agent_client_protocol::AvailableCommand::new("mismatched", "Workflow: Mismatched")
                .meta(
                    serde_json::json!({
                        "botOwnership": {
                            "version": 1,
                            "provider": "grok",
                            "owner": "project",
                            "kind": "workflow",
                        },
                        "workflowSource": "user",
                    })
                    .as_object()
                    .cloned()
                    .expect("object"),
                );
        let future = agent_client_protocol::AvailableCommand::new("future", "Workflow: Future")
            .meta(
                serde_json::json!({
                    "botOwnership": {
                        "version": 2,
                        "provider": "grok",
                        "owner": "user",
                        "kind": "workflow",
                    },
                    "workflowSource": "user",
                })
                .as_object()
                .cloned()
                .expect("object"),
            );

        registry.set_acp_commands(&[legacy, mismatched, future]);

        assert!(registry.saved_workflows().is_empty());
    }

    #[test]
    fn saved_workflows_include_names_skipped_as_reserved() {
        let mut registry = CommandRegistry::new(vec![Arc::new(DummyCommand {
            name: "theme",
            aliases: &[],
        })]);
        registry.set_acp_commands(&[acp_workflow("theme", "Workflow: colliding name", "user")]);
        assert!(
            registry.get("theme").is_some(),
            "pager builtin keeps the name"
        );
        assert_eq!(registry.saved_workflows().len(), 1);
        assert_eq!(registry.saved_workflows()[0].name, "theme");
        assert_eq!(registry.saved_workflows()[0].description, "colliding name");
    }

    #[test]
    fn dashboard_command_hidden_by_default_and_toggleable() {
        let dashboard: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "dashboard",
            aliases: &[],
        });
        let other: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &[],
        });
        let mut registry = CommandRegistry::new(vec![dashboard, other]);

        // Fail-closed: hidden by default (until the feature flag reveals it).
        assert!(registry.get("dashboard").is_none());
        assert!(
            !registry
                .triggers()
                .iter()
                .any(|t| t.canonical == "dashboard")
        );
        // Unrelated commands are unaffected.
        assert!(registry.get("exit").is_some());

        // Enabling the feature reveals it.
        registry.set_dashboard_visible(true);
        assert!(registry.get("dashboard").is_some());
        assert!(
            registry
                .triggers()
                .iter()
                .any(|t| t.canonical == "dashboard")
        );

        // Hiding again removes it.
        registry.set_dashboard_visible(false);
        assert!(registry.get("dashboard").is_none());
    }

    // ── Builtin/skill name collisions ───────────────────────────────
    //
    fn login_builtin() -> Arc<dyn SlashCommand> {
        Arc::new(DummyCommand {
            name: "login",
            aliases: &[],
        })
    }

    fn acp_skill(name: &str, meta: serde_json::Value) -> agent_client_protocol::AvailableCommand {
        agent_client_protocol::AvailableCommand::new(name.to_string(), format!("{name} skill"))
            .meta(meta.as_object().cloned().unwrap())
    }

    #[test]
    fn advertised_qualified_skill_sits_beside_builtin() {
        let mut registry = CommandRegistry::new(vec![login_builtin()]);
        registry.set_acp_commands(&[acp_skill(
            "acme:login",
            serde_json::json!({
                "scope": "plugin",
                "path": "/x/SKILL.md",
                "pluginName": "acme",
            }),
        )]);

        assert!(registry.is_builtin("login"));
        assert_eq!(
            registry.get("login").unwrap().provenance(),
            CommandProvenance::Builtin
        );
        let skill = registry.get("acme:login").expect("qualified skill");
        assert!(!registry.is_builtin("acme:login"));
        assert_eq!(
            skill.provenance(),
            CommandProvenance::Skill {
                source: "acme".to_string()
            }
        );
        assert_eq!(registry.command_count(), 2);

        let skill_match_texts: HashSet<&str> = registry
            .triggers()
            .iter()
            .filter(|t| t.canonical == "acme:login")
            .inspect(|t| assert_eq!(t.display, "/acme:login"))
            .map(|t| t.match_text.as_str())
            .collect();
        assert_eq!(skill_match_texts, HashSet::from(["login", "acme:login"]));
    }

    #[test]
    fn colliding_mixed_case_acp_name_is_skipped() {
        let mut registry = CommandRegistry::new(vec![login_builtin()]);
        registry.set_acp_commands(&[acp_skill(
            "Login",
            serde_json::json!({
                "scope": "local",
                "path": "/x/SKILL.md",
            }),
        )]);
        assert_eq!(registry.command_count(), 1);
        assert!(registry.is_builtin("login"));
        assert!(registry.get("Login").is_none());
        assert!(registry.get("local:login").is_none());
    }

    #[test]
    fn colliding_bare_acp_name_is_skipped() {
        let mut registry = CommandRegistry::new(vec![login_builtin()]);
        let commands = [acp_skill(
            "login",
            serde_json::json!({
                "scope": "plugin",
                "path": "/x/SKILL.md",
                "pluginName": "acme",
            }),
        )];
        registry.set_acp_commands(&commands);
        assert_eq!(registry.command_count(), 1);
        assert!(registry.is_builtin("login"));
        assert!(registry.get("acme:login").is_none());
        assert_eq!(registry.skipped_acp_names, ["login"]);

        registry.set_acp_commands(&commands);
        assert_eq!(registry.skipped_acp_names, ["login"]);
    }

    #[test]
    fn first_claimant_wins_duplicate_acp_name() {
        let mut registry = CommandRegistry::new(vec![login_builtin()]);
        let first = agent_client_protocol::AvailableCommand::new(
            "acme:login".to_string(),
            "first".to_string(),
        )
        .meta(
            serde_json::json!({"scope": "plugin", "path": "/a/SKILL.md", "pluginName": "acme"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        let second = agent_client_protocol::AvailableCommand::new(
            "acme:login".to_string(),
            "second".to_string(),
        )
        .meta(
            serde_json::json!({"scope": "plugin", "path": "/b/SKILL.md", "pluginName": "other"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        registry.set_acp_commands(&[first, second]);
        assert_eq!(registry.command_count(), 2, "builtin + first acme:login");
        assert_eq!(registry.get("acme:login").unwrap().description(), "first");
        assert_eq!(registry.skipped_acp_names, ["acme:login"]);
    }

    #[test]
    fn colliding_non_skill_or_malformed_command_is_dropped() {
        let non_skill = agent_client_protocol::AvailableCommand::new(
            "login".to_string(),
            "shell login".to_string(),
        );
        let malformed = acp_skill("login", serde_json::json!({"scope": "local"}));
        for cmd in [non_skill, malformed] {
            let mut registry = CommandRegistry::new(vec![login_builtin()]);
            registry.set_acp_commands(&[cmd]);
            assert_eq!(registry.command_count(), 1, "only the builtin remains");
            assert!(registry.is_builtin("login"));
            assert!(registry.skipped_acp_names.is_empty());
        }
    }

    #[test]
    fn collision_detection_covers_builtin_aliases() {
        let builtin: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &["quit"],
        });
        let mut registry = CommandRegistry::new(vec![builtin]);
        registry.set_acp_commands(&[agent_client_protocol::AvailableCommand::new(
            "quit".to_string(),
            "Should be dropped".to_string(),
        )]);
        assert_eq!(registry.command_count(), 1);
    }

    #[test]
    fn skill_named_after_blocked_name_is_skipped() {
        let mut registry = CommandRegistry::new(vec![login_builtin()]);
        registry.set_acp_commands(&[acp_skill(
            "hooks-add",
            serde_json::json!({"scope": "local", "path": "/x/SKILL.md"}),
        )]);
        assert!(registry.get("hooks-add").is_none());
        assert!(registry.get("local:hooks-add").is_none());

        registry.set_acp_commands(&[acp_skill(
            "local:hooks-add",
            serde_json::json!({"scope": "local", "path": "/x/SKILL.md"}),
        )]);
        assert!(registry.get("local:hooks-add").is_some());
    }

    #[test]
    fn command_without_required_tools_is_always_visible() {
        let plain: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &[],
        });
        let mut reg = CommandRegistry::new(vec![plain]);
        // Default (None): visible
        assert!(reg.get("exit").is_some());
        // Empty advertised toolset still doesn't hide a no-requirements command.
        reg.set_available_tools(HashSet::new());
        assert!(reg.get("exit").is_some());
        assert!(reg.triggers().iter().any(|t| t.canonical == "exit"));
    }

    #[test]
    fn tool_gated_command_hidden_when_toolset_unknown() {
        let gated: Arc<dyn SlashCommand> = Arc::new(ToolGatedCommand {
            name: "loop",
            required: &["scheduler_create"],
        });
        let reg = CommandRegistry::new(vec![gated]);
        // Tool list not yet known: fail-closed
        // The user can't submit /loop from the home screen and start a session whose model can't actually run scheduler_create
        assert!(reg.get("loop").is_none());
        assert!(!reg.triggers().iter().any(|t| t.canonical == "loop"));
    }

    #[test]
    fn tool_gated_command_hidden_when_required_tool_missing() {
        let gated: Arc<dyn SlashCommand> = Arc::new(ToolGatedCommand {
            name: "loop",
            required: &["scheduler_create"],
        });
        let plain: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &[],
        });
        let mut reg = CommandRegistry::new(vec![gated, plain]);
        // Advertise a toolset missing `scheduler_create`.
        reg.set_available_tools(tool_set(["read_file"]));
        assert!(reg.get("loop").is_none());
        assert!(!reg.triggers().iter().any(|t| t.canonical == "loop"));
        // Plain command is unaffected.
        assert!(reg.get("exit").is_some());
    }

    #[test]
    fn tool_gated_command_reappears_after_tool_added() {
        let gated: Arc<dyn SlashCommand> = Arc::new(ToolGatedCommand {
            name: "loop",
            required: &["scheduler_create"],
        });
        let mut reg = CommandRegistry::new(vec![gated]);
        reg.set_available_tools(HashSet::new());
        assert!(reg.get("loop").is_none());

        // Add the tool; the command becomes visible again
        reg.set_available_tools(tool_set(["scheduler_create"]));
        assert!(reg.get("loop").is_some());
        assert!(reg.triggers().iter().any(|t| t.canonical == "loop"));
    }

    #[test]
    fn multi_tool_command_requires_all_tools() {
        let gated: Arc<dyn SlashCommand> = Arc::new(ToolGatedCommand {
            name: "multi",
            required: &["a", "b"],
        });
        let mut reg = CommandRegistry::new(vec![gated]);

        // Only one of two tools present: hidden
        reg.set_available_tools(tool_set(["a"]));
        assert!(reg.get("multi").is_none());

        // Both tools present: visible
        reg.set_available_tools(tool_set(["a", "b"]));
        assert!(reg.get("multi").is_some());

        // Superset is fine.
        reg.set_available_tools(tool_set(["a", "b", "c"]));
        assert!(reg.get("multi").is_some());
    }

    /// Builds a registry with `always-approve` (plus a `yolo` alias to cover alias key handling), `auto`, and a bystander `exit`.
    fn permission_mode_registry() -> CommandRegistry {
        let always_approve: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "always-approve",
            aliases: &["yolo"],
        });
        let auto: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "auto",
            aliases: &[],
        });
        let exit: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "exit",
            aliases: &[],
        });
        CommandRegistry::new(vec![always_approve, auto, exit])
    }

    /// Menu-only hide: the command disappears from `get()` / triggers.
    /// A typed submission still resolves via `get_for_dispatch()` (including aliases).
    #[test]
    fn menu_hidden_is_menu_only_and_still_dispatches() {
        let mut reg = permission_mode_registry();
        reg.set_auto_mode_available(true);

        reg.set_menu_hidden_for_test("always-approve", true);
        assert!(
            reg.get("always-approve").is_none(),
            "menu lookup hides the command"
        );
        assert!(
            !reg.triggers()
                .iter()
                .any(|t| t.canonical == "always-approve"),
            "no completion trigger while menu-hidden"
        );
        assert!(
            reg.get_for_dispatch("always-approve").is_some(),
            "typed invocation must still resolve for dispatch"
        );
        assert!(
            reg.get_for_dispatch("yolo").is_some(),
            "aliases of a menu-hidden command must still resolve for dispatch"
        );
        // Bystanders unaffected.
        assert!(reg.get("exit").is_some());
        assert!(reg.get("auto").is_some());

        reg.set_menu_hidden_for_test("always-approve", false);
        assert!(reg.get("always-approve").is_some());
        assert!(
            reg.triggers()
                .iter()
                .any(|t| t.canonical == "always-approve")
        );
    }

    /// The `/auto` feature gate stays HARD (fail-closed): gated off, `/auto` is neither offered nor executable.
    /// `get_for_dispatch` must NOT resurrect feature-hidden commands. `/always-approve` is ungated.
    #[test]
    fn auto_feature_gate_blocks_dispatch_resolution() {
        let mut reg = permission_mode_registry();

        // Fail-closed default from `new()`: /auto starts hard-hidden.
        assert!(reg.get_for_dispatch("auto").is_none());
        assert!(reg.get("auto").is_none());

        // Gate on: offered and dispatchable. Always-approve always was.
        reg.set_auto_mode_available(true);
        assert!(reg.get("auto").is_some());
        assert!(reg.get_for_dispatch("auto").is_some());
        assert!(reg.get("always-approve").is_some());
        assert!(reg.get_for_dispatch("always-approve").is_some());

        // Gate off again: /auto gone everywhere; /always-approve stays.
        reg.set_auto_mode_available(false);
        assert!(reg.get("auto").is_none());
        assert!(reg.get_for_dispatch("auto").is_none());
        assert!(!reg.triggers().iter().any(|t| t.canonical == "auto"));
        assert!(reg.get("always-approve").is_some());
        assert!(reg.get_for_dispatch("always-approve").is_some());
    }

    /// `get_for_dispatch` only bypasses the menu-only hide.
    /// Hard-hidden and tool-gated commands stay unresolvable for dispatch, exactly like `get()`.
    #[test]
    fn get_for_dispatch_respects_hard_gates() {
        // Hard-hidden by name (e.g. /dashboard default).
        let dashboard: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "dashboard",
            aliases: &[],
        });
        // Tool-gated (toolset unknown, fail-closed)
        let gated: Arc<dyn SlashCommand> = Arc::new(ToolGatedCommand {
            name: "loop",
            required: &["scheduler_create"],
        });
        let mut reg = CommandRegistry::new(vec![dashboard, gated]);
        reg.set_dashboard_visible(false);

        assert!(
            reg.get_for_dispatch("dashboard").is_none(),
            "hard-hidden stays hard"
        );
        assert!(
            reg.get_for_dispatch("loop").is_none(),
            "tool-gated stays fail-closed pre-handshake"
        );
    }

    #[test]
    fn commands_by_index_in_range_returns_some_out_of_range_returns_none() {
        let alpha: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "alpha",
            aliases: &[],
        });
        let beta: Arc<dyn SlashCommand> = Arc::new(DummyCommand {
            name: "beta",
            aliases: &[],
        });
        let registry = CommandRegistry::new(vec![alpha, beta]);

        // In-range indices resolve to the matching command.
        assert_eq!(
            registry.commands_by_index(0).map(|c| c.name()),
            Some("alpha"),
        );
        assert_eq!(
            registry.commands_by_index(1).map(|c| c.name()),
            Some("beta"),
        );
        // Out-of-range returns None (boundary and far-out)
        assert!(registry.commands_by_index(2).is_none());
        assert!(registry.commands_by_index(usize::MAX).is_none());
    }
}
