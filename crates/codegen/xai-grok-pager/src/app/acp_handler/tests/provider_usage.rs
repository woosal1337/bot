use super::*;

fn notification(session_id: &str) -> acp::ExtNotification {
    let update = bot_core::ProviderUsageUpdate {
        session_id: session_id.to_owned(),
        usage: bot_core::ProviderUsage {
            provider: bot_core::ProviderId::Codex,
            lifetime_tokens: Some(1_234_567),
            limits: vec![bot_core::UsageLimit {
                id: Some("codex".to_owned()),
                name: "Codex".to_owned(),
                model: None,
                windows: vec![bot_core::UsageLimitWindow {
                    label: "Secondary".to_owned(),
                    used_percent: 8.0,
                    duration_minutes: Some(10_080),
                    resets_at: None,
                }],
            }],
            extensions: Default::default(),
        },
    };
    let raw = serde_json::value::to_raw_value(&update).unwrap();
    acp::ExtNotification::new(bot_core::PROVIDER_USAGE_UPDATED_METHOD, raw.into())
}

#[test]
fn applies_provider_usage_to_the_target_session() {
    let mut app = make_app_with_agent("sess-1");

    assert!(handle_ext_notification(&notification("sess-1"), &mut app));
    let usage = app
        .agents
        .get(&AgentId(0))
        .and_then(|agent| agent.provider_usage.as_ref())
        .expect("provider usage");
    assert_eq!(usage.provider, bot_core::ProviderId::Codex);
    assert_eq!(usage.lifetime_tokens, Some(1_234_567));
    assert_eq!(
        usage
            .longest_window()
            .map(|(_, window)| window.remaining_percent()),
        Some(92.0)
    );
}

#[test]
fn ignores_provider_usage_for_an_unknown_session() {
    let mut app = make_app_with_agent("sess-1");

    assert!(!handle_ext_notification(
        &notification("sess-other"),
        &mut app
    ));
    assert!(
        app.agents
            .get(&AgentId(0))
            .and_then(|agent| agent.provider_usage.as_ref())
            .is_some_and(|usage| usage.provider == bot_core::ProviderId::Grok
                && usage.lifetime_tokens.is_none()
                && usage.limits.is_empty())
    );
}
