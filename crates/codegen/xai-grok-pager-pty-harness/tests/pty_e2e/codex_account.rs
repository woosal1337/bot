// Added by the Bot project on 2026-09-14: Codex account terminal coverage.

#[allow(unused_imports)]
use super::common::*;

fn spawn_codex_fixture(signed_out: bool) -> (xai_grok_test_support::TestSandbox, PtyHarness) {
    spawn_codex_fixture_with_browser(signed_out, true)
}

fn spawn_codex_fixture_with_browser(
    signed_out: bool,
    browser_available: bool,
) -> (xai_grok_test_support::TestSandbox, PtyHarness) {
    let binary = pager_binary().expect("resolve pager binary");
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .to_path_buf();
    let fixtures = workspace.join("crates/codegen/xai-grok-pager-bin/tests/fixtures");
    let mut paths = vec![fixtures];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).expect("Codex fixture PATH");
    let path = path.to_str().expect("UTF-8 Codex fixture PATH");
    let sandbox = xai_grok_test_support::TestSandbox::new();
    let code_home = sandbox.home().join(".codex");
    std::fs::create_dir_all(&code_home).expect("create CODEX_HOME");
    let code_home = code_home.to_str().expect("UTF-8 CODEX_HOME");
    let state_dir = sandbox.workspace().to_str().expect("UTF-8 state path");
    let open_url = sandbox.root().join("opened-url");
    let open_url = open_url.to_str().expect("UTF-8 URL path");
    let open_url_env = if browser_available {
        EnvOp::set("GROK_TEST_OPEN_URL_FILE", open_url)
    } else {
        EnvOp::remove("GROK_TEST_OPEN_URL_FILE")
    };
    let signed_out = if signed_out { "1" } else { "" };
    let harness = PtyHarness::new_in_sandbox_ops(
        &binary,
        DEFAULT_ROWS,
        DEFAULT_COLS,
        &["--provider", "codex"],
        &sandbox,
        &[
            EnvOp::set("PATH", path),
            EnvOp::set("CODEX_HOME", code_home),
            EnvOp::set("BOT_FIXTURE_STATE_DIR", state_dir),
            EnvOp::set("BOT_FIXTURE_SIGNED_OUT", signed_out),
            EnvOp::set("BOT_FIXTURE_LOGIN_DELAY_MS", "1000"),
            open_url_env,
            EnvOp::remove("XAI_API_KEY"),
            EnvOp::remove("GROK_API_KEY"),
        ],
        Some(sandbox.workspace()),
    )
    .expect("spawn Bot with Codex fixture");
    (sandbox, harness)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn codex_headless_login_defaults_to_a_device_code() {
    let (sandbox, mut harness) = spawn_codex_fixture_with_browser(true, false);

    harness
        .wait_for_text("Waiting for approval", Duration::from_secs(10))
        .expect("automatic device login screen");
    harness
        .wait_for_text("ABCD-EFGH", Duration::from_secs(10))
        .expect("device login code");
    harness
        .wait_for_text("New worktree", Duration::from_secs(20))
        .expect("device login completed");
    let login: serde_json::Value = serde_json::from_slice(
        &std::fs::read(sandbox.workspace().join("last-account-login.json"))
            .expect("read login request"),
    )
    .expect("parse login request");
    assert_eq!(login["type"], "chatgptDeviceCode");
    harness.quit().expect("clean quit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn codex_account_shows_status_and_rate_limits() {
    let (_sandbox, mut harness) = spawn_codex_fixture(false);

    harness
        .wait_for_text(WELCOME_SCREEN_SENTINEL, WELCOME_TIMEOUT)
        .expect("Codex welcome screen");
    harness.inject_keys(b"/account\r").expect("submit /account");
    harness
        .wait_for_text("Codex account", Duration::from_secs(20))
        .expect("account document");
    harness
        .wait_for_text("63% remaining", Duration::from_secs(10))
        .expect("primary rate limit");
    assert!(
        harness.contains_text("bot@example.com"),
        "account email missing\nscreen:\n{}",
        harness.screen_contents()
    );
    assert!(
        harness.contains_text("12.50"),
        "credit balance missing\nscreen:\n{}",
        harness.screen_contents()
    );
    harness.quit().expect("clean quit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn codex_browser_device_and_logout_work_in_the_tui() {
    let (sandbox, mut harness) = spawn_codex_fixture(true);
    let login_text = "Login with Sign in with ChatGPT";

    harness
        .wait_for_text("Waiting for login to complete", Duration::from_secs(10))
        .expect("automatic browser login screen");
    harness
        .wait_for_text("New worktree", Duration::from_secs(20))
        .expect("browser login completed");
    let browser: serde_json::Value = serde_json::from_slice(
        &std::fs::read(sandbox.workspace().join("last-account-login.json"))
            .expect("read browser login request"),
    )
    .expect("parse browser login request");
    assert_eq!(browser["type"], "chatgpt");

    harness
        .inject_keys(b"/login device\r")
        .expect("start device login");
    harness
        .wait_for_text("Waiting for approval", Duration::from_secs(10))
        .expect("device login screen");
    harness
        .wait_for_text("ABCD-EFGH", Duration::from_secs(10))
        .expect("device login code");
    harness
        .wait_for_text("Shift+Tab:mode", Duration::from_secs(20))
        .expect("device login completed");
    let device: serde_json::Value = serde_json::from_slice(
        &std::fs::read(sandbox.workspace().join("last-account-login.json"))
            .expect("read device login request"),
    )
    .expect("parse device login request");
    assert_eq!(device["type"], "chatgptDeviceCode");

    harness.inject_keys(b"/logout\r").expect("submit /logout");
    harness
        .wait_for_text(login_text, Duration::from_secs(20))
        .expect("logout returned to the signed-out screen");
    assert!(sandbox.workspace().join("account-logout").exists());
    harness.quit().expect("clean quit");
}
