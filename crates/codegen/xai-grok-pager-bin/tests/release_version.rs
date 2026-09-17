use std::process::Command;

#[test]
fn cli_and_tui_release_versions_match() {
    assert_eq!(bot_core::VERSION, env!("CARGO_PKG_VERSION"));

    let output = Command::new(env!("CARGO_BIN_EXE_bot"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = format!("bot {} (", bot_core::VERSION);
    assert!(stdout.starts_with(&expected), "{stdout:?}");
}
