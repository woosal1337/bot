// Modified by the Bot project on 2026-09-13: Kept provider-neutral usage and URL tests.
//! Tests for local session usage and browser fallback behavior.

use super::*;

fn is_session_usage_fetch(effects: &[Effect]) -> bool {
    matches!(
        effects,
        [Effect::FetchSessionUsage { agent_id, .. }] if *agent_id == AgentId(0)
    )
}

fn complete_session_usage(
    app: &mut AppView,
    session_id: &str,
    usage: xai_grok_shell::extensions::notification::PromptUsage,
) -> Vec<Effect> {
    dispatch(
        Action::TaskComplete(TaskResult::SessionUsageComplete {
            agent_id: AgentId(0),
            session_id: session_id.to_string().into(),
            usage: Box::new(usage),
            nonce: Default::default(),
        }),
        app,
    )
}

fn fail_session_usage(app: &mut AppView, session_id: &str, error: &str) -> Vec<Effect> {
    dispatch(
        Action::TaskComplete(TaskResult::SessionUsageFailed {
            agent_id: AgentId(0),
            session_id: session_id.to_string().into(),
            error: error.into(),
            nonce: Default::default(),
        }),
        app,
    )
}

#[test]
fn show_usage_schedules_session_fetch_only() {
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    assert!(is_session_usage_fetch(&dispatch(
        Action::ShowUsage,
        &mut app
    )));
}

#[test]
fn show_usage_without_session_reports_unavailable() {
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = None;
    let before = agent_scrollback_len(&app);
    let effects = dispatch(Action::ShowUsage, &mut app);
    assert!(last_system_text(&app, AgentId(0)).contains("unavailable"));
    assert_eq!(agent_scrollback_len(&app), before + 1);
    assert!(effects.is_empty());
}

#[test]
fn session_usage_complete_pushes_local_block() {
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    let before = agent_scrollback_len(&app);
    let usage = xai_grok_shell::extensions::notification::PromptUsage {
        totals: xai_grok_shell::extensions::notification::PromptUsageModel {
            input_tokens: 1_000,
            output_tokens: 100,
            total_tokens: 1_100,
            model_calls: 3,
            cost_usd_ticks: Some(5_000_000_000),
            ..Default::default()
        },
        ..Default::default()
    };
    let effects = complete_session_usage(&mut app, "test-session", usage);
    assert_eq!(agent_scrollback_len(&app), before + 1);
    let text = last_system_text(&app, AgentId(0));
    assert!(
        text.contains("Session usage") && text.contains("$0.5000"),
        "{text}"
    );
    assert!(effects.is_empty());
}

#[test]
fn session_usage_complete_drops_stale_session() {
    let mut app = test_app_with_agent();
    let before = agent_scrollback_len(&app);
    let effects = complete_session_usage(
        &mut app,
        "old-session",
        xai_grok_shell::extensions::notification::PromptUsage {
            totals: xai_grok_shell::extensions::notification::PromptUsageModel {
                model_calls: 99,
                cost_usd_ticks: Some(1_000_000_000_000),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(effects.is_empty());
    assert_eq!(agent_scrollback_len(&app), before);
}

#[test]
fn session_usage_failed_pushes_local_error() {
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    let before = agent_scrollback_len(&app);
    let effects = fail_session_usage(&mut app, "test-session", "boom");
    assert_eq!(agent_scrollback_len(&app), before + 1);
    assert!(last_system_text(&app, AgentId(0)).contains("Couldn't load session usage: boom"));
    assert!(effects.is_empty());
}

#[test]
fn session_usage_failed_drops_stale_session() {
    let mut app = test_app_with_agent();
    let before = agent_scrollback_len(&app);
    assert!(fail_session_usage(&mut app, "old-session", "boom").is_empty());
    assert_eq!(agent_scrollback_len(&app), before);
}

#[test]
fn unknown_command_still_passes_through() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let effects = dispatch(Action::SendPrompt("/frobnicate arg".into()), &mut app);
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(&effects[0], Effect::SendPrompt { text, .. } if text == "/frobnicate arg"),
        "unknown command must still pass through: {effects:?}"
    );
    assert!(app.agents[&id].question_view.is_none());
}

#[serial_test::serial(GROK_TEST_OPEN_URL_FILE)]
#[test]
fn open_url_shows_manual_url_when_browser_unavailable() {
    let bad = std::env::temp_dir().join(format!(
        "bot-open-url-missing-{}/out.txt",
        std::process::id()
    ));
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::set_var("GROK_TEST_OPEN_URL_FILE", &bad) };
    let mut app = test_app_with_agent();
    let before = agent_scrollback_len(&app);
    let url = "https://example.com/account";
    let effects = dispatch(Action::OpenUrl(url.to_string()), &mut app);
    assert!(effects.is_empty());
    assert_eq!(agent_scrollback_len(&app), before + 1);
    assert_eq!(
        last_system_text(&app, AgentId(0)),
        crate::app::link_opener::browser_unavailable_message(url)
    );
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::remove_var("GROK_TEST_OPEN_URL_FILE") };
}

#[serial_test::serial(GROK_TEST_OPEN_URL_FILE)]
#[test]
fn open_url_does_not_show_fallback_when_opener_succeeds() {
    let url_file = std::env::temp_dir().join(format!("bot-open-url-ok-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&url_file);
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::set_var("GROK_TEST_OPEN_URL_FILE", &url_file) };
    let mut app = test_app_with_agent();
    let before = agent_scrollback_len(&app);
    let url = "https://example.com/help";
    let _ = dispatch(Action::OpenUrl(url.to_string()), &mut app);
    assert_eq!(agent_scrollback_len(&app), before);
    let recorded = std::fs::read_to_string(&url_file).unwrap_or_default();
    assert!(recorded.lines().any(|line| line == url));
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::remove_var("GROK_TEST_OPEN_URL_FILE") };
    let _ = std::fs::remove_file(&url_file);
}

#[serial_test::serial(GROK_TEST_OPEN_URL_FILE)]
#[test]
fn open_url_welcome_toasts_single_line_url_when_browser_unavailable() {
    let bad = std::env::temp_dir().join(format!(
        "bot-open-url-welcome-missing-{}/out.txt",
        std::process::id()
    ));
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::set_var("GROK_TEST_OPEN_URL_FILE", &bad) };
    let mut app = test_app();
    let url = "https://example.com/settings";
    let effects = dispatch(Action::OpenUrl(url.to_string()), &mut app);
    assert!(effects.is_empty());
    let toast = app
        .welcome_toast
        .as_ref()
        .map(|(message, _)| message.as_str())
        .unwrap_or("");
    assert!(toast.starts_with(url));
    assert!(!toast.contains('\n'));
    // SAFETY: serial_test prevents concurrent access to the process environment.
    unsafe { std::env::remove_var("GROK_TEST_OPEN_URL_FILE") };
}
