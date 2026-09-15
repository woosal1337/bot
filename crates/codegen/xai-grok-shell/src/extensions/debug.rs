// Modified by the Bot project on 2026-09-13: remove xAI feedback runtime.
//! `x.ai/debug/*` extension handlers for local client testing.
//!
//! - `arm_auto_compact`: make the next turn trigger auto-compaction unconditionally, regardless of context window usage.
//! - `agent`: agent-process diagnostics (registry counts).

use agent_client_protocol as acp;

use super::{ExtResult, parse_params};
use crate::agent::MvpAgent;
use crate::session::ExtMethodResult;

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    match args.method.as_ref() {
        "x.ai/debug/arm_auto_compact" => handle_arm_auto_compact(agent, args),
        "x.ai/debug/agent" => handle_agent(agent).await,
        _ => Err(acp::Error::method_not_found()),
    }
}

async fn handle_agent(agent: &MvpAgent) -> ExtResult {
    let registries = agent.registry_snapshot().await;
    ExtMethodResult::success(serde_json::json!({ "registries": registries }))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}

fn handle_arm_auto_compact(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let params: serde_json::Value = parse_params(args)?;

    let session_id_str = params["sessionId"]
        .as_str()
        .or_else(|| params["session_id"].as_str())
        .ok_or_else(|| acp::Error::invalid_params().data("sessionId required"))?;
    let session_id = acp::SessionId::new(session_id_str);

    let handle = agent
        .resident_handle(&session_id)
        .ok_or_else(|| acp::Error::invalid_params().data("unknown session id"))?;

    handle
        .force_compact
        .store(true, std::sync::atomic::Ordering::Relaxed);

    tracing::info!(
        session_id = %session_id_str,
        "debug: armed auto-compact for next turn"
    );

    ExtMethodResult::success(serde_json::json!({ "armed": true }))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}
