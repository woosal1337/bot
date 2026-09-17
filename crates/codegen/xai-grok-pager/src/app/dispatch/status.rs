// Modified by the Bot project on 2026-09-13: Renamed the local document action.
//! Session status, usage, and info dispatchers.

use agent_client_protocol as acp;

use super::ctx::get_active_agent;
use super::queue::push_and_page_flip;
use crate::app::actions::Effect;
use crate::app::agent::AgentId;
use crate::app::agent_view::AgentView;
use crate::app::app_view::{ActiveView, AppView};
use crate::notifications::{NotificationEvent, NotificationEventKind};
use crate::scrollback::block::RenderBlock;

/// Monotonic generation for usage-modal fetches, shared by the agent-hosted and dashboard-hosted modal.
/// A reply from a previous open (modal closed and reopened) then can't overwrite newer results.
static USAGE_FETCH_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_usage_fetch_nonce() -> u64 {
    USAGE_FETCH_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// The agent's open usage modal state, if any.
pub(super) fn usage_modal_state_mut(
    agent: &mut AgentView,
) -> Option<&mut crate::views::usage_modal::UsageInfoModalState> {
    match agent.active_modal.as_mut() {
        Some(crate::views::modal::ActiveModal::UsageInfo { state }) => Some(state),
        _ => None,
    }
}

/// Open (or re-tab) the usage/session-info modal and fire the fetch effects that populate it.
/// Full-TUI only; minimal mode keeps scrollback blocks.
pub(super) fn open_usage_info_modal(
    app: &mut AppView,
    tab: crate::views::usage_modal::UsageInfoTab,
) -> Vec<Effect> {
    use crate::views::modal::ActiveModal;
    use crate::views::usage_modal::{UsageInfoContext, UsageInfoModalState};

    if matches!(app.active_view, ActiveView::AgentDashboard) {
        return open_dashboard_usage_modal(app, tab);
    }
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let show_resolved_model = app.show_resolved_model;
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    let session_id = agent.session.session_id.clone();

    if let Some(state) = usage_modal_state_mut(agent) {
        state.set_tab(tab);
        return vec![];
    }

    let nonce = next_usage_fetch_nonce();
    let mut state = UsageInfoModalState::new(
        tab,
        UsageInfoContext {
            session_id: session_id.as_ref().map(|s| s.0.to_string()),
        },
    );
    state.provider_usage = agent.provider_usage.clone();
    state.fetch_nonce = nonce;

    let mut effects = Vec::new();
    if let Some(session_id) = session_id {
        effects.push(Effect::ShowContextInfo {
            agent_id: id,
            session_id: session_id.clone(),
            nonce,
        });
        effects.push(Effect::ShowSessionInfo {
            agent_id: id,
            session_id: session_id.clone(),
            show_resolved_model,
            nonce,
        });
        if crate::provider::active_provider() == crate::provider::ProviderId::Grok {
            state.session_usage_pending = true;
            effects.push(Effect::FetchSessionUsage {
                agent_id: id,
                session_id,
                nonce,
            });
        }
    }
    agent.active_modal = Some(ActiveModal::UsageInfo {
        state: Box::new(state),
    });
    effects
}

fn open_dashboard_usage_modal(
    app: &mut AppView,
    tab: crate::views::usage_modal::UsageInfoTab,
) -> Vec<Effect> {
    use crate::views::usage_modal::{UsageInfoContext, UsageInfoModalState};

    let ctx = UsageInfoContext { session_id: None };
    let Some(dashboard) = app.dashboard.as_mut() else {
        return vec![];
    };
    if let Some(state) = dashboard.usage_modal.as_mut() {
        state.set_tab(tab);
        return vec![];
    }
    let state = UsageInfoModalState::new(tab, ctx);
    dashboard.usage_modal = Some(Box::new(state));
    vec![]
}

/// `/session-info`: open the usage modal on its "Session info" tab, or fetch-and-show in scrollback in minimal mode.
pub(super) fn dispatch_show_session_info(app: &mut AppView) -> Vec<Effect> {
    if !app.screen_mode.is_minimal() {
        return open_usage_info_modal(app, crate::views::usage_modal::UsageInfoTab::SessionInfo);
    }
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    let Some(session_id) = agent.session.session_id.clone() else {
        // No active session; the slash command should have caught this, but guard here just in case
        return vec![];
    };

    vec![Effect::ShowSessionInfo {
        agent_id: id,
        session_id,
        show_resolved_model: app.show_resolved_model,
        nonce: Default::default(),
    }]
}

pub(super) fn dispatch_show_account(app: &mut AppView) -> Vec<Effect> {
    app.show_toast("Loading account…");
    vec![Effect::FetchAccountStatus]
}

/// Scrub an untrusted error string for toast display.
/// Substitutes a generic placeholder when the input exceeds 120 chars or contains control / bidi-override characters.
/// That prevents escape-sequence injection and visual spoofing.
pub(super) fn scrub_error_for_toast(error: &str) -> String {
    const MAX_TOAST_ERROR_LEN: usize = 120;
    if error.len() > MAX_TOAST_ERROR_LEN
        || error
            .chars()
            .any(crate::render::line_utils::is_unsafe_display_char)
    {
        "server error (see logs for details)".to_string()
    } else {
        error.to_string()
    }
}

/// `/context` and the context-bar click: open the usage modal on its "Context usage" tab, or fetch-and-show in scrollback in minimal mode.
pub(super) fn dispatch_show_context_info(app: &mut AppView) -> Vec<Effect> {
    if !app.screen_mode.is_minimal() {
        return open_usage_info_modal(app, crate::views::usage_modal::UsageInfoTab::ContextUsage);
    }
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    let Some(session_id) = agent.session.session_id.clone() else {
        return vec![];
    };

    vec![Effect::ShowContextInfo {
        agent_id: id,
        session_id,
        nonce: Default::default(),
    }]
}

/// `/usage`: open the usage modal on its "Session usage" tab.
pub(super) fn dispatch_show_usage(app: &mut AppView) -> Vec<Effect> {
    if !app.screen_mode.is_minimal() {
        return open_usage_info_modal(app, crate::views::usage_modal::UsageInfoTab::SessionUsage);
    }
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let session_id = {
        let Some(agent) = app.agents.get_mut(&id) else {
            return vec![];
        };
        if crate::provider::active_provider() != crate::provider::ProviderId::Grok {
            let text = agent
                .provider_usage
                .as_ref()
                .map(|usage| {
                    crate::views::provider_usage::detail_text(
                        usage,
                        &crate::theme::Theme::current(),
                    )
                })
                .unwrap_or_else(|| "Usage data is unavailable.".to_owned());
            push_and_page_flip(&mut agent.scrollback, RenderBlock::system(text));
            return vec![];
        }
        agent.session.session_id.clone()
    };
    match session_id {
        Some(session_id) => vec![Effect::FetchSessionUsage {
            agent_id: id,
            session_id,
            nonce: Default::default(),
        }],
        None => {
            if let Some(agent) = app.agents.get_mut(&id) {
                push_and_page_flip(
                    &mut agent.scrollback,
                    RenderBlock::system(
                        "Session usage is unavailable until the session starts.".to_string(),
                    ),
                );
            }
            vec![]
        }
    }
}

/// Route a session-usage result (success or failure text) into the open usage modal, or into scrollback in minimal mode.
/// Stale results are dropped.
pub(super) fn handle_session_usage_result(
    app: &mut AppView,
    agent_id: AgentId,
    session_id: &acp::SessionId,
    text: String,
    nonce: u64,
) -> Vec<Effect> {
    if !app.screen_mode.is_minimal() {
        if let Some(agent) = app.agents.get_mut(&agent_id) {
            if agent.session.session_id.as_ref() != Some(session_id) {
                return vec![];
            }
            if let Some(state) = usage_modal_state_mut(agent)
                && state.fetch_nonce == nonce
            {
                state.session_usage_pending = false;
                state.session_usage_text = Some(text);
            }
        }
        return vec![];
    }
    commit_session_usage_block(app, agent_id, session_id, text)
}

pub(super) fn commit_session_usage_block(
    app: &mut AppView,
    agent_id: AgentId,
    session_id: &acp::SessionId,
    text: String,
) -> Vec<Effect> {
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    if agent.session.session_id.as_ref() != Some(session_id) {
        return vec![];
    }
    push_and_page_flip(&mut agent.scrollback, RenderBlock::system(text));
    vec![]
}

/// Commit a one-line "update available" notice into the active agent's scrollback.
/// Minimal mode has no welcome screen (where the full TUI shows updates), so the background update check's result is shown here instead.
/// No-op when there is no active agent.
pub(crate) fn commit_minimal_update_notice(app: &mut AppView, latest_version: &str) {
    if let ActiveView::Agent(id) = app.active_view
        && let Some(agent) = app.agents.get_mut(&id)
    {
        agent.scrollback.push_block(RenderBlock::system(format!(
            "Update available: v{latest_version}. Restart to apply."
        )));
    }
}

/// `/queue`: commit a read-only list of the queued prompts as a system block.
/// The text is built by [`crate::app::status_blocks::queue_block_text`]; this just resolves the active agent and pushes it.
/// Works in every render mode; in minimal, which has no interactive `QueuePane`, it is the primary way to inspect the queue.
pub(super) fn dispatch_show_queue(app: &mut AppView) -> Vec<Effect> {
    if let ActiveView::Agent(id) = app.active_view
        && let Some(agent) = app.agents.get_mut(&id)
    {
        let text = crate::app::status_blocks::queue_block_text(agent);
        agent.scrollback.push_block(RenderBlock::system(text));
    }
    vec![]
}

/// `/tasks`: commit a read-only list of background tasks, subagents, and scheduled (`/loop`) tasks as a system block.
/// The text is built by [`crate::app::status_blocks::tasks_block_text`]; this just resolves the active agent and pushes it.
/// Works in every render mode; in minimal, which has no interactive `TasksPane`, it is the primary task snapshot.
pub(super) fn dispatch_show_tasks(app: &mut AppView) -> Vec<Effect> {
    if let ActiveView::Agent(id) = app.active_view
        && let Some(agent) = app.agents.get_mut(&id)
    {
        let text = crate::app::status_blocks::tasks_block_text(agent);
        agent.scrollback.push_block(RenderBlock::system(text));
    }
    vec![]
}

/// Open the hidden `/gboom` easter egg as a modal over the active agent view.
/// Requires a graphics-capable terminal (kitty protocol or iTerm2); otherwise a toast explains why nothing happened.
/// On session-less views (dashboard, welcome) this is a silent no-op.
pub(super) fn dispatch_open_gboom(app: &mut AppView) -> Vec<Effect> {
    use crate::terminal::image::{GraphicsProtocol, detect_graphics_protocol};
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    if detect_graphics_protocol() == GraphicsProtocol::None {
        agent.show_toast(
            "No demons here: GBOOM needs a graphics-capable terminal \
             (kitty, Ghostty, WezTerm, iTerm2)",
        );
        return vec![];
    }
    // Close other media modals: they share the kitty placement id
    // Drop the image viewer's in-flight loader too (its close path clears both; a leaked rx would mis-feed the next image viewer's poll loop)
    agent.image_viewer = None;
    agent.image_load_rx = None;
    agent.video_viewer = None;
    let mut game = crate::gboom::GboomState::new();
    // Deliberately `kitty_flags_pushed`, not `kitty_releases_reported`: the game pushes its own REPORT_ALL_KEYS layer over a downgraded base
    game.set_release_aware(crate::terminal::kitty_flags_pushed());
    agent.gboom = Some(game);
    vec![]
}

/// Emit a `SessionReady` notification for the given agent.
///
/// Takes `&NotificationService` separately from `&AgentView` to avoid borrow-checker conflicts when `agent` is borrowed from `app.agents`.
pub(super) fn notify_session_ready(
    notification_service: &crate::notifications::NotificationService,
    agent: &AgentView,
) {
    notification_service.notify(NotificationEvent {
        kind: NotificationEventKind::SessionReady,
        title: "Bot".into(),
        body: NotificationEventKind::SessionReady.as_ref().into(),
        session_id: agent.session.session_id.as_ref().map(|s| s.0.to_string()),
    });
}

// TaskResult handlers.

pub(super) fn handle_context_info_complete(
    app: &mut AppView,
    agent_id: AgentId,
    session_id: &acp::SessionId,
    info: Box<xai_grok_shell::session::SessionInfoResponse>,
    nonce: u64,
) -> Vec<Effect> {
    let minimal = app.screen_mode.is_minimal();
    if let Some(agent) = app.agents.get_mut(&agent_id) {
        if agent.session.session_id.as_ref() != Some(session_id) {
            return vec![];
        }
        // A reply from a previous modal open must not touch anything, not even the agent's context mirrors, which a fresher reply already set
        if let Some(state) = usage_modal_state_mut(agent)
            && state.fetch_nonce != nonce
        {
            return vec![];
        }
        let model = info.data.model.as_deref().unwrap_or("unknown").to_string();
        let snapshot = info.data.context;
        agent.apply_full_context_info(snapshot.clone());
        if let Some(state) = usage_modal_state_mut(agent) {
            state.context = Some(crate::scrollback::blocks::ContextInfoBlock::new(
                snapshot, model,
            ));
            state.context_error = None;
        } else if minimal {
            push_and_page_flip(
                &mut agent.scrollback,
                crate::scrollback::block::RenderBlock::context_info(snapshot, model),
            );
        }
        // Full mode with the modal closed: the result arrived after dismissal, so drop it
    }
    vec![]
}

// Action handlers.

pub(super) fn dispatch_copy_session_id(app: &mut AppView, index: usize) -> Vec<Effect> {
    use crate::views::modal::ActiveModal;
    // Try agent modal first, then fall back to app fields (welcome screen).
    let id = get_active_agent(app)
        .and_then(|agent| {
            if let Some(ActiveModal::SessionPicker {
                entries: Some(ref e),
                ..
            }) = agent.active_modal
            {
                e.get(index).map(|entry| entry.id.clone())
            } else {
                None
            }
        })
        .or_else(|| {
            app.session_picker_entries
                .as_ref()
                .and_then(|s| s.get(index))
                .map(|e| e.id.clone())
        });
    if let Some(id) = id {
        let delivery = crate::clipboard::copy_text_or_file(&id);
        app.show_toast(delivery.toast_message().as_ref());
    }
    vec![]
}

/// Open the onboarding tutorial overlay (a top-level modal; works over both the welcome screen and an agent session).
/// Toggles: dispatching while open closes instead of stacking.
pub(super) fn dispatch_open_tutorial(app: &mut AppView) -> Vec<Effect> {
    // Minimal mode has no modal host: the overlay would render nothing while the app-level intercept swallowed all input
    if app.screen_mode.is_minimal() {
        return vec![];
    }
    if app.tutorial.is_some() {
        app.tutorial = None;
        return vec![];
    }
    app.tutorial = Some(crate::views::tutorial::TutorialState::new());
    vec![]
}

pub(super) fn dispatch_show_document(
    app: &mut AppView,
    title: String,
    content: String,
) -> Vec<Effect> {
    match app.active_view {
        ActiveView::Agent(id) => {
            if let Some(agent) = app.agents.get_mut(&id) {
                agent.active_modal = Some(crate::views::modal::ActiveModal::DocViewer {
                    title,
                    content,
                    scroll: 0,
                    window: crate::views::modal_window::ModalWindowState::new(),
                    cached_lines: None,
                    previous_palette: None,
                    standalone: true,
                });
            }
        }
        ActiveView::Welcome => {
            app.welcome_doc_viewer = Some(crate::views::modal::ActiveModal::DocViewer {
                title,
                content,
                scroll: 0,
                window: crate::views::modal_window::ModalWindowState::new(),
                cached_lines: None,
                previous_palette: None,
                standalone: true,
            });
        }
        _ => {}
    }
    vec![]
}
