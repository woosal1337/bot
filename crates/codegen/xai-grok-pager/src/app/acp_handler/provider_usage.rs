use super::*;

pub(super) fn handle_provider_usage(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    let Ok(update) = serde_json::from_str::<bot_core::ProviderUsageUpdate>(notif.params.get())
    else {
        tracing::warn!("Ignoring invalid Bot provider usage update");
        return false;
    };
    let session_id = acp::SessionId::new(update.session_id);
    let Some(matched) = find_session_match(app, &session_id) else {
        return false;
    };
    let agent_id = matched.agent_id();
    let is_active = is_matched_agent_active(app, agent_id);
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return false;
    };
    agent.provider_usage = Some(update.usage.clone());
    if let Some(crate::views::modal::ActiveModal::UsageInfo { state }) = agent.active_modal.as_mut()
    {
        state.provider_usage = Some(update.usage);
    }
    is_active
}
