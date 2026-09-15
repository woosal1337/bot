// Modified by the Bot project on 2026-09-13: Removed obsolete updater launch flags.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use xai_tty_utils::ProcessGroup;
use xai_tty_utils::ProcessScope;

fn spawn(
    args: &[&str],
) -> (
    tempfile::TempDir,
    ProcessScope,
    Arc<ProcessGroup>,
    tokio::process::Child,
) {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths = vec![fixtures];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut command = Command::new(env!("CARGO_BIN_EXE_bot"));
    command
        .args(["--provider", "codex"])
        .args(args)
        .current_dir(directory.path())
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("GROK_HOME", directory.path().join("grok"))
        .env("CODEX_HOME", directory.path().join("codex"))
        .env_remove("XAI_API_KEY")
        .env_remove("GROK_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let scope = ProcessScope::new();
    let (child, group) = scope.spawn(command).unwrap();
    (directory, scope, group, child)
}

async fn run(args: &[&str]) -> std::process::Output {
    let (_directory, _scope, _group, child) = spawn(args);
    tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("headless command timed out")
        .unwrap()
}

#[tokio::test]
async fn codex_headless_interrupts_a_running_turn() {
    let (directory, scope, _group, mut child) = spawn(&["--single", "WAIT"]);
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(20), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.trim(), "BOT_FIXTURE_OK");
    let mut signal = Command::new("/bin/kill");
    signal.args(["-INT", "--", &format!("-{}", child.id().unwrap())]);
    let (mut signal, _signal_group) = scope.spawn(signal).unwrap();
    assert!(signal.wait().await.unwrap().success());
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("cancel timed out")
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Turn cancelled"));
    assert!(directory.path().join("cancelled").exists());
}

#[tokio::test]
async fn codex_headless_keeps_events_sent_before_the_turn_response() {
    let output = run(&["--single", "OK"]).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "BOT_FIXTURE_OK"
    );
}

#[tokio::test]
async fn codex_headless_uses_native_permission_and_effort_keywords() {
    let (directory, _scope, _group, child) = spawn(&[
        "--single",
        "OK",
        "--permission-mode",
        "read-only",
        "--model",
        "test-model",
        "--effort",
        "ultra",
    ]);
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("headless command timed out")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let turn: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("last-turn.json")).unwrap())
            .unwrap();
    assert_eq!(turn["model"], "test-model");
    assert_eq!(turn["effort"], "ultra");
    assert_eq!(turn["approvalPolicy"], "on-request");
    assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
}

#[tokio::test]
async fn codex_headless_full_access_disables_approval_and_sandbox() {
    let (directory, _scope, _group, child) =
        spawn(&["--single", "OK", "--permission-mode", "full-access"]);
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("headless command timed out")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let turn: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("last-turn.json")).unwrap())
            .unwrap();
    assert_eq!(turn["approvalPolicy"], "never");
    assert_eq!(turn["sandboxPolicy"]["type"], "dangerFullAccess");
}

#[tokio::test]
async fn codex_headless_emits_json_and_resumes_provider_threads() {
    let output = run(&[
        "--single",
        "OK",
        "--output-format",
        "json",
        "--resume",
        "bot-fixture-thread",
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.to_string().contains("BOT_FIXTURE_OK"));
    assert!(value.to_string().contains("bot-fixture-thread"));
}

#[tokio::test]
async fn codex_headless_returns_nonzero_for_provider_errors() {
    let output = run(&["--single", "FAIL"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Fixture turn failed"));
}

#[tokio::test]
async fn codex_headless_rejects_unconnected_options() {
    for args in [
        vec!["--single", "OK", "--max-turns", "1"],
        vec!["--single", "OK", "--json-schema", r#"{"type":"object"}"#],
    ] {
        let output = run(&args).await;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not connected"));
    }
}

#[tokio::test]
async fn codex_headless_rejects_grok_permission_keywords() {
    let output = run(&["--single", "OK", "--permission-mode", "acceptEdits"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not available for Codex"));
}

#[tokio::test]
async fn codex_selection_cannot_run_a_grok_login_command() {
    let output = run(&["login"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("use the native codex CLI"));
}
