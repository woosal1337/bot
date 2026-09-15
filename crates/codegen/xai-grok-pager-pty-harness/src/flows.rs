// Modified by the Bot project on 2026-09-12: remove privacy-specific auth fixtures.
//! Cross-suite e2e flow helpers over [`PtyHarness`] / [`ContentController`].
//!
//! Driving/seeding helpers shared by the pager's `pty_e2e` and `leader_pty_e2e` test targets (both depend on this crate).
//! Suite-local constants (sizes, sentinels, timeouts) stay in each suite's `common.rs`.

use std::time::{Duration, Instant};

use crate::{ContentController, PtyHarness};

/// Pump PTY output until every label is absent from the visible screen.
pub fn wait_for_labels_absent(h: &mut PtyHarness, labels: &[&str], timeout: Duration) {
    let _ = h.wait_until("screen labels to disappear", timeout, |h| {
        labels.iter().all(|label| !h.contains_text(label))
    });
}

/// Re-press Enter until `sentinel`: a leader attach race can drop the first submit and leave the prompt in the composer.
/// Extra Enter is a no-op once the draft is taken, so it cannot double-submit.
pub fn submit_turn(h: &mut PtyHarness, prompt: &str, sentinel: &str, timeout: Duration) {
    h.inject_keys(format!("{prompt}\r").as_bytes())
        .expect("inject prompt submit");
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        // Each attempt gets its own budget, generous enough that a genuinely in-progress submit resolves before we press Enter again
        // The extra Enter then only ever fires on an empty composer, where it is a no-op
        if h.wait_for_text(sentinel, Duration::from_secs(10).min(remaining))
            .is_ok()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {timeout:?} waiting for {sentinel:?}\nscreen:\n{}",
            h.screen_contents()
        );
        let _ = h.inject_keys(b"\r");
    }
}

/// Count only inference requests (chat completions / responses / messages), ignoring incidental GETs like /v1/models and /v1/settings.
/// A replay invariant then means "no turn was re-driven" rather than "no HTTP at all".
pub fn inference_request_count(content: &ContentController) -> usize {
    content
        .requests()
        .iter()
        .filter(|e| {
            e.path.contains("/chat/completions")
                || e.path.contains("/responses")
                || e.path.contains("/messages")
        })
        .count()
}

/// `XAI_API_KEY` never enters the auth manager. Scope is `<issuer>::<client_id>`, oidc, far-future expiry so no refresh.
/// Opt-out must be false or collection e2es never enqueue; a missing field deserializes as opted-out.
pub fn seed_fake_oauth(content: &ContentController, user: &str) {
    let grok_home = content.home().join(".grok");
    std::fs::create_dir_all(&grok_home).expect("create temp .grok");
    std::fs::write(
        grok_home.join("auth.json"),
        format!(
            r#"{{
  "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {{
    "key": "pty-test-oauth-token",
    "auth_mode": "oidc",
    "create_time": "2026-01-01T00:00:00Z",
    "user_id": "{user}",
    "email": "{user}@test.invalid",
    "expires_at": "2030-01-01T00:00:00Z",
    "refresh_token": "pty-test-refresh-token",
    "oidc_issuer": "https://auth.x.ai",
    "oidc_client_id": "b1a00492-073a-47ea-816f-4c329264a828",
    "coding_data_retention_opt_out": false
  }}
}}"#
        ),
    )
    .expect("seed fake oauth auth.json");
}

/// Remove only the sandbox's fake API-key credential.
/// The `auth.json` entry written by [`seed_fake_oauth`] then determines the advertised auth method.
pub fn oauth_credential_ops() -> [crate::EnvOp<'static>; 1] {
    [crate::EnvOp::remove("XAI_API_KEY")]
}

/// Campaigns apply to new sessions only, and settings prefetch is 2s-capped, so the first session may lack the model.
pub fn wait_for_model_via_new_sessions(h: &mut PtyHarness, model: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if h.contains_text(model) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        let _ = h.inject_keys(b"/new\r");
        h.update(Duration::from_millis(3000));
    }
}
