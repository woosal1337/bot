// Modified by the Bot project on 2026-09-13: verify provider-gated extension fetches and recovery.
//! Tests for session-related modals (extensions, /new worktree question)
//! and session close helpers shared with the dashboard.

use super::*;

#[test]
fn open_extensions_modal_no_session_sets_flag_no_fetches() {
    use crate::views::extensions_modal::ExtensionsTab;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let effects = dispatch(
        Action::OpenExtensionsModal {
            tab: ExtensionsTab::Hooks,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        },
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 0);
    assert!(app.agents[&id].pending_extensions_fetch);
    assert!(app.agents[&id].extensions_modal.is_some());
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "opening the modal must not create a session, got {effects:?}"
    );
}

#[test]
fn open_extensions_modal_with_session_emits_fetches_no_flag() {
    use crate::views::extensions_modal::ExtensionsTab;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let effects = dispatch(
        Action::OpenExtensionsModal {
            tab: ExtensionsTab::Hooks,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        },
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 5);
    assert!(!app.agents[&id].pending_extensions_fetch);
}

#[test]
fn open_extensions_modal_with_session_resets_stale_flag() {
    use crate::views::extensions_modal::ExtensionsTab;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().pending_extensions_fetch = true;
    let effects = dispatch(
        Action::OpenExtensionsModal {
            tab: ExtensionsTab::Hooks,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        },
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 5);
    assert!(!app.agents[&id].pending_extensions_fetch);
}

#[test]
fn codex_extension_fetches_match_the_provider_matrix() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};

    let mut modal = ExtensionsModalState::new_for_provider(
        ExtensionsTab::Hooks,
        &crate::provider::ProviderId::Codex,
    );
    let effects = crate::app::dispatch::transcript::extensions_modal_tab_fetches(
        &mut modal,
        AgentId(0),
        acp::SessionId::new("codex-session"),
    );

    assert_eq!(effects.len(), 4);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchHooksList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchPluginsList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchSkillsList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchMcpsList { .. }))
    );
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::FetchMarketplaceList { .. } | Effect::FetchWorkflowsList { .. }
    )));
}

#[test]
fn reload_skills_marks_both_lists_loading_and_refetches() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let mut modal = ExtensionsModalState::new(ExtensionsTab::Workflows);
    modal.skills_data = TabDataState::Loaded(vec![]);
    modal.workflows_data = TabDataState::Loaded(vec![]);
    app.agents.get_mut(&id).unwrap().extensions_modal = Some(modal);

    let effects = dispatch(Action::ReloadSkills, &mut app);

    // The router arm is the sole owner of the Loading transitions; the modal key handler only emits the action
    let modal = app.agents[&id].extensions_modal.as_ref().unwrap();
    assert!(matches!(modal.skills_data, TabDataState::Loading));
    assert!(matches!(modal.workflows_data, TabDataState::Loading));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchSkillsList { .. })),
        "reload must refetch skills, got {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchWorkflowsList { .. })),
        "reload must refetch workflows, got {effects:?}"
    );
}

#[test]
fn codex_skill_reload_does_not_call_the_workflow_api() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};

    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let mut modal = ExtensionsModalState::new_for_provider(
        ExtensionsTab::Skills,
        &crate::provider::ProviderId::Codex,
    );
    modal.skills_data = TabDataState::Loaded(vec![]);
    modal.workflows_data = TabDataState::Loaded(vec![]);
    app.agents.get_mut(&id).unwrap().extensions_modal = Some(modal);

    let effects = dispatch(Action::ReloadSkills, &mut app);

    assert!(matches!(
        app.agents[&id]
            .extensions_modal
            .as_ref()
            .unwrap()
            .skills_data,
        TabDataState::Loading
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchSkillsList { .. }))
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchWorkflowsList { .. }))
    );
}

#[test]
fn a_hook_fetch_failure_does_not_clear_other_extension_data() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};

    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let mut modal = ExtensionsModalState::new_for_provider(
        ExtensionsTab::Hooks,
        &crate::provider::ProviderId::Codex,
    );
    modal.plugins_data =
        TabDataState::Loaded(xai_hooks_plugins_types::PluginsListResponse { plugins: vec![] });
    modal.skills_data = TabDataState::Loaded(vec![]);
    app.agents.get_mut(&id).unwrap().extensions_modal = Some(modal);

    let effects = dispatch(
        Action::TaskComplete(TaskResult::HooksListLoaded {
            agent_id: id,
            result: Err("Method not found".into()),
        }),
        &mut app,
    );

    assert!(effects.is_empty());
    let modal = app.agents[&id].extensions_modal.as_ref().unwrap();
    assert!(matches!(modal.hooks_data, TabDataState::Error(_)));
    assert!(matches!(modal.plugins_data, TabDataState::Loaded(_)));
    assert!(matches!(modal.skills_data, TabDataState::Loaded(_)));
    assert_eq!(modal.active_tab, ExtensionsTab::Hooks);
    assert!(app.agents[&id].session.session_id.is_some());
}

#[test]
fn a_failed_hook_reload_updates_the_hook_tab_after_a_tab_switch() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};

    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let mut modal = ExtensionsModalState::new_for_provider(
        ExtensionsTab::Hooks,
        &crate::provider::ProviderId::Codex,
    );
    modal.hooks_data = TabDataState::Loading;
    modal.plugins_data =
        TabDataState::Loaded(xai_hooks_plugins_types::PluginsListResponse { plugins: vec![] });
    modal.active_tab = ExtensionsTab::Plugins;
    modal.pending_action = Some("Reloading...".into());
    app.agents.get_mut(&id).unwrap().extensions_modal = Some(modal);

    let effects = dispatch(
        Action::TaskComplete(TaskResult::HooksActionResult {
            agent_id: id,
            result: Err("Codex hooks reload failed".into()),
        }),
        &mut app,
    );

    assert!(effects.is_empty());
    let modal = app.agents[&id].extensions_modal.as_ref().unwrap();
    assert!(matches!(
        &modal.hooks_data,
        TabDataState::Error(message) if message == "Codex hooks reload failed"
    ));
    assert!(matches!(modal.plugins_data, TabDataState::Loaded(_)));
    assert_eq!(modal.active_tab, ExtensionsTab::Plugins);
    assert_eq!(modal.pending_action, None);
    assert_eq!(modal.modal_message, None);
    assert!(app.agents[&id].session.session_id.is_some());
}

#[test]
fn reload_skills_without_session_keeps_loaded_state() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let mut modal = ExtensionsModalState::new(ExtensionsTab::Workflows);
    modal.skills_data = TabDataState::Loaded(vec![]);
    modal.workflows_data = TabDataState::Loaded(vec![]);
    app.agents.get_mut(&id).unwrap().extensions_modal = Some(modal);

    let effects = dispatch(Action::ReloadSkills, &mut app);

    // Nothing can fetch without a session, so nothing may flip to Loading; a stranded spinner would make repeat presses no-ops
    assert!(effects.is_empty(), "got {effects:?}");
    let modal = app.agents[&id].extensions_modal.as_ref().unwrap();
    assert!(matches!(modal.skills_data, TabDataState::Loaded(_)));
    assert!(matches!(modal.workflows_data, TabDataState::Loaded(_)));
}

#[test]
fn session_created_with_flag_but_modal_closed_clears_flag_no_fetches() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.session.session_id = None;
        a.pending_extensions_fetch = true;
        a.extensions_modal = None;
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: acp::SessionId::new("s"),
            models: None,
        }),
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 0);
    assert!(!app.agents[&id].pending_extensions_fetch);
}

// ── /new dispatcher tests ─────────────────────────────────────────────

#[test]
fn dispatch_new_session_opens_question_modal_in_git_repo() {
    let mut app = new_session_test_app();
    app.new_session_worktree_mode = crate::app::app_view::WorktreeMode::Ask;
    let effects = dispatch(Action::NewSession, &mut app);
    assert!(effects.is_empty(), "no effects until modal answered");
    // No new agent yet (creation is deferred until modal answered).
    assert_eq!(app.agents.len(), 1);
    let qv = app.agents[&AgentId(0)]
        .question_view
        .as_ref()
        .expect("modal must be open");
    match qv.local_kind.as_ref().expect("local_kind must be set") {
        crate::views::question_view::LocalQuestionKind::NewSession => {}
        other => panic!("expected NewSession, got {other:?}"),
    }
    assert_eq!(
        qv.questions[0].options.len(),
        4,
        "modal must offer exactly 4 options (Yes/No/Always/Never)"
    );
    let labels: Vec<&str> = qv.questions[0]
        .options
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["Yes", "No", "Always worktree", "Never worktree"]
    );
}

#[test]
fn dispatch_new_session_skips_modal_in_non_git_repo() {
    // current_branch stays None (no git repo), so no modal opens and dispatch goes straight to dispatch_new_session_inner
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::NewSession, &mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "non-git path must emit CreateSession, got {effects:?}"
    );
    assert!(
        app.agents.values().all(|a| a.question_view.is_none()),
        "non-git path must not open the modal"
    );
}

// ── Session close (shared with dashboard) ─────────────────────────────

#[test]
fn close_inactive_agent_drops_it() {
    let mut app = three_agent_app();
    let effects = dispatch_sessions_confirm_close(&mut app, AgentId(2));
    assert!(
        effects
            .iter()
            .all(|e| matches!(e, Effect::UnregisterActiveSession { .. }))
    );
    assert!(!app.agents.contains_key(&AgentId(2)));
    assert_eq!(app.agents.len(), 2);
}

#[test]
fn close_agent_releases_retained_memory() {
    use crate::memory_release::test_support;
    test_support::install_counting_hook();

    let mut app = three_agent_app();

    // Dropping a real AgentView (scrollback, caches, child views) purges
    let before = test_support::calls();
    dispatch_sessions_confirm_close(&mut app, AgentId(2));
    assert!(!app.agents.contains_key(&AgentId(2)));
    assert_eq!(
        test_support::calls(),
        before + 1,
        "dropping the closed AgentView must purge retained pages"
    );

    // Closing an unknown agent drops nothing, so no purge
    let before = test_support::calls();
    dispatch_sessions_confirm_close(&mut app, AgentId(999));
    assert_eq!(
        test_support::calls(),
        before,
        "a no-op close must not purge"
    );
}

#[test]
fn close_clears_forked_from_on_surviving_children() {
    let mut app = three_agent_app();
    set_forked_from(&mut app, AgentId(2), AgentId(1));
    dispatch_sessions_confirm_close(&mut app, AgentId(1));
    assert!(
        app.agents[&AgentId(2)].session.forked_from.is_none(),
        "stale forked_from pointer must be cleared after parent close"
    );
}

#[test]
fn close_only_agent_is_refused_with_toast() {
    let mut app = test_app_with_agent();
    let agents_before = app.agents.len();
    dispatch_sessions_confirm_close(&mut app, AgentId(0));
    assert_eq!(
        app.agents.len(),
        agents_before,
        "the only agent must NOT be closed"
    );
}

#[test]
fn close_unknown_agent_is_silent_noop() {
    let mut app = three_agent_app();
    let agents_before = app.agents.len();
    dispatch_sessions_confirm_close(&mut app, AgentId(999));
    assert_eq!(app.agents.len(), agents_before);
}

#[test]
fn close_only_agent_short_circuits_before_reaching_welcome_fallback() {
    let mut app = test_app_with_agent();
    assert!(matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(0)));
    dispatch_sessions_confirm_close(&mut app, AgentId(0));
    assert!(matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(0)));
    assert!(app.agents.contains_key(&AgentId(0)));
}

#[test]
fn close_does_not_disturb_unrelated_forked_from_pointers() {
    let mut app = three_agent_app();
    set_forked_from(&mut app, AgentId(1), AgentId(0));
    set_forked_from(&mut app, AgentId(2), AgentId(0));
    dispatch_sessions_confirm_close(&mut app, AgentId(1));
    assert_eq!(
        app.agents[&AgentId(2)].session.forked_from,
        Some(AgentId(0)),
        "unrelated forked_from must NOT be cleared"
    );
}

fn count_marketplace_fetches(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::FetchMarketplaceList { .. }))
        .count()
}

fn success_outcome() -> xai_hooks_plugins_types::ActionOutcome {
    xai_hooks_plugins_types::ActionOutcome {
        status: xai_hooks_plugins_types::OutcomeStatus::Success,
        message: "ok".into(),
        requires_reload: false,
        requires_restart: false,
    }
}

#[test]
fn a_codex_hook_action_refreshes_only_hooks() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};

    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().extensions_modal =
        Some(ExtensionsModalState::new_for_provider(
            ExtensionsTab::Hooks,
            &crate::provider::ProviderId::Codex,
        ));

    let effects = dispatch(
        Action::TaskComplete(TaskResult::HooksActionResult {
            agent_id: id,
            result: Ok(success_outcome()),
        }),
        &mut app,
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::FetchHooksList { .. }]
    ));
}

#[test]
fn a_codex_plugin_action_refreshes_each_supported_extension_surface() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};

    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().extensions_modal =
        Some(ExtensionsModalState::new_for_provider(
            ExtensionsTab::Plugins,
            &crate::provider::ProviderId::Codex,
        ));

    let effects = dispatch(
        Action::TaskComplete(TaskResult::PluginsActionResult {
            agent_id: id,
            result: Ok(success_outcome()),
        }),
        &mut app,
    );

    assert_eq!(effects.len(), 5);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshAvailableCommands { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchHooksList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchPluginsList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchSkillsList { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchMcpsList { cache: false, .. }))
    );
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::FetchMarketplaceList { .. } | Effect::FetchWorkflowsList { .. }
    )));
}

fn empty_marketplace_response() -> xai_hooks_plugins_types::MarketplaceListResponse {
    xai_hooks_plugins_types::MarketplaceListResponse { sources: vec![] }
}

#[test]
fn marketplace_fetch_coalesces_while_inflight() {
    use crate::views::extensions_modal::ExtensionsTab;
    let mut app = test_app_with_agent();
    let id = AgentId(0);

    let effects = dispatch(
        Action::OpenExtensionsModal {
            tab: ExtensionsTab::Marketplace,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        },
        &mut app,
    );
    assert_eq!(count_marketplace_fetches(&effects), 1);

    // A successful action while the open-fetch is still in flight must not stack a second scan; it queues one refetch instead
    let effects = dispatch(
        Action::TaskComplete(TaskResult::PluginsActionResult {
            agent_id: id,
            result: Ok(success_outcome()),
        }),
        &mut app,
    );
    assert_eq!(count_marketplace_fetches(&effects), 0);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchHooksList { .. })),
        "non-marketplace refetches still fire"
    );

    // When the in-flight fetch lands, the queued refetch fires exactly once.
    let effects = dispatch(
        Action::TaskComplete(TaskResult::MarketplaceListLoaded {
            agent_id: id,
            result: Ok(empty_marketplace_response()),
        }),
        &mut app,
    );
    assert_eq!(count_marketplace_fetches(&effects), 1);

    // And the queue drains: the refetch landing issues nothing further.
    let effects = dispatch(
        Action::TaskComplete(TaskResult::MarketplaceListLoaded {
            agent_id: id,
            result: Ok(empty_marketplace_response()),
        }),
        &mut app,
    );
    assert_eq!(count_marketplace_fetches(&effects), 0);
}

#[test]
fn marketplace_fetch_fires_immediately_when_idle() {
    use crate::views::extensions_modal::ExtensionsTab;
    let mut app = test_app_with_agent();
    let id = AgentId(0);

    dispatch(
        Action::OpenExtensionsModal {
            tab: ExtensionsTab::Marketplace,
            trigger: xai_grok_telemetry::events::ExtensionsModalTrigger::SlashCommand,
        },
        &mut app,
    );
    dispatch(
        Action::TaskComplete(TaskResult::MarketplaceListLoaded {
            agent_id: id,
            result: Ok(empty_marketplace_response()),
        }),
        &mut app,
    );

    // Nothing in flight: an action-triggered refetch goes out immediately.
    let effects = dispatch(
        Action::TaskComplete(TaskResult::PluginsActionResult {
            agent_id: id,
            result: Ok(success_outcome()),
        }),
        &mut app,
    );
    assert_eq!(count_marketplace_fetches(&effects), 1);
}
